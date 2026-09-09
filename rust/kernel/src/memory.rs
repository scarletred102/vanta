//! Physical-memory discovery and buddy frame allocation with reference counting.
//!
//! Replaces the early 256 MiB linear-scan allocator with an O(1) Binary Buddy
//! Allocator supporting multi-gigabyte address spaces, orders 0..=10 (4 KiB to 4 MiB),
//! per-frame reference counts for Copy-On-Write (COW), and instant O(1) freeing.

use limine::{memmap, request::MemmapResponse};
use spin::Mutex;

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_ORDER: usize = 11; // Orders 0 through 10 (4 KiB to 4 MiB)

/// Maximum physical frames tracked (524,288 * 4096 = 2 GiB physical address space).
pub const MAX_TRACKED_FRAMES: usize = 524_288;
const NONE: u32 = u32::MAX;

const FLAG_ALLOCATED: u8 = 1 << 4;
const FLAG_FREE: u8 = 1 << 5;
const FLAG_RESERVED: u8 = 1 << 6;
const ORDER_MASK: u8 = 0x0F;

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

struct BuddyAllocator {
    /// Heads of doubly-linked free lists for each order (0..=10).
    free_lists: [Option<usize>; MAX_ORDER],
    /// Doubly linked list next pointers for each frame.
    next_free: [u32; MAX_TRACKED_FRAMES],
    /// Doubly linked list prev pointers for each frame.
    prev_free: [u32; MAX_TRACKED_FRAMES],
    /// Per-frame order and status flags.
    flags: [u8; MAX_TRACKED_FRAMES],
    /// Per-frame reference count (0 = free, 1 = exclusive, >1 = shared COW).
    refcounts: [u16; MAX_TRACKED_FRAMES],
    total_usable_frames: usize,
    free_frames: usize,
}

impl BuddyAllocator {
    const fn empty() -> Self {
        Self {
            free_lists: [None; MAX_ORDER],
            next_free: [NONE; MAX_TRACKED_FRAMES],
            prev_free: [NONE; MAX_TRACKED_FRAMES],
            flags: [0; MAX_TRACKED_FRAMES],
            refcounts: [0; MAX_TRACKED_FRAMES],
            total_usable_frames: 0,
            free_frames: 0,
        }
    }

    fn list_push(&mut self, order: usize, pfn: usize) {
        let head = self.free_lists[order];
        self.next_free[pfn] = head.map_or(NONE, |h| h as u32);
        self.prev_free[pfn] = NONE;
        if let Some(h) = head {
            self.prev_free[h] = pfn as u32;
        }
        self.free_lists[order] = Some(pfn);
        self.flags[pfn] = (order as u8 & ORDER_MASK) | FLAG_FREE;
    }

    fn list_remove(&mut self, order: usize, pfn: usize) {
        let prev = self.prev_free[pfn];
        let next = self.next_free[pfn];

        if prev != NONE {
            self.next_free[prev as usize] = next;
        } else if self.free_lists[order] == Some(pfn) {
            self.free_lists[order] = if next != NONE { Some(next as usize) } else { None };
        }

        if next != NONE {
            self.prev_free[next as usize] = prev;
        }

        self.prev_free[pfn] = NONE;
        self.next_free[pfn] = NONE;
        self.flags[pfn] &= !FLAG_FREE;
    }

    fn list_pop(&mut self, order: usize) -> Option<usize> {
        let head = self.free_lists[order]?;
        self.list_remove(order, head);
        Some(head)
    }

