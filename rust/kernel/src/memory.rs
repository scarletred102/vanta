//! Physical-memory discovery and buddy frame allocation with reference counting.
//!
//! Replaces the early 256 MiB linear-scan allocator with an O(1) Binary Buddy
//! Allocator supporting multi-gigabyte address spaces, orders 0..=10 (4 KiB to 4 MiB),
//! per-frame reference counts for Copy-On-Write (COW), and instant O(1) freeing.

use limine::{memmap, request::MemmapResponse};
use spin::Mutex;

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_ORDER: usize = 11; // Orders 0 through 10 (4 KiB to 4 MiB)

/// Maximum physical frames supported (16,777,216 * 4096 = 64 GiB physical address space).
pub const MAX_SUPPORTED_FRAMES: usize = 16_777_216;
pub const MAX_TRACKED_FRAMES: usize = MAX_SUPPORTED_FRAMES;
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
    next_free: &'static mut [u32],
    /// Doubly linked list prev pointers for each frame.
    prev_free: &'static mut [u32],
    /// Per-frame order and status flags.
    flags: &'static mut [u8],
    /// Per-frame reference count (0 = free, 1 = exclusive, >1 = shared COW).
    refcounts: &'static mut [u16],
    total_usable_frames: usize,
    free_frames: usize,
    max_pfn: usize,
}

impl BuddyAllocator {
    const fn empty() -> Self {
        Self {
            free_lists: [None; MAX_ORDER],
            next_free: &mut [],
            prev_free: &mut [],
            flags: &mut [],
            refcounts: &mut [],
            total_usable_frames: 0,
            free_frames: 0,
            max_pfn: 0,
        }
    }

    fn list_push(&mut self, order: usize, pfn: usize) {
        if pfn >= self.max_pfn {
            return;
        }
        let head = self.free_lists[order];
        self.next_free[pfn] = head.map_or(NONE, |h| h as u32);
        self.prev_free[pfn] = NONE;
        if let Some(h) = head {
            if h < self.max_pfn {
                self.prev_free[h] = pfn as u32;
            }
        }
        self.free_lists[order] = Some(pfn);
        self.flags[pfn] = (order as u8 & ORDER_MASK) | FLAG_FREE;
    }

