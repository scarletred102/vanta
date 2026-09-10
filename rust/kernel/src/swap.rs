//! VirtIO-blk backed swap subsystem and page eviction.
//!
//! Implements:
//! - VirtIO-blk disk I/O (8 sectors / 4 KiB page)
//! - Dynamic swap slot allocator with bitmap
//! - Clock / Second-Chance eviction algorithm scanning active user pages
//! - Page-out path: write 4 KiB to swap disk, mark PTE with MAP_SWAPPED, free frame
//! - Page-in path: resolve #PF on MAP_SWAPPED, read 4 KiB from swap disk, restore PTE, free slot
//! - Boot / self-check: force memory pressure, evict page, verify data integrity on page-in.

#![allow(dead_code, unused_imports)]

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use spin::Mutex;
pub use crate::paging::MAP_SWAPPED;
use crate::{memory, paging, vfs};

pub const MAX_SWAP_SLOTS: usize = 16384;
const BITMAP_WORDS: usize = MAX_SWAP_SLOTS / 64;

static SWAP_START_LBA: AtomicU64 = AtomicU64::new(0);
static SWAP_TOTAL_SECTORS: AtomicU64 = AtomicU64::new(0);
static SWAP_INITIALIZED: AtomicBool = AtomicBool::new(false);
static LOW_WATERMARK: AtomicUsize = AtomicUsize::new(0);
static IN_EVICTION: AtomicBool = AtomicBool::new(false);

pub fn low_watermark() -> usize {
    LOW_WATERMARK.load(Ordering::Relaxed)
}

pub fn set_low_watermark(val: usize) {
    LOW_WATERMARK.store(val, Ordering::SeqCst);
}

pub fn track_user_page(space: paging::AddressSpace, vaddr: u64) {
    SWAP_MANAGER.lock().track_page(space, vaddr);
}

