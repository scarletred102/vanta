//! Early kernel heap backed by Rust page mappings.
//!
//! This is a small coalescing free-list allocator for the bootstrap phase. It
//! gives Rust-native kernel code real dynamic allocation and reclamation while
//! the later slab allocator is still being translated. Every heap page is
//! mapped through the Rust paging layer before the allocator is published.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;

use spin::Mutex;

use crate::{memory, paging};

const HEAP_BASE: u64 = 0xffff_ff00_0000_0000;
// RedoxFS keeps an LZ4 cache for one 128 KiB metadata record. Its worst-case
// compressed representation is 144,199 bytes, so the bootstrap heap must be
// larger than the old 128 KiB reservation before a GPT root can be mounted.
const HEAP_PAGES: usize = 256;
const HEAP_SIZE: usize = HEAP_PAGES * memory::PAGE_SIZE as usize;
const MAX_FREE_BLOCKS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeapStats {
    pub base: u64,
    pub size: usize,
    pub used: usize,
    pub free: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeapInitError {
    AlreadyInitialized,
    OutOfMemory,
    Mapping(paging::MapError),
}

#[derive(Clone, Copy)]
struct FreeBlock {
    start: usize,
    size: usize,
}

impl FreeBlock {
    const fn empty() -> Self {
        Self { start: 0, size: 0 }
    }
}

struct HeapState {
    blocks: [FreeBlock; MAX_FREE_BLOCKS],
    len: usize,
    initialized: bool,
}

impl HeapState {
    const fn empty() -> Self {
        Self {
            blocks: [FreeBlock::empty(); MAX_FREE_BLOCKS],
            len: 0,
            initialized: false,
        }
    }

    fn free_bytes(&self) -> usize {
        self.blocks[..self.len]
            .iter()
            .fold(0, |total, block| total.saturating_add(block.size))
    }

    fn remove_block(&mut self, index: usize) {
        self.len -= 1;
        if index != self.len {
            self.blocks[index] = self.blocks[self.len];
        }
    }

    fn release(&mut self, start: usize, size: usize) -> bool {
        if size == 0 || self.len == MAX_FREE_BLOCKS {
            return false;
        }
        self.blocks[self.len] = FreeBlock { start, size };
        self.len += 1;
        self.coalesce();
        true
    }

    fn coalesce(&mut self) {
        let mut left = 0;
        while left < self.len {
            let mut right = left + 1;
            while right < self.len {
                let left_end = self.blocks[left].start + self.blocks[left].size;
                let right_end = self.blocks[right].start + self.blocks[right].size;

                if left_end == self.blocks[right].start {
                    self.blocks[left].size += self.blocks[right].size;
                    self.remove_block(right);
                    continue;
                }
                if right_end == self.blocks[left].start {
                    self.blocks[left].start = self.blocks[right].start;
                    self.blocks[left].size += self.blocks[right].size;
                    self.remove_block(right);
                    continue;
                }
                right += 1;
            }
            left += 1;
        }
    }
}

pub struct KernelHeap {
    state: Mutex<HeapState>,
}

impl KernelHeap {
    const fn new() -> Self {
        Self {
            state: Mutex::new(HeapState::empty()),
        }
    }

    fn is_initialized(&self) -> bool {
        self.state.lock().initialized
    }

    fn stats(&self) -> HeapStats {
        let state = self.state.lock();
        let free = if state.initialized {
            state.free_bytes()
        } else {
            0
        };
        HeapStats {
            base: HEAP_BASE,
            size: HEAP_SIZE,
            used: HEAP_SIZE.saturating_sub(free),
            free,
        }
    }
}

#[global_allocator]
static GLOBAL_HEAP: KernelHeap = KernelHeap::new();

pub fn init() -> Result<HeapStats, HeapInitError> {
    if GLOBAL_HEAP.is_initialized() {
        return Err(HeapInitError::AlreadyInitialized);
    }

    let space = paging::current_address_space();
    for page in 0..HEAP_PAGES {
        let frame = memory::alloc_frame().ok_or(HeapInitError::OutOfMemory)?;
        let virtual_address = HEAP_BASE + page as u64 * memory::PAGE_SIZE;
        paging::map(
            space,
            virtual_address,
            frame.start_address(),
            paging::MAP_WRITABLE,
        )
        .map_err(HeapInitError::Mapping)?;
    }

    let mut state = GLOBAL_HEAP.state.lock();
    state.blocks[0] = FreeBlock {
        start: HEAP_BASE as usize,
        size: HEAP_SIZE,
    };
    state.len = 1;
    state.initialized = true;
    drop(state);
    Ok(GLOBAL_HEAP.stats())
}

pub fn stats() -> HeapStats {
    GLOBAL_HEAP.stats()
}

unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.size() == 0 {
            return layout.align() as *mut u8;
        }

        let mut state = self.state.lock();
        if !state.initialized {
            return null_mut();
        }

        let alignment = layout.align();
        for index in 0..state.len {
            let block = state.blocks[index];
            let aligned = match block.start.checked_add(alignment - 1) {
                Some(value) => value & !(alignment - 1),
                None => continue,
            };
            let end = match aligned.checked_add(layout.size()) {
                Some(value) => value,
                None => continue,
            };
            let block_end = match block.start.checked_add(block.size) {
                Some(value) => value,
                None => continue,
            };
            if end > block_end {
                continue;
            }

            let prefix = aligned - block.start;
            let suffix = block_end - end;
            if prefix != 0 && suffix != 0 {
                if state.len == MAX_FREE_BLOCKS {
                    return null_mut();
                }
                state.blocks[index].start = block.start;
                state.blocks[index].size = prefix;
                let free_index = state.len;
                state.blocks[free_index] = FreeBlock {
                    start: end,
                    size: suffix,
                };
                state.len += 1;
            } else if prefix != 0 {
                state.blocks[index].start = block.start;
                state.blocks[index].size = prefix;
            } else if suffix != 0 {
                state.blocks[index].start = end;
                state.blocks[index].size = suffix;
            } else {
                state.remove_block(index);
            }
            return aligned as *mut u8;
        }
        null_mut()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.size() == 0 || ptr.is_null() {
            return;
        }

        let start = ptr as usize;
        let mut state = self.state.lock();
        if !state.initialized || start < HEAP_BASE as usize {
            return;
        }
        let Some(end) = start.checked_add(layout.size()) else {
            return;
        };
        if end > HEAP_BASE as usize + HEAP_SIZE {
            return;
        }
        let _ = state.release(start, layout.size());
    }
}