    fn list_remove(&mut self, order: usize, pfn: usize) {
        if pfn >= self.max_pfn {
            return;
        }
        let prev = self.prev_free[pfn];
        let next = self.next_free[pfn];

        if prev != NONE && (prev as usize) < self.max_pfn {
            self.next_free[prev as usize] = next;
        } else if self.free_lists[order] == Some(pfn) {
            self.free_lists[order] = if next != NONE && (next as usize) < self.max_pfn { Some(next as usize) } else { None };
        }

        if next != NONE && (next as usize) < self.max_pfn {
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
        let end_pfn = ((last / PAGE_SIZE) as usize).min(self.max_pfn);
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
            if buddy_pfn >= self.max_pfn {
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
        if pfn >= self.max_pfn {
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
        if pfn >= self.max_pfn {
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
        if pfn < self.max_pfn {
            self.refcounts[pfn]
        } else {
            0
        }
    }

    fn ref_inc(&mut self, pfn: usize) {
        if pfn < self.max_pfn {
            self.refcounts[pfn] = self.refcounts[pfn].saturating_add(1);
        }
    }

    fn ref_dec(&mut self, pfn: usize) -> u16 {
        if pfn < self.max_pfn {
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

pub fn init(response: &MemmapResponse, hhdm_offset: u64) -> MemoryStats {
    let mut max_usable_phys = 0u64;
    let mut total_usable_bytes = 0u64;
    for entry in response.entries() {
        if entry.type_ == memmap::MEMMAP_USABLE {
            max_usable_phys = max_usable_phys.max(entry.base.saturating_add(entry.length));
            total_usable_bytes = total_usable_bytes.saturating_add(entry.length);
        }
    }

    let raw_max_pfn = ((max_usable_phys.saturating_add(PAGE_SIZE - 1)) / PAGE_SIZE) as usize;
    let max_pfn = raw_max_pfn.min(MAX_SUPPORTED_FRAMES);

    let meta_bytes = max_pfn * (core::mem::size_of::<u32>() * 2 + core::mem::size_of::<u8>() + core::mem::size_of::<u16>());
    let meta_bytes_aligned = (meta_bytes + (PAGE_SIZE as usize) - 1) & !((PAGE_SIZE as usize) - 1);
    let meta_frames = meta_bytes_aligned / (PAGE_SIZE as usize);

    // Pick the largest usable memory block to house the metadata arrays
    let mut chosen_entry: Option<&memmap::Entry> = None;
    for entry in response.entries() {
        if entry.type_ == memmap::MEMMAP_USABLE && entry.length >= (meta_bytes_aligned as u64) {
            if chosen_entry.is_none() || entry.length > chosen_entry.unwrap().length {
                chosen_entry = Some(entry);
            }
        }
    }
    let chosen = chosen_entry.expect("No usable memory region large enough for frame tracking metadata");
    let meta_phys_base = align_up(chosen.base);

    let meta_virt = hhdm_offset.checked_add(meta_phys_base).expect("Invalid HHDM offset");
    let next_free_ptr = meta_virt as *mut u32;
    let prev_free_ptr = unsafe { next_free_ptr.add(max_pfn) };
    let flags_ptr = unsafe { prev_free_ptr.add(max_pfn) as *mut u8 };
    let refcounts_ptr = unsafe { flags_ptr.add(max_pfn) as *mut u16 };

    let next_free: &'static mut [u32] = unsafe { core::slice::from_raw_parts_mut(next_free_ptr, max_pfn) };
    let prev_free: &'static mut [u32] = unsafe { core::slice::from_raw_parts_mut(prev_free_ptr, max_pfn) };
    let flags: &'static mut [u8] = unsafe { core::slice::from_raw_parts_mut(flags_ptr, max_pfn) };
    let refcounts: &'static mut [u16] = unsafe { core::slice::from_raw_parts_mut(refcounts_ptr, max_pfn) };

    next_free.fill(NONE);
    prev_free.fill(NONE);
    flags.fill(0);
    refcounts.fill(0);

    let mut allocator = FRAME_ALLOCATOR.lock();
    *allocator = BuddyAllocator {
        free_lists: [None; MAX_ORDER],
        next_free,
        prev_free,
        flags,
        refcounts,
        total_usable_frames: 0,
        free_frames: 0,
        max_pfn,
    };

    for entry in response.entries() {
        if entry.type_ != memmap::MEMMAP_USABLE {
            continue;
        }

        if entry.base == chosen.base && entry.length == chosen.length {
            // Carve out the metadata region from this block
            if meta_phys_base > chosen.base {
                allocator.add_range(chosen.base, meta_phys_base - chosen.base);
            }
            let meta_end = meta_phys_base + (meta_bytes_aligned as u64);
            let block_end = chosen.base + chosen.length;
            if block_end > meta_end {
                allocator.add_range(meta_end, block_end - meta_end);
            }
            // Mark the metadata frames as reserved
            let meta_start_pfn = (meta_phys_base / PAGE_SIZE) as usize;
            for f in 0..meta_frames {
                let pfn = meta_start_pfn + f;
                allocator.flags[pfn] = FLAG_RESERVED | FLAG_ALLOCATED;
                allocator.refcounts[pfn] = 1;
            }
            allocator.total_usable_frames += meta_frames;
        } else {
            allocator.add_range(entry.base, entry.length);
        }
    }

    let stats = MemoryStats {
        usable_bytes: total_usable_bytes,
        usable_frames: allocator.free_frames,
        tracked_frames: allocator.total_usable_frames,
        map_entries: response.entries().len(),
    };
    *MEMORY_STATS.lock() = stats;
    stats
}

pub fn alloc_frame() -> Option<PhysFrame> {
    let wm = crate::swap::low_watermark();
    if wm > 0 && free_frames_count() <= wm {
        let _ = crate::swap::evict_page_clock();
    }

    let mut res = FRAME_ALLOCATOR.lock().alloc();
    if res.is_none() && wm > 0 {
        let _ = crate::swap::evict_page_clock();
        res = FRAME_ALLOCATOR.lock().alloc();
    }
    res
}

pub fn alloc_frames(order: usize) -> Option<PhysFrame> {
    let wm = crate::swap::low_watermark();
    if wm > 0 && free_frames_count() <= wm {
        let _ = crate::swap::evict_page_clock();
    }
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

/// Query recorded memory statistics.
pub fn stats() -> MemoryStats {
    *MEMORY_STATS.lock()
}
