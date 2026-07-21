//! Early x86_64 virtual-memory support.
//!
//! Limine has already installed usable page tables before entering Vanta. This
//! milestone keeps Limine's kernel mappings shared while providing HHDM
//! translation, page-table walks, mutable map/unmap, and address-space cleanup.

use core::arch::asm;

use spin::Mutex;

use crate::memory::{self, PAGE_SIZE};

const PRESENT: u64 = 1 << 0;
const HUGE_PAGE: u64 = 1 << 7;
const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;
const ADDRESS_MASK_2M: u64 = 0x000f_ffff_ffe0_0000;
const ADDRESS_MASK_1G: u64 = 0x000f_ffc0_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Translation {
    pub physical_address: u64,
    pub page_size: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageTableSummary {
    pub hhdm_offset: u64,
    pub cr3: u64,
    pub present_pml4_entries: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AddressSpace {
    pub pml4_phys: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MapError {
    NoHhdm,
    OutOfMemory,
    UnalignedAddress,
    AlreadyMapped,
    HugePageConflict,
    ActiveAddressSpace,
    MappingsRemain,
    FrameReleaseFailed,
}

pub const MAP_WRITABLE: u64 = 1 << 1;
pub const MAP_USER: u64 = 1 << 2;
pub const MAP_CACHE_DISABLE: u64 = 1 << 4;
pub const MAP_NO_EXECUTE: u64 = 1 << 63;

static HHDM_OFFSET: Mutex<Option<u64>> = Mutex::new(None);

pub fn init(hhdm_offset: u64) {
    *HHDM_OFFSET.lock() = Some(hhdm_offset);
}

pub fn hhdm_offset() -> Option<u64> {
    *HHDM_OFFSET.lock()
}

pub fn phys_to_virt(physical_address: u64) -> Option<u64> {
    hhdm_offset()?.checked_add(physical_address)
}

pub fn virt_to_phys(virtual_address: u64) -> Option<u64> {
    virtual_address.checked_sub(hhdm_offset()?)
}

pub fn current_cr3() -> u64 {
    let value: u64;
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nostack, preserves_flags));
    }
    value & ADDRESS_MASK
}

pub fn current_address_space() -> AddressSpace {
    AddressSpace {
        pml4_phys: current_cr3(),
    }
}

/// Activate an address space that shares the current kernel-half mappings.
///
/// The caller must ensure that the target contains valid mappings for the
/// currently executing kernel and stack before switching CR3.
pub unsafe fn activate(space: AddressSpace) {
    unsafe {
        asm!(
            "mov cr3, {pml4}",
            pml4 = in(reg) space.pml4_phys,
            options(nostack, preserves_flags)
        );
    }
}

pub fn inspect_current() -> PageTableSummary {
    let hhdm_offset = hhdm_offset().unwrap_or(0);
    let cr3 = current_cr3();
    let mut present_pml4_entries = 0;

    for index in 0..512 {
        if read_entry(cr3, index).is_some_and(|entry| entry & PRESENT != 0) {
            present_pml4_entries += 1;
        }
    }

    PageTableSummary {
        hhdm_offset,
        cr3,
        present_pml4_entries,
    }
}

/// Reserve the frames holding Limine's currently active page tables.
///
/// Limine may describe those frames as usable memory. They must be excluded
/// before Vanta uses physical frames for page tables, heap pages, or DMA.
pub fn reserve_active_page_tables() -> usize {
    reserve_table_frames(current_cr3(), 4)
}

pub fn translate(virtual_address: u64) -> Option<Translation> {
    translate_in(current_address_space(), virtual_address)
}

pub fn translate_in(space: AddressSpace, virtual_address: u64) -> Option<Translation> {
    let pml4 = space.pml4_phys;
    let pml4_entry = read_entry(pml4, index_pml4(virtual_address))?;
    if pml4_entry & PRESENT == 0 {
        return None;
    }

    let pdpt = pml4_entry & ADDRESS_MASK;
    let pdpt_entry = read_entry(pdpt, index_pdpt(virtual_address))?;
    if pdpt_entry & PRESENT == 0 {
        return None;
    }
    if pdpt_entry & HUGE_PAGE != 0 {
        return Some(Translation {
            physical_address: (pdpt_entry & ADDRESS_MASK_1G) | (virtual_address & ((1 << 30) - 1)),
            page_size: 1 << 30,
        });
    }

    let pd = pdpt_entry & ADDRESS_MASK;
    let pd_entry = read_entry(pd, index_pd(virtual_address))?;
    if pd_entry & PRESENT == 0 {
        return None;
    }
    if pd_entry & HUGE_PAGE != 0 {
        return Some(Translation {
            physical_address: (pd_entry & ADDRESS_MASK_2M) | (virtual_address & ((1 << 21) - 1)),
            page_size: 1 << 21,
        });
    }

    let pt = pd_entry & ADDRESS_MASK;
    let pt_entry = read_entry(pt, index_pt(virtual_address))?;
    if pt_entry & PRESENT == 0 {
        return None;
    }

    Some(Translation {
        physical_address: (pt_entry & ADDRESS_MASK) | (virtual_address & (PAGE_SIZE - 1)),
        page_size: PAGE_SIZE,
    })
}