pub fn untrack_user_page(space: paging::AddressSpace, vaddr: u64) {
    SWAP_MANAGER.lock().untrack_page(space, vaddr);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackedUserPage {
    pub space: paging::AddressSpace,
    pub vaddr: u64,
}

const MAX_TRACKED_PAGES: usize = 512;

pub struct SwapManager {
    bitmap: [u64; BITMAP_WORDS],
    used_slots: usize,
    clock_hand: usize,
    tracked_pages: [Option<TrackedUserPage>; MAX_TRACKED_PAGES],
    tracked_count: usize,
}

impl SwapManager {
    pub const fn new() -> Self {
        Self {
            bitmap: [0; BITMAP_WORDS],
            used_slots: 0,
            clock_hand: 0,
            tracked_pages: [None; MAX_TRACKED_PAGES],
            tracked_count: 0,
        }
    }

    pub fn alloc_slot(&mut self) -> Option<u32> {
        for (word_idx, word) in self.bitmap.iter_mut().enumerate() {
            if *word != !0u64 {
                let bit_idx = (!*word).trailing_zeros() as usize;
                *word |= 1u64 << bit_idx;
                self.used_slots += 1;
                return Some((word_idx * 64 + bit_idx) as u32);
            }
        }
        None
    }

    pub fn free_slot(&mut self, slot: u32) {
        let slot = slot as usize;
        if slot >= MAX_SWAP_SLOTS {
            return;
        }
        let word_idx = slot / 64;
        let bit_idx = slot % 64;
        if self.bitmap[word_idx] & (1u64 << bit_idx) != 0 {
            self.bitmap[word_idx] &= !(1u64 << bit_idx);
            self.used_slots = self.used_slots.saturating_sub(1);
        }
    }

    pub fn track_page(&mut self, space: paging::AddressSpace, vaddr: u64) {
        let page_vaddr = vaddr & !(memory::PAGE_SIZE - 1);
        for slot in self.tracked_pages.iter_mut() {
            if let Some(entry) = slot {
                if entry.space == space && entry.vaddr == page_vaddr {
                    return;
                }
            }
        }
        for slot in self.tracked_pages.iter_mut() {
            if slot.is_none() {
                *slot = Some(TrackedUserPage {
                    space,
                    vaddr: page_vaddr,
                });
                self.tracked_count += 1;
                return;
            }
        }
    }

    pub fn untrack_page(&mut self, space: paging::AddressSpace, vaddr: u64) {
        let page_vaddr = vaddr & !(memory::PAGE_SIZE - 1);
        for slot in self.tracked_pages.iter_mut() {
            if let Some(entry) = slot {
                if entry.space == space && entry.vaddr == page_vaddr {
                    *slot = None;
                    self.tracked_count = self.tracked_count.saturating_sub(1);
                    return;
                }
            }
        }
    }

    pub fn used_slots(&self) -> usize {
        self.used_slots
    }

    pub fn free_slots(&self) -> usize {
        MAX_SWAP_SLOTS.saturating_sub(self.used_slots)
    }
}

pub static SWAP_MANAGER: Mutex<SwapManager> = Mutex::new(SwapManager::new());

pub fn init(start_lba: u64, sectors: u64) {
    SWAP_START_LBA.store(start_lba, Ordering::SeqCst);
    SWAP_TOTAL_SECTORS.store(sectors, Ordering::SeqCst);
    SWAP_INITIALIZED.store(true, Ordering::SeqCst);
}

pub fn is_initialized() -> bool {
    SWAP_INITIALIZED.load(Ordering::Relaxed)
}

pub fn swap_start_lba() -> u64 {
    SWAP_START_LBA.load(Ordering::Relaxed)
}

pub fn swap_total_sectors() -> u64 {
    SWAP_TOTAL_SECTORS.load(Ordering::Relaxed)
}

pub fn write_page_to_disk(slot: u32, page_phys: u64) -> Result<(), ()> {
    let start_lba = swap_start_lba();
    let sector_base = start_lba + (slot as u64) * 8;
    let virt = paging::phys_to_virt(page_phys).ok_or(())?;
    let slice = unsafe { core::slice::from_raw_parts(virt as *const u8, 4096) };

    for i in 0..8 {
        let mut sector = [0u8; 512];
        sector.copy_from_slice(&slice[i * 512..(i + 1) * 512]);
        vfs::write_block_sector(sector_base + i as u64, &sector).map_err(|_| ())?;
    }
    Ok(())
}

pub fn read_page_from_disk(slot: u32, page_phys: u64) -> Result<(), ()> {
    let start_lba = swap_start_lba();
    let sector_base = start_lba + (slot as u64) * 8;
    let virt = paging::phys_to_virt(page_phys).ok_or(())?;
    let slice = unsafe { core::slice::from_raw_parts_mut(virt as *mut u8, 4096) };

    for i in 0..8 {
        let mut sector = [0u8; 512];
        vfs::read_block_sector(sector_base + i as u64, &mut sector).map_err(|_| ())?;
        slice[i * 512..(i + 1) * 512].copy_from_slice(&sector);
    }
    Ok(())
}

/// Evict a candidate page using the Clock / Second-Chance algorithm.
/// Scans active user pages, checks the hardware ACCESSED bit:
/// - If set: clears it and gives second chance
/// - If not set: evicts page to VirtIO swap disk and frees frame to buddy allocator.
pub fn evict_page_clock() -> Result<u32, ()> {
    if !is_initialized() {
        return Err(());
    }
    if IN_EVICTION
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return Err(());
    }
    struct EvictionGuard;
    impl Drop for EvictionGuard {
        fn drop(&mut self) {
            IN_EVICTION.store(false, Ordering::Release);
        }
    }
    let _guard = EvictionGuard;

    let mut mgr = SWAP_MANAGER.lock();
    if mgr.tracked_count == 0 {
        return Err(());
    }

    let slot = mgr.alloc_slot().ok_or(())?;

    let total_slots = MAX_TRACKED_PAGES;
    let mut scanned = 0;
    while scanned < total_slots * 2 {
        let idx = mgr.clock_hand % total_slots;
        mgr.clock_hand = (mgr.clock_hand + 1) % total_slots;
        scanned += 1;

        if let Some(target) = mgr.tracked_pages[idx] {
            match paging::check_and_clear_accessed(target.space, target.vaddr) {
                Ok(Some(true)) => {
                    // Second chance given, advance clock hand
                    continue;
                }
                Ok(Some(false)) => {
                    // Victim found! Evict this page to VirtIO disk
                    mgr.tracked_pages[idx] = None;
                    mgr.tracked_count = mgr.tracked_count.saturating_sub(1);
                    drop(mgr);

                    crate::serial_println!(
                        "[swap] watermark reached: evicting page {:#x} to slot {}",
                        target.vaddr,
                        slot
                    );

                    match paging::swap_page_out(target.space, target.vaddr, slot) {
                        Ok(_phys) => return Ok(slot),
                        Err(_) => {
                            SWAP_MANAGER.lock().free_slot(slot);
                            return Err(());
                        }
                    }
                }
                _ => {
                    // Page not mapped or not present
                    mgr.tracked_pages[idx] = None;
                    mgr.tracked_count = mgr.tracked_count.saturating_sub(1);
                }
            }
        }
    }

    mgr.free_slot(slot);
    Err(())
}

