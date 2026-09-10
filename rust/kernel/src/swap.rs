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

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use spin::Mutex;
pub use crate::paging::MAP_SWAPPED;
use crate::{memory, paging, vfs};

pub const MAX_SWAP_SLOTS: usize = 16384;
const BITMAP_WORDS: usize = MAX_SWAP_SLOTS / 64;

static SWAP_START_LBA: AtomicU64 = AtomicU64::new(0);
static SWAP_TOTAL_SECTORS: AtomicU64 = AtomicU64::new(0);
static SWAP_INITIALIZED: AtomicBool = AtomicBool::new(false);

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
    let space = match paging::create_address_space() {
        Ok(s) => s,
        Err(e) => {
            crate::serial_println!("[swap] WARNING: create_address_space failed: {:?}", e);
            return;
        }
    };
    let test_vaddr = 0x5000_0000_u64;

    let frame = match memory::alloc_frame() {
        Some(f) => f,
        None => {
            crate::serial_println!("[swap] WARNING: frame alloc failed");
            let _ = paging::destroy_address_space(space);
            return;
        }
    };
    let phys = frame.start_address();
    let virt = match paging::phys_to_virt(phys) {
        Some(v) => v,
        None => {
            crate::serial_println!("[swap] WARNING: phys_to_virt failed");
            let _ = memory::free_frame(frame);
            let _ = paging::destroy_address_space(space);
            return;
        }
    };

    // Fill page with known non-zero pattern
    let pattern_val = 0xdead_beef_c001_cafe_u64;
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
        let _ = paging::destroy_address_space(space);
        return;
    }

    // Track page in Clock eviction pool
    SWAP_MANAGER.lock().track_page(space, test_vaddr);

    let free_before = memory::free_frames_count();

    // Force memory pressure eviction via Clock algorithm
    let slot = match evict_page_clock() {
        Ok(s) => s,
        Err(_) => {
            crate::serial_println!("[swap] WARNING: clock eviction failed");
            let _ = paging::unmap(space, test_vaddr);
            let _ = memory::free_frame(frame);
            let _ = paging::destroy_address_space(space);
            return;
        }
    };

    let free_after = memory::free_frames_count();
    let frame_reclaimed = free_after > free_before;

    // Verify PTE is not present
    let is_swapped = paging::translate_in(space, test_vaddr).is_none();

    // Resolve swapped page (simulate page fault dispatch)
    let page_in_ok = paging::resolve_swapped_page(space, test_vaddr).unwrap_or(false);

    // Verify data integrity of restored page from VirtIO disk
    let translation = paging::translate_in(space, test_vaddr);
    let mut data_verified = false;
    if let Some(trans) = translation {
        if let Some(restored_virt) = paging::phys_to_virt(trans.physical_address) {
            let mut match_count = 0;
            unsafe {
                let ptr = restored_virt as *const u64;
                for i in 0..512 {
                    if ptr.add(i).read() == pattern_val ^ (i as u64) {
                        match_count += 1;
                    }
                }
            }
            data_verified = match_count == 512;
        }
    }

    // Clean up
    if let Ok(Some(p)) = paging::unmap(space, test_vaddr) {
        let _ = memory::free_frame(memory::PhysFrame(p));
    }
    let _ = paging::destroy_address_space(space);

    if frame_reclaimed && is_swapped && page_in_ok && data_verified {
        crate::serial_println!(
            "[swap] memory pressure eviction and swap-in self-check passed: slot={} data-verified=true",
            slot
        );
    } else {
        crate::serial_println!(
            "[swap] WARNING: self-check failed (reclaimed={} swapped={} page_in={} verified={})",
            frame_reclaimed,
            is_swapped,
            page_in_ok,
            data_verified
        );
    }
}