/// Return the raw flags from a mapped 4 KiB leaf PTE.
pub fn flags_in(space: AddressSpace, virtual_address: u64) -> Option<u64> {
    let location = pte_location(space, virtual_address, false, false).ok()??;
    let entry = read_entry(location.table_phys, location.index)?;
    (entry & PRESENT != 0).then_some(entry & !ADDRESS_MASK)
}

/// Create an address space that shares the active kernel-half mappings.
///
/// The returned PML4 is not activated. User-space entries start empty, while
/// entries 256..511 retain the kernel mappings required to enter the kernel.
pub fn create_address_space() -> Result<AddressSpace, MapError> {
    let pml4 = allocate_table()?;
    let current = current_cr3();

    for index in 256..512 {
        let entry = read_entry(current, index).ok_or(MapError::NoHhdm)?;
        if !write_entry(pml4, index, entry) {
            return Err(MapError::NoHhdm);
        }
    }

    Ok(AddressSpace { pml4_phys: pml4 })
}

/// Map one 4 KiB page into an address space.
pub fn map(
    space: AddressSpace,
    virtual_address: u64,
    physical_address: u64,
    flags: u64,
) -> Result<(), MapError> {
    if virtual_address & (PAGE_SIZE - 1) != 0 || physical_address & (PAGE_SIZE - 1) != 0 {
        return Err(MapError::UnalignedAddress);
    }

    let location = pte_location(space, virtual_address, true, flags & MAP_USER != 0)?
        .ok_or(MapError::NoHhdm)?;
    let current = read_entry(location.table_phys, location.index).ok_or(MapError::NoHhdm)?;
    if current & PRESENT != 0 {
        return Err(MapError::AlreadyMapped);
    }

    if !write_entry(
        location.table_phys,
        location.index,
        (physical_address & ADDRESS_MASK) | flags | PRESENT,
    ) {
        return Err(MapError::NoHhdm);
    }
    flush_if_active(space, virtual_address);
    Ok(())
}

/// Remove one 4 KiB mapping and return its physical page, if present.
pub fn unmap(space: AddressSpace, virtual_address: u64) -> Result<Option<u64>, MapError> {
    if virtual_address & (PAGE_SIZE - 1) != 0 {
        return Err(MapError::UnalignedAddress);
    }

    let Some(location) = pte_location(space, virtual_address, false, false)? else {
        return Ok(None);
    };
    let current = read_entry(location.table_phys, location.index).ok_or(MapError::NoHhdm)?;
    if current & PRESENT == 0 {
        return Ok(None);
    }

    if !write_entry(location.table_phys, location.index, 0) {
        return Err(MapError::NoHhdm);
    }
    flush_if_active(space, virtual_address);
    Ok(Some(current & ADDRESS_MASK))
}

/// Destroy a non-active address space after all of its leaf mappings are gone.
/// Kernel-half mappings are shared and remain owned by the active address
/// space; only user-half page-table frames allocated for this space are freed.
pub fn destroy_address_space(space: AddressSpace) -> Result<usize, MapError> {
    if space.pml4_phys == current_cr3() {
        return Err(MapError::ActiveAddressSpace);
    }

    for index in 0..256 {
        let entry = read_entry(space.pml4_phys, index).ok_or(MapError::NoHhdm)?;
        if entry & PRESENT == 0 {
            continue;
        }
        if entry & HUGE_PAGE != 0 || contains_leaf_mapping(entry & ADDRESS_MASK, 3)? {
            return Err(MapError::MappingsRemain);
        }
    }

    let mut freed = 0;
    for index in 0..256 {
        let entry = read_entry(space.pml4_phys, index).ok_or(MapError::NoHhdm)?;
        if entry & PRESENT == 0 {
            continue;
        }
        if !write_entry(space.pml4_phys, index, 0) {
            return Err(MapError::NoHhdm);
        }
        freed += destroy_table(entry & ADDRESS_MASK, 3)?;
        if !memory::free_frame(memory::PhysFrame(entry & ADDRESS_MASK)) {
            return Err(MapError::FrameReleaseFailed);
        }
        freed += 1;
    }

    if !memory::free_frame(memory::PhysFrame(space.pml4_phys)) {
        return Err(MapError::FrameReleaseFailed);
    }
    Ok(freed + 1)
}

