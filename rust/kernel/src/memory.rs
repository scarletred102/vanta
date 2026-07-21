//! Physical-memory discovery and frame allocation.
//!
//! This is the first Rust translation of Vanta's memory-management boundary.
//! It follows the Linux split between memory discovery/accounting and the
//! allocator while keeping the early kernel dependency-free: Limine provides
//! the map, and a bounded static frame table provides allocations until the
//! Rust heap and page-table code arrive.

use limine::{memmap, request::MemmapResponse};
use spin::Mutex;

pub const PAGE_SIZE: u64 = 4096;
const MAX_TRACKED_FRAMES: usize = 65_536;
const FRAME_BITMAP_WORDS: usize = (MAX_TRACKED_FRAMES + 63) / 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysFrame(pub u64);

impl PhysFrame {
    pub const fn start_address(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryStats {
    pub usable_bytes: u64,
    pub usable_frames: usize,
    pub tracked_frames: usize,
    pub map_entries: usize,
}

impl MemoryStats {
    const fn empty() -> Self {
        Self {
            usable_bytes: 0,
            usable_frames: 0,
            tracked_frames: 0,
            map_entries: 0,
        }
    }
}

struct FrameAllocator {
    frames: [u64; MAX_TRACKED_FRAMES],
    used: [u64; FRAME_BITMAP_WORDS],
    next: usize,
    len: usize,
}

impl FrameAllocator {
    const fn empty() -> Self {
        Self {
            frames: [0; MAX_TRACKED_FRAMES],
            used: [0; FRAME_BITMAP_WORDS],
            next: 0,
            len: 0,
        }
    }

    fn add_range(&mut self, base: u64, length: u64) -> usize {
        let Some(end) = base.checked_add(length) else {
            return 0;
        };

        // Keep the null page permanently unavailable. Firmware maps often
        // describe it as usable, but allocating it would undermine null
        // pointer faulting once paging and userspace are enabled.
        let first = align_up(base).max(PAGE_SIZE);
        let last = end & !(PAGE_SIZE - 1);
        if first >= last {
            return 0;
        }

        let frame_count = ((last - first) / PAGE_SIZE) as usize;
        let to_track = frame_count.min(MAX_TRACKED_FRAMES.saturating_sub(self.len));
        for index in 0..to_track {
            self.frames[self.len + index] = first + (index as u64 * PAGE_SIZE);
        }
        self.len += to_track;
        frame_count
    }

    fn alloc(&mut self) -> Option<PhysFrame> {
        for attempt in 0..self.len {
            let index = (self.next + attempt) % self.len;
            if self.is_used(index) {
                continue;
            }

            self.set_used(index, true);
            self.next = (index + 1) % self.len;
            return Some(PhysFrame(self.frames[index]));
        }
        None
    }

    fn free(&mut self, frame: PhysFrame) -> bool {
        if frame.start_address() == 0 || frame.start_address() & (PAGE_SIZE - 1) != 0 {
            return false;
        }

        for index in 0..self.len {
            if self.frames[index] != frame.start_address() {
                continue;
            }
            if !self.is_used(index) {
                return false;
            }
            self.set_used(index, false);
            self.next = index;
            return true;
        }
        false
    }

    fn reserve(&mut self, frame: PhysFrame) -> bool {
        for index in 0..self.len {
            if self.frames[index] != frame.start_address() {
                continue;
            }
            if self.is_used(index) {
                return false;
            }
            self.set_used(index, true);
            return true;
        }
        false
    }

    fn is_used(&self, index: usize) -> bool {
        self.used[index / 64] & (1 << (index % 64)) != 0
    }

    fn set_used(&mut self, index: usize, used: bool) {
        let word = &mut self.used[index / 64];
        let bit = 1 << (index % 64);
        if used {
            *word |= bit;
        } else {
            *word &= !bit;
        }
    }
}

static FRAME_ALLOCATOR: Mutex<FrameAllocator> = Mutex::new(FrameAllocator::empty());
static MEMORY_STATS: Mutex<MemoryStats> = Mutex::new(MemoryStats::empty());

const fn align_up(address: u64) -> u64 {
    address.saturating_add(PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

pub fn init(response: &MemmapResponse) -> MemoryStats {
    let mut allocator = FRAME_ALLOCATOR.lock();
    *allocator = FrameAllocator::empty();

    let mut stats = MemoryStats {
        map_entries: response.entries().len(),
        ..MemoryStats::empty()
    };

    for entry in response.entries() {
        if entry.type_ != memmap::MEMMAP_USABLE {
            continue;
        }

        stats.usable_bytes = stats.usable_bytes.saturating_add(entry.length);
        stats.usable_frames = stats
            .usable_frames
            .saturating_add(allocator.add_range(entry.base, entry.length));
    }

    stats.tracked_frames = allocator.len;
    *MEMORY_STATS.lock() = stats;
    stats
}

pub fn alloc_frame() -> Option<PhysFrame> {
    FRAME_ALLOCATOR.lock().alloc()
}

pub fn free_frame(frame: PhysFrame) -> bool {
    FRAME_ALLOCATOR.lock().free(frame)
}

/// Mark a bootloader-owned frame unavailable to Vanta allocations.
pub fn reserve_frame(frame: PhysFrame) -> bool {
    FRAME_ALLOCATOR.lock().reserve(frame)
}