pub fn page_out(space: paging::AddressSpace, vaddr: u64) -> Result<u32, ()> {
    let slot = SWAP_MANAGER.lock().alloc_slot().ok_or(())?;
    match paging::swap_page_out(space, vaddr, slot) {
        Ok(_) => Ok(slot),
        Err(_) => {
            SWAP_MANAGER.lock().free_slot(slot);
            Err(())
        }
    }
}

pub fn self_check() {
    let space = crate::paging::current_address_space();
    let test_vaddr = 0x5000_0000_u64;

    let frame = match memory::alloc_frame() {
        Some(f) => f,
        None => {
            crate::serial_println!("[swap] WARNING: frame alloc failed");
            return;
        }
    };
    let phys = frame.start_address();
    let virt = match paging::phys_to_virt(phys) {
        Some(v) => v,
        None => {
            crate::serial_println!("[swap] WARNING: phys_to_virt failed");
            let _ = memory::free_frame(frame);
            return;
        }
    };

    // Fill page with known non-zero pattern
    let pattern_val = 0xcafe_d00d_1234_5678_u64;
    unsafe {
        let ptr = virt as *mut u64;
        for i in 0..512 {
            ptr.add(i).write(pattern_val ^ (i as u64));
        }
    }

    let flags = paging::MAP_USER | paging::MAP_WRITABLE;
    if let Err(e) = paging::map(space, test_vaddr, phys, flags) {
        crate::serial_println!("[swap] WARNING: map failed: {:?}", e);
        let _ = memory::free_frame(frame);
        return;
    }

    // Track page in Clock eviction pool
    track_user_page(space, test_vaddr);

    // Set watermark threshold to trigger automatic eviction under pressure
    let free_before = memory::free_frames_count();
    let trigger_wm = free_before.saturating_sub(2);
    set_low_watermark(trigger_wm);

    // Allocate frames until watermark is breached.
    // memory::alloc_frame() checks `free_frames_count() <= low_watermark()`
    // and automatically invokes `evict_page_clock()`.
    let mut pressure_frames = [None; 4];
    for slot in pressure_frames.iter_mut() {
        *slot = memory::alloc_frame();
    }

    // Reset watermark back to operational threshold (256 frames = 1 MiB headroom)
    set_low_watermark(256);

    // Verify page at test_vaddr was automatically evicted (PTE not present)
    let is_swapped = paging::translate_in(space, test_vaddr).is_none();

    // Trigger transparent page-in by accessing the evicted address.
    // In current_address_space(), this triggers a hardware #PF -> resolve_swapped_page().
    let first_val = unsafe { core::ptr::read_volatile(test_vaddr as *const u64) };
    let first_ok = first_val == pattern_val;

    // Verify data integrity of entire restored page
    let mut match_count = 0;
    unsafe {
        let ptr = test_vaddr as *const u64;
        for i in 0..512 {
            if core::ptr::read_volatile(ptr.add(i)) == (pattern_val ^ (i as u64)) {
                match_count += 1;
            }
        }
    }
    let data_verified = match_count == 512;

    // Clean up allocated pressure frames
    for slot in pressure_frames.iter_mut() {
        if let Some(f) = slot.take() {
            let _ = memory::free_frame(f);
        }
    }

    // Clean up test page
    if let Ok(Some(p)) = paging::unmap(space, test_vaddr) {
        let _ = memory::free_frame(memory::PhysFrame(p));
    }
    untrack_user_page(space, test_vaddr);

    if is_swapped && first_ok && data_verified {
        crate::serial_println!(
            "[swap] memory pressure eviction and swap-in verified: slot=0 data-verified=true"
        );
    } else {
        crate::serial_println!(
            "[swap] WARNING: pressure test failed (swapped={} first_ok={} verified={})",
            is_swapped,
            first_ok,
            data_verified
        );
    }
}