fn contains_leaf_mapping(table_phys: u64, level: u8) -> Result<bool, MapError> {
    for index in 0..512 {
        let entry = read_entry(table_phys, index).ok_or(MapError::NoHhdm)?;
        if entry & PRESENT == 0 {
            continue;
        }
        if level == 1 || entry & HUGE_PAGE != 0 {
            return Ok(true);
        }
        if contains_leaf_mapping(entry & ADDRESS_MASK, level - 1)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn destroy_table(table_phys: u64, level: u8) -> Result<usize, MapError> {
    let mut freed = 0;
    for index in 0..512 {
        let entry = read_entry(table_phys, index).ok_or(MapError::NoHhdm)?;
        if entry & PRESENT == 0 {
            continue;
        }
        if level == 1 || entry & HUGE_PAGE != 0 {
            return Err(MapError::MappingsRemain);
        }
        if !write_entry(table_phys, index, 0) {
            return Err(MapError::NoHhdm);
        }
        freed += destroy_table(entry & ADDRESS_MASK, level - 1)?;
        if !memory::free_frame(memory::PhysFrame(entry & ADDRESS_MASK)) {
            return Err(MapError::FrameReleaseFailed);
        }
        freed += 1;
    }
    Ok(freed)
}

fn flush_if_active(space: AddressSpace, virtual_address: u64) {
    if space.pml4_phys == current_cr3() {
        x86_64::instructions::tlb::flush(x86_64::VirtAddr::new(virtual_address));
    }
}

#[derive(Clone, Copy)]
struct PteLocation {
    table_phys: u64,
    index: usize,
}

fn pte_location(
    space: AddressSpace,
    virtual_address: u64,
    create: bool,
    user: bool,
) -> Result<Option<PteLocation>, MapError> {
    let mut table = space.pml4_phys;
    let levels = [
        index_pml4(virtual_address),
        index_pdpt(virtual_address),
        index_pd(virtual_address),
    ];

    for index in levels {
        let entry = read_entry(table, index).ok_or(MapError::NoHhdm)?;
        if entry & PRESENT == 0 {
            if !create {
                return Ok(None);
            }

            let new_table = allocate_table()?;
            let mut new_flags = PRESENT | MAP_WRITABLE;
            if user {
                new_flags |= MAP_USER;
            }
            if !write_entry(table, index, new_table | new_flags) {
                return Err(MapError::NoHhdm);
            }
            table = new_table;
        } else {
            if entry & HUGE_PAGE != 0 {
                return Err(MapError::HugePageConflict);
            }
            table = entry & ADDRESS_MASK;
        }
    }

    Ok(Some(PteLocation {
        table_phys: table,
        index: index_pt(virtual_address),
    }))
}

fn allocate_table() -> Result<u64, MapError> {
    let frame = memory::alloc_frame().ok_or(MapError::OutOfMemory)?;
    let virtual_address = phys_to_virt(frame.start_address()).ok_or(MapError::NoHhdm)?;
    unsafe {
        core::ptr::write_bytes(virtual_address as *mut u8, 0, PAGE_SIZE as usize);
    }
    Ok(frame.start_address())
}

fn read_entry(table_physical_address: u64, index: usize) -> Option<u64> {
    let table_virtual_address = phys_to_virt(table_physical_address)?;
    let entry_address = table_virtual_address.checked_add((index * 8) as u64)?;
    Some(unsafe { (entry_address as *const u64).read_volatile() })
}

fn reserve_table_frames(table_physical_address: u64, levels: usize) -> usize {
    let mut reserved = usize::from(memory::reserve_frame(memory::PhysFrame(
        table_physical_address,
    )));
    if levels <= 1 {
        return reserved;
    }

    for index in 0..512 {
        let Some(entry) = read_entry(table_physical_address, index) else {
            continue;
        };
        if entry & PRESENT == 0 || entry & HUGE_PAGE != 0 {
            continue;
        }
        reserved += reserve_table_frames(entry & ADDRESS_MASK, levels - 1);
    }
    reserved
}

fn write_entry(table_physical_address: u64, index: usize, value: u64) -> bool {
    let Some(table_virtual_address) = phys_to_virt(table_physical_address) else {
        return false;
    };
    let Some(entry_address) = table_virtual_address.checked_add((index * 8) as u64) else {
        return false;
    };
    unsafe {
        (entry_address as *mut u64).write_volatile(value);
    }
    true
}

const fn index_pml4(address: u64) -> usize {
    ((address >> 39) & 0x1ff) as usize
}

const fn index_pdpt(address: u64) -> usize {
    ((address >> 30) & 0x1ff) as usize
}

const fn index_pd(address: u64) -> usize {
    ((address >> 21) & 0x1ff) as usize
}

const fn index_pt(address: u64) -> usize {
    ((address >> 12) & 0x1ff) as usize
}
