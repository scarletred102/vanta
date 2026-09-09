#![allow(dead_code)]

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::Mutex;

use crate::memory::PAGE_SIZE;
use crate::paging::AddressSpace;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VmaFlags(pub u32);

impl VmaFlags {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXEC: Self = Self(1 << 2);
    pub const SHARED: Self = Self(1 << 3);
    pub const ANONYMOUS: Self = Self(1 << 4);
    pub const GROWSDOWN: Self = Self(1 << 5);
    pub const STACK: Self = Self(1 << 6);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn all() -> Self {
        Self(0x7F)
    }

    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn bits(&self) -> u32 {
        self.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}

impl core::ops::BitOr for VmaFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl core::ops::BitOrAssign for VmaFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl core::ops::BitAnd for VmaFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VmaBacking {
    Anonymous,
    FileBacked {
        inode: u64,
        offset: u64,
        file_size: u64,
    },
    DeviceMmio {
        phys_addr: u64,
    },
}

#[derive(Clone, Debug)]
pub struct Vma {
    pub start: u64,
    pub end: u64, // Exclusive
    pub flags: VmaFlags,
    pub backing: VmaBacking,
}

impl Vma {
    pub fn new(start: u64, end: u64, flags: VmaFlags, backing: VmaBacking) -> Self {
        Self {
            start,
            end,
            flags,
            backing,
        }
    }

    pub fn contains(&self, addr: u64) -> bool {
        addr >= self.start && addr < self.end
    }