    fn add_range(&mut self, base: u64, length: u64) -> usize {
        let Some(end) = base.checked_add(length) else {
            return 0;
        };

        // Keep the null page permanently unavailable.
        let first = align_up(base).max(PAGE_SIZE);
        let last = end & !(PAGE_SIZE - 1);
        if first >= last {
            return 0;
        }

        let mut start_pfn = (first / PAGE_SIZE) as usize;
        let end_pfn = ((last / PAGE_SIZE) as usize).min(MAX_TRACKED_FRAMES);
        if start_pfn >= end_pfn {
            return 0;
        }

        let frames_added = end_pfn - start_pfn;
        self.total_usable_frames += frames_added;
        self.free_frames += frames_added;

        while start_pfn < end_pfn {
            let mut order = MAX_ORDER - 1;
            while order > 0 {
                let block_size = 1usize << order;
                if start_pfn + block_size <= end_pfn && (start_pfn & (block_size - 1)) == 0 {
                    break;
                }
                order -= 1;
            }

            self.list_push(order, start_pfn);
            start_pfn += 1usize << order;
        }

        frames_added
    }

    fn alloc_order(&mut self, order: usize) -> Option<usize> {
        if order >= MAX_ORDER {
            return None;
        }

        let mut current_order = order;
        while current_order < MAX_ORDER && self.free_lists[current_order].is_none() {
            current_order += 1;
        }

        if current_order >= MAX_ORDER {
            return None;
        }

        let pfn = self.list_pop(current_order)?;

        while current_order > order {
            current_order -= 1;
            let buddy_pfn = pfn + (1usize << current_order);
            self.list_push(current_order, buddy_pfn);
        }

        self.flags[pfn] = (order as u8 & ORDER_MASK) | FLAG_ALLOCATED;
        self.refcounts[pfn] = 1;
        self.free_frames = self.free_frames.saturating_sub(1usize << order);
        Some(pfn)
    }

    fn free_order(&mut self, mut pfn: usize, mut order: usize) {
        let block_size = 1usize << order;
        self.free_frames += block_size;
        self.flags[pfn] &= !FLAG_ALLOCATED;

        while order < MAX_ORDER - 1 {
            let buddy_pfn = pfn ^ (1usize << order);
            if buddy_pfn >= MAX_TRACKED_FRAMES {
                break;
            }

            let buddy_flags = self.flags[buddy_pfn];
            let buddy_order = (buddy_flags & ORDER_MASK) as usize;
            let buddy_is_free = (buddy_flags & FLAG_FREE) != 0;

            if buddy_is_free && buddy_order == order {
                self.list_remove(order, buddy_pfn);
                pfn = pfn.min(buddy_pfn);
                order += 1;
            } else {
                break;
            }
        }

        self.list_push(order, pfn);
    }

    fn alloc(&mut self) -> Option<PhysFrame> {
        self.alloc_order(0).map(|pfn| PhysFrame((pfn as u64) * PAGE_SIZE))
    }

    fn free(&mut self, frame: PhysFrame) -> bool {
        let addr = frame.start_address();
        if addr == 0 || addr & (PAGE_SIZE - 1) != 0 {
            return false;
        }

        let pfn = (addr / PAGE_SIZE) as usize;
        if pfn >= MAX_TRACKED_FRAMES {
            return false;
        }

        let current_ref = self.refcounts[pfn];
        if current_ref > 1 {
            self.refcounts[pfn] = current_ref - 1;
            return true;
        }

        if current_ref == 1 {
            self.refcounts[pfn] = 0;
            let order = (self.flags[pfn] & ORDER_MASK) as usize;
            self.free_order(pfn, order);
            return true;
        }

        false
    }

