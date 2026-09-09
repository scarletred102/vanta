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
// Support loading multiple native and Linux static ELFs into memory.
const HEAP_PAGES: usize = 8192;
const HEAP_SIZE: usize = HEAP_PAGES * memory::PAGE_SIZE as usize;
const MAX_FREE_BLOCKS: usize = 16384;

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
    mapped_pages: usize,
    initialized: bool,
}

impl HeapState {
    const fn empty() -> Self {
        Self {
            blocks: [FreeBlock::empty(); MAX_FREE_BLOCKS],
            len: 0,
            mapped_pages: 0,
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

    fn expand_heap(&mut self, needed_bytes: usize) -> bool {
        let chunk_size = needed_bytes.max(2 * 1024 * 1024);
        let pages = (chunk_size + memory::PAGE_SIZE as usize - 1) / memory::PAGE_SIZE as usize;
        let space = paging::current_address_space();
        let start_vaddr = HEAP_BASE + self.mapped_pages as u64 * memory::PAGE_SIZE;

        for i in 0..pages {
            let Some(frame) = memory::alloc_frame() else {
                return false;
            };
            let vaddr = start_vaddr + i as u64 * memory::PAGE_SIZE;
            if paging::map(space, vaddr, frame.start_address(), paging::MAP_WRITABLE).is_err() {
                let _ = memory::free_frame(frame);
                return false;
            }
        }

        let added_size = pages * memory::PAGE_SIZE as usize;
        let start_addr = start_vaddr as usize;
        self.mapped_pages += pages;
        self.release(start_addr, added_size)
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
        let total_size = if state.initialized {
            state.mapped_pages * memory::PAGE_SIZE as usize
        } else {
            0
        };
        HeapStats {
            base: HEAP_BASE,
            size: total_size,
            used: total_size.saturating_sub(free),
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
    state.mapped_pages = HEAP_PAGES;
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

        // If no free block fits the allocation, dynamically expand the heap
        if state.expand_heap(layout.size() + alignment) {
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
        let max_mapped = HEAP_BASE as usize + state.mapped_pages * memory::PAGE_SIZE as usize;
        if end > max_mapped {
            return;
        }
        let _ = state.release(start, layout.size());
    }
}

pub struct KMemCache {
    name: &'static str,
    object_size: usize,
    alignment: usize,
    freelist: Mutex<Option<core::ptr::NonNull<u8>>>,
}

impl KMemCache {
    pub const fn new(name: &'static str, object_size: usize, alignment: usize) -> Self {
        Self {
            name,
            object_size,
            alignment,
            freelist: Mutex::new(None),
        }
    }

    pub fn alloc(&self) -> Option<*mut u8> {
        let mut list = self.freelist.lock();
        if let Some(node) = *list {
            unsafe {
                let next = (node.as_ptr() as *mut *mut u8).read();
                *list = core::ptr::NonNull::new(next);
                return Some(node.as_ptr());
            }
        }
        let frame = memory::alloc_frame()?;
        let phys = frame.start_address();
        let vaddr = paging::phys_to_virt(phys)? as *mut u8;
        let obj_size = self.object_size.max(core::mem::size_of::<*mut u8>());
        let count = (memory::PAGE_SIZE as usize) / obj_size;

        unsafe {
            for i in 1..count {
                let curr = vaddr.add(i * obj_size);
                let next = if i + 1 < count {
                    vaddr.add((i + 1) * obj_size)
                } else {
                    core::ptr::null_mut()
                };
                (curr as *mut *mut u8).write(next);
            }
            if count > 1 {
                *list = core::ptr::NonNull::new(vaddr.add(obj_size));
            }
            Some(vaddr)
        }
    }

    pub fn free(&self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }
        let mut list = self.freelist.lock();
        unsafe {
            let next = list.map(|n| n.as_ptr()).unwrap_or(core::ptr::null_mut());
            (ptr as *mut *mut u8).write(next);
            *list = core::ptr::NonNull::new(ptr);
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}