    pub fn size(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

#[derive(Clone, Debug)]
pub struct ProcessMemoryMap {
    pub vmas: BTreeMap<u64, Vma>, // Keyed by start address
    pub heap_start: u64,
    pub heap_break: u64,
    pub mmap_hint: u64,
    pub stack_bottom: u64,
    pub stack_top: u64,
}

impl ProcessMemoryMap {
    pub fn new(stack_top: u64, stack_bottom: u64) -> Self {
        Self {
            vmas: BTreeMap::new(),
            heap_start: 0,
            heap_break: 0,
            mmap_hint: 0x0000_7000_0000_0000,
            stack_bottom,
            stack_top,
        }
    }

    pub fn find_vma(&self, addr: u64) -> Option<&Vma> {
        // Find the VMA with largest start <= addr
        if let Some((_, vma)) = self.vmas.range(..=addr).next_back() {
            if vma.contains(addr) {
                return Some(vma);
            }
        }
        None
    }

    pub fn find_vma_mut(&mut self, addr: u64) -> Option<&mut Vma> {
        if let Some((&start, _)) = self.vmas.range(..=addr).next_back() {
            if let Some(vma) = self.vmas.get_mut(&start) {
                if vma.contains(addr) {
                    return Some(vma);
                }
            }
        }
        None
    }

    pub fn insert_vma(
        &mut self,
        start: u64,
        end: u64,
        flags: VmaFlags,
        backing: VmaBacking,
    ) -> Result<(), ()> {
        if start >= end || (start & (PAGE_SIZE - 1) != 0) || (end & (PAGE_SIZE - 1) != 0) {
            return Err(());
        }

        // Verify no overlap with existing VMAs
        if let Some((_, prev_vma)) = self.vmas.range(..end).next_back() {
            if prev_vma.end > start {
                return Err(());
            }
        }

        self.vmas.insert(start, Vma::new(start, end, flags, backing));
        Ok(())
    }

    pub fn remove_vma_range(&mut self, start: u64, end: u64) -> Vec<Vma> {
        let mut affected = Vec::new();
        let mut to_reinsert = Vec::new();

        let keys: Vec<u64> = self
            .vmas
            .range(..end)
            .filter_map(|(&v_start, vma)| if vma.end > start { Some(v_start) } else { None })
            .collect();

        for k in keys {
            if let Some(vma) = self.vmas.remove(&k) {
                affected.push(vma.clone());

                // Left surviving portion
                if vma.start < start {
                    to_reinsert.push(Vma::new(vma.start, start, vma.flags, vma.backing));
                }
                // Right surviving portion
                if vma.end > end {
                    let new_backing = match vma.backing {
                        VmaBacking::Anonymous => VmaBacking::Anonymous,
                        VmaBacking::FileBacked { inode, offset, file_size } => {
                            let skipped = end - vma.start;
                            VmaBacking::FileBacked {
                                inode,
                                offset: offset + skipped,
                                file_size,
                            }
                        }
                        VmaBacking::DeviceMmio { phys_addr } => {
                            let skipped = end - vma.start;
                            VmaBacking::DeviceMmio {
                                phys_addr: phys_addr + skipped,
                            }
                        }
                    };
                    to_reinsert.push(Vma::new(end, vma.end, vma.flags, new_backing));
                }
            }
        }

        for vma in to_reinsert {
            self.vmas.insert(vma.start, vma);
        }

        affected
    }

    pub fn find_stack_vma(&self) -> Option<&Vma> {
        self.vmas.values().find(|vma| vma.flags.contains(VmaFlags::STACK))
    }

    pub fn can_grow_stack(&self, addr: u64) -> bool {
        if addr >= self.stack_top || addr < self.stack_bottom {
            return false;
        }
        if let Some(stack_vma) = self.find_stack_vma() {
            if addr < stack_vma.start && addr >= self.stack_bottom {
                // Ensure stack doesn't collide with lower VMAs
                if let Some((_, prev_vma)) = self.vmas.range(..stack_vma.start).next_back() {
                    if addr < prev_vma.end {
                        return false;
                    }
                }
                return true;
            }
        }
        false
    }

    pub fn grow_stack_to(&mut self, addr: u64) -> Result<u64, ()> {
        let new_start = addr & !(PAGE_SIZE - 1);
        if new_start < self.stack_bottom {
            return Err(());
        }

        let stack_key = self
            .vmas
            .iter()
            .find_map(|(&k, vma)| if vma.flags.contains(VmaFlags::STACK) { Some(k) } else { None })
            .ok_or(())?;

        let mut stack_vma = self.vmas.remove(&stack_key).ok_or(())?;
        if new_start >= stack_vma.start {
            self.vmas.insert(stack_key, stack_vma);
            return Ok(stack_key);
        }

        // Verify no overlap with lower VMAs
        if let Some((_, prev_vma)) = self.vmas.range(..stack_vma.start).next_back() {
            if new_start < prev_vma.end {
                self.vmas.insert(stack_key, stack_vma);
                return Err(());
            }
        }

        stack_vma.start = new_start;
        self.vmas.insert(new_start, stack_vma);
        Ok(new_start)
    }

    pub fn find_free_mmap_region(&mut self, length: u64) -> Result<u64, ()> {
        let aligned_length = (length.checked_add(PAGE_SIZE - 1).ok_or(())?) & !(PAGE_SIZE - 1);
        let mut candidate = self.mmap_hint;

        // Search upward from mmap_hint below stack_bottom
        while candidate.checked_add(aligned_length).ok_or(())? < self.stack_bottom {
            let end = candidate + aligned_length;
            let mut conflict = false;

            for vma in self.vmas.values() {
                if candidate < vma.end && end > vma.start {
                    candidate = (vma.end + (PAGE_SIZE - 1)) & !(PAGE_SIZE - 1);
                    conflict = true;
                    break;
                }
            }

            if !conflict {
                self.mmap_hint = candidate + aligned_length;
                return Ok(candidate);
            }
        }

        // Fallback: search from 0x0000_1000_0000 upwards
        candidate = 0x0000_1000_0000;
        while candidate.checked_add(aligned_length).ok_or(())? < self.stack_bottom {
            let end = candidate + aligned_length;
            let mut conflict = false;

            for vma in self.vmas.values() {
                if candidate < vma.end && end > vma.start {
                    candidate = (vma.end + (PAGE_SIZE - 1)) & !(PAGE_SIZE - 1);
                    conflict = true;
                    break;
                }
            }

            if !conflict {
                return Ok(candidate);
            }
        }

        Err(())
    }
}

static ADDRESS_SPACE_VMAS: Mutex<BTreeMap<u64, Arc<Mutex<ProcessMemoryMap>>>> =
    Mutex::new(BTreeMap::new());

pub fn register_address_space_vmas(
    space: AddressSpace,
    vmas: Arc<Mutex<ProcessMemoryMap>>,
) {
    ADDRESS_SPACE_VMAS.lock().insert(space.pml4_phys, vmas);
}

pub fn unregister_address_space_vmas(space: AddressSpace) {
    ADDRESS_SPACE_VMAS.lock().remove(&space.pml4_phys);
}

pub fn get_address_space_vmas(space: AddressSpace) -> Option<Arc<Mutex<ProcessMemoryMap>>> {
    ADDRESS_SPACE_VMAS.lock().get(&space.pml4_phys).cloned()
}

pub fn resolve_demand_page(space: AddressSpace, address: u64) -> Result<bool, ()> {
    if address >= 0x0000_8000_0000_0000 {
        return Ok(false);
    }
    let Some(mem_map_arc) = get_address_space_vmas(space) else {
        return Ok(false);
    };
    let mut mem_map = mem_map_arc.lock();

    // Check stack growth
    if mem_map.can_grow_stack(address) {
        if mem_map.grow_stack_to(address).is_ok() {
            let page_aligned = address & !(PAGE_SIZE - 1);
            let frame = crate::memory::alloc_frame().ok_or(())?;
            let phys = frame.start_address();
            if let Some(virt) = crate::paging::phys_to_virt(phys) {
                unsafe {
                    core::ptr::write_bytes(virt as *mut u8, 0, PAGE_SIZE as usize);
                }
            }
            let flags = crate::paging::MAP_USER | crate::paging::MAP_WRITABLE | crate::paging::MAP_NO_EXECUTE;
            if crate::paging::map(space, page_aligned, phys, flags).is_ok() {
                return Ok(true);
            } else {
                let _ = crate::memory::free_frame(frame);
                return Err(());
            }
        }
    }

    // Check VMA range
    if let Some(vma) = mem_map.find_vma(address) {
        if vma.backing == VmaBacking::Anonymous {
            let page_aligned = address & !(PAGE_SIZE - 1);
            let frame = crate::memory::alloc_frame().ok_or(())?;
            let phys = frame.start_address();
            if let Some(virt) = crate::paging::phys_to_virt(phys) {
                unsafe {
                    core::ptr::write_bytes(virt as *mut u8, 0, PAGE_SIZE as usize);
                }
            }
            let mut pte_flags = crate::paging::MAP_USER;
            if vma.flags.contains(VmaFlags::WRITE) {
                pte_flags |= crate::paging::MAP_WRITABLE;
            }
            if !vma.flags.contains(VmaFlags::EXEC) {
                pte_flags |= crate::paging::MAP_NO_EXECUTE;
            }
            if crate::paging::map(space, page_aligned, phys, pte_flags).is_ok() {
                return Ok(true);
            } else {
                let _ = crate::memory::free_frame(frame);
                return Err(());
            }
        }
    }

    Ok(false)
}