    fn reserve(&mut self, frame: PhysFrame) -> bool {
        let addr = frame.start_address();
        if addr == 0 || addr & (PAGE_SIZE - 1) != 0 {
            return false;
        }

        let pfn = (addr / PAGE_SIZE) as usize;
        if pfn >= MAX_TRACKED_FRAMES {
            return false;
        }

        if (self.flags[pfn] & FLAG_ALLOCATED) != 0 || (self.flags[pfn] & FLAG_RESERVED) != 0 {
            return true;
        }

        // Search which free block contains this PFN
        for order in 0..MAX_ORDER {
            let block_size = 1usize << order;
            let block_head = pfn & !(block_size - 1);
            let flags = self.flags[block_head];
            if (flags & FLAG_FREE) != 0 && (flags & ORDER_MASK) as usize == order {
                self.list_remove(order, block_head);

                let mut cur_head = block_head;
                let mut cur_order = order;
                while cur_order > 0 {
                    cur_order -= 1;
                    let half = 1usize << cur_order;
                    let lower = cur_head;
                    let upper = cur_head + half;
                    if pfn < upper {
                        self.list_push(cur_order, upper);
                        cur_head = lower;
                    } else {
                        self.list_push(cur_order, lower);
                        cur_head = upper;
                    }
                }

                self.flags[pfn] = FLAG_RESERVED | FLAG_ALLOCATED;
                self.refcounts[pfn] = 1;
                self.free_frames = self.free_frames.saturating_sub(1);
                return true;
            }
        }

        self.flags[pfn] |= FLAG_RESERVED;
        true
    }

    fn refcount(&self, pfn: usize) -> u16 {
        if pfn < MAX_TRACKED_FRAMES {
            self.refcounts[pfn]
        } else {
            0
        }
    }

    fn ref_inc(&mut self, pfn: usize) {
        if pfn < MAX_TRACKED_FRAMES {
            self.refcounts[pfn] = self.refcounts[pfn].saturating_add(1);
        }
    }

    fn ref_dec(&mut self, pfn: usize) -> u16 {
        if pfn < MAX_TRACKED_FRAMES {
            let new_ref = self.refcounts[pfn].saturating_sub(1);
            self.refcounts[pfn] = new_ref;
            new_ref
        } else {
            0
        }
    }
}

static FRAME_ALLOCATOR: Mutex<BuddyAllocator> = Mutex::new(BuddyAllocator::empty());
static MEMORY_STATS: Mutex<MemoryStats> = Mutex::new(MemoryStats::empty());

const fn align_up(address: u64) -> u64 {
    address.saturating_add(PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

pub fn init(response: &MemmapResponse) -> MemoryStats {
    let mut allocator = FRAME_ALLOCATOR.lock();
    *allocator = BuddyAllocator::empty();

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

    stats.tracked_frames = allocator.total_usable_frames;
    *MEMORY_STATS.lock() = stats;
    stats
}

pub fn alloc_frame() -> Option<PhysFrame> {
    FRAME_ALLOCATOR.lock().alloc()
}

pub fn alloc_frames(order: usize) -> Option<PhysFrame> {
    FRAME_ALLOCATOR
        .lock()
        .alloc_order(order)
        .map(|pfn| PhysFrame((pfn as u64) * PAGE_SIZE))
}

pub fn free_frame(frame: PhysFrame) -> bool {
    FRAME_ALLOCATOR.lock().free(frame)
}

/// Mark a bootloader-owned frame unavailable to Vanta allocations.
pub fn reserve_frame(frame: PhysFrame) -> bool {
    FRAME_ALLOCATOR.lock().reserve(frame)
}

/// Return current reference count of a physical frame.
pub fn frame_refcount(frame: PhysFrame) -> u16 {
    let pfn = (frame.start_address() / PAGE_SIZE) as usize;
    FRAME_ALLOCATOR.lock().refcount(pfn)
}

/// Increment reference count of a physical frame (for Copy-on-Write sharing).
pub fn frame_ref_inc(frame: PhysFrame) {
    let pfn = (frame.start_address() / PAGE_SIZE) as usize;
    FRAME_ALLOCATOR.lock().ref_inc(pfn);
}

/// Decrement reference count of a physical frame. Returns the new refcount.
pub fn frame_ref_dec(frame: PhysFrame) -> u16 {
    let pfn = (frame.start_address() / PAGE_SIZE) as usize;
    FRAME_ALLOCATOR.lock().ref_dec(pfn)
}

/// Query current free physical frames.
pub fn free_frames_count() -> usize {
    FRAME_ALLOCATOR.lock().free_frames
}
