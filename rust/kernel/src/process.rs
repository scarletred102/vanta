//! First user-process lifecycle: ELF segments, user stack, and ring-3 entry.

use alloc::vec::Vec;
use core::ptr;

use crate::elf::{self, ElfError, ElfImage, ProgramHeader};
use crate::memory::{self, PhysFrame, PAGE_SIZE};
use crate::paging::{self, AddressSpace, MapError};

pub const INTERP_BASE: u64 = 0x0000_7f00_0000_0000;
pub const DEFAULT_PIE_BASE: u64 = 0x0040_0000;
const USER_STACK_TOP: u64 = 0x0000_7fff_ffff_f000;
const USER_STACK_PAGES: usize = 16;
const USER_STACK_START: u64 = USER_STACK_TOP - (USER_STACK_PAGES as u64 * PAGE_SIZE);
const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MappedPage {
    virtual_address: u64,
    physical_address: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PagePlan {
    virtual_address: u64,
    flags: u64,
    executable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessError {
    Elf(ElfError),
    LinuxElf(vanta_linuxd::ElfError),
    Map(MapError),
    OutOfMemory,
    InvalidLoadSegment,
    InvalidUserAddress,
    EntryNotExecutable,
    FrameReleaseFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessPersonality {
    NativeVanta,
    LinuxX86_64Static,
}

pub struct Process {
    space: AddressSpace,
    entry: u64,
    personality: ProcessPersonality,
    #[allow(dead_code)]
    fs_base: u64,
    user_stack_top: u64,
    brk_start: u64,
    brk_current: u64,
    mmap_next: u64,
    mappings: Vec<MappedPage>,
    destroyed: bool,
}

impl Process {
    pub fn entry(&self) -> u64 {
        self.entry
    }

    pub fn personality(&self) -> ProcessPersonality {
        self.personality
    }

    #[allow(dead_code)]
    pub fn fs_base(&self) -> u64 {
        self.fs_base
    }

    #[allow(dead_code)]
    pub fn set_fs_base(&mut self, fs_base: u64) {
        self.fs_base = fs_base;
    }

    pub fn address_space(&self) -> AddressSpace {
        self.space
    }

    pub fn user_stack_top(&self) -> u64 {
        self.user_stack_top
    }

    pub fn clone_process(&self, new_space: AddressSpace) -> Self {
        Self {
            space: new_space,
            entry: self.entry,
            personality: self.personality,
            fs_base: self.fs_base,
            user_stack_top: self.user_stack_top,
            brk_start: self.brk_start,
            brk_current: self.brk_current,
            mmap_next: self.mmap_next,
            mappings: self.mappings.clone(),
            destroyed: false,
        }
    }

    pub fn brk(&mut self, new_brk: u64) -> u64 {
        if new_brk == 0 || new_brk < self.brk_start {
            return self.brk_current;
        }
        if new_brk >= 0x0000_7000_0000_0000 {
            return self.brk_current;
        }
        let current_page = (self.brk_current.saturating_add(PAGE_SIZE - 1)) & !(PAGE_SIZE - 1);
        let target_page = (new_brk.saturating_add(PAGE_SIZE - 1)) & !(PAGE_SIZE - 1);
        if target_page > current_page {
            let mut page = current_page;
            while page < target_page {
                let Some(frame) = memory::alloc_frame() else {
                    return self.brk_current;
                };
                let physical = frame.start_address();
                if let Some(virt) = paging::phys_to_virt(physical) {
                    unsafe {
                        ptr::write_bytes(virt as *mut u8, 0, PAGE_SIZE as usize);
                    }
                }
                if paging::map(
                    self.space,
                    page,
                    physical,
                    paging::MAP_USER | paging::MAP_WRITABLE | paging::MAP_NO_EXECUTE,
                )
                .is_err()
                {
                    let _ = memory::free_frame(frame);
                    return self.brk_current;
                }
                self.mappings.push(MappedPage {
                    virtual_address: page,
                    physical_address: physical,
                });
                page += PAGE_SIZE;
            }
        }
        self.brk_current = new_brk;
        self.brk_current
    }

    pub fn mmap_anonymous(
        &mut self,
        addr: u64,
        length: u64,
        prot: u64,
        flags: u64,
    ) -> Result<u64, ()> {
        if length == 0 {
            return Err(());
        }
        let aligned_length = (length.checked_add(PAGE_SIZE - 1).ok_or(())?) & !(PAGE_SIZE - 1);
        let map_fixed = flags & 0x10 != 0;
        let base_address = if map_fixed {
            if addr == 0
                || addr & (PAGE_SIZE - 1) != 0
                || addr.checked_add(aligned_length).ok_or(())? >= USER_STACK_START
            {
                return Err(());
            }
            addr
        } else if addr != 0
            && addr & (PAGE_SIZE - 1) == 0
            && addr.checked_add(aligned_length).ok_or(())? < USER_STACK_START
        {
            addr
        } else {
            let base = self.mmap_next;
            self.mmap_next = self.mmap_next.checked_add(aligned_length).ok_or(())?;
            base
        };
        let mut pte_flags = paging::MAP_USER;
        if prot & 2 != 0 {
            pte_flags |= paging::MAP_WRITABLE;
        }
        if prot & 4 == 0 {
            pte_flags |= paging::MAP_NO_EXECUTE;
        }
        let mut allocated = Vec::new();
        let mut page = base_address;
        while page < base_address + aligned_length {
            if map_fixed {
                if let Ok(Some(physical)) = paging::unmap(self.space, page) {
                    let _ = memory::free_frame(memory::PhysFrame(physical));
                    if let Some(pos) = self.mappings.iter().position(|m| m.virtual_address == page) {
                        self.mappings.remove(pos);
                    }
                }
            }
            let frame = memory::alloc_frame().ok_or(())?;
            let physical = frame.start_address();
            if let Some(virt) = paging::phys_to_virt(physical) {
                unsafe {
                    ptr::write_bytes(virt as *mut u8, 0, PAGE_SIZE as usize);
                }
            }
            if paging::map(self.space, page, physical, pte_flags).is_err() {
                let _ = memory::free_frame(frame);
                for (unmap_page, unmap_phys) in allocated {
                    let _ = paging::unmap(self.space, unmap_page);
                    let _ = memory::free_frame(memory::PhysFrame(unmap_phys));
                }
                return Err(());
            }
            allocated.push((page, physical));
            page += PAGE_SIZE;
        }
        for (mapped_page, physical) in allocated {
            self.mappings.push(MappedPage {
                virtual_address: mapped_page,
                physical_address: physical,
            });
        }
        Ok(base_address)
    }

    #[allow(dead_code)]
    pub fn mprotect(&mut self, addr: u64, length: u64, prot: u64) -> Result<(), ()> {
        if addr & (PAGE_SIZE - 1) != 0 || length == 0 {
            return Err(());
        }
        let aligned_length = (length.checked_add(PAGE_SIZE - 1).ok_or(())?) & !(PAGE_SIZE - 1);
        if addr.checked_add(aligned_length).ok_or(())? >= USER_ADDRESS_LIMIT {
            return Err(());
        }
        let mut flags = paging::MAP_USER;
        if prot & 2 != 0 {
            flags |= paging::MAP_WRITABLE;
        }
        if prot & 4 == 0 {
            flags |= paging::MAP_NO_EXECUTE;
        }
        let pages = (aligned_length / PAGE_SIZE) as usize;
        paging::protect(self.space, addr, pages, flags).map_err(|_| ())
    }

    pub fn munmap(&mut self, addr: u64, length: u64) -> Result<(), ()> {
        if addr & (PAGE_SIZE - 1) != 0 || length == 0 {
            return Err(());
        }
        let aligned_length = (length.checked_add(PAGE_SIZE - 1).ok_or(())?) & !(PAGE_SIZE - 1);
        let mut page = addr;
        while page < addr + aligned_length {
            if let Ok(Some(physical)) = paging::unmap(self.space, page) {
                let _ = memory::free_frame(memory::PhysFrame(physical));
                if let Some(pos) = self.mappings.iter().position(|m| m.virtual_address == page) {
                    self.mappings.remove(pos);
                }
            }
            page += PAGE_SIZE;
        }
        Ok(())
    }

    pub fn read_user_byte(&self, virtual_address: u64) -> Option<u8> {
        let translation = paging::translate_in(self.space, virtual_address)?;
        let physical = paging::phys_to_virt(translation.physical_address)?;
        Some(unsafe { (physical as *const u8).read_volatile() })
    }

    /// Tear down all user leaf mappings and then the process page tables.
    pub fn destroy(&mut self) -> Result<usize, ProcessError> {
        self.cleanup()
    }

    fn cleanup(&mut self) -> Result<usize, ProcessError> {
        if self.destroyed {
            return Ok(0);
        }

        while let Some(mapping) = self.mappings.pop() {
            let unmapped = paging::unmap(self.space, mapping.virtual_address)
                .map_err(ProcessError::Map)?
                .ok_or(ProcessError::Map(MapError::NoHhdm))?;
            if unmapped != mapping.physical_address || !memory::free_frame(PhysFrame(unmapped)) {
                return Err(ProcessError::FrameReleaseFailed);
            }
        }

        let freed_tables = paging::destroy_address_space(self.space).map_err(ProcessError::Map)?;
        self.destroyed = true;
        Ok(freed_tables)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub fn load_elf(bytes: &[u8]) -> Result<Process, ProcessError> {
    load_elf_with_args(bytes, &[])
}

pub fn load_elf_with_args(bytes: &[u8], args: &[&[u8]]) -> Result<Process, ProcessError> {
    load_elf_with_args_and_env(bytes, args, &[])
}

pub fn load_elf_with_args_and_env(
    bytes: &[u8],
    args: &[&[u8]],
    environment: &[&[u8]],
) -> Result<Process, ProcessError> {
    load_elf_with_personality(bytes, args, environment, ProcessPersonality::NativeVanta)
}

pub fn load_linux_elf(bytes: &[u8]) -> Result<Process, ProcessError> {
    load_linux_elf_with_args_and_env(bytes, &[], &[])
}

pub fn load_linux_elf_with_args_and_env(
    bytes: &[u8],
    args: &[&[u8]],
    environment: &[&[u8]],
) -> Result<Process, ProcessError> {
    vanta_linuxd::StaticElf::parse(bytes).map_err(ProcessError::LinuxElf)?;
    load_elf_with_personality(bytes, args, environment, ProcessPersonality::LinuxX86_64Static)
}

fn is_zero_base(image: &ElfImage<'_>) -> bool {
    for header in image.program_headers().filter(|h| h.kind == elf::PT_LOAD) {
        if header.virtual_address < DEFAULT_PIE_BASE {
            return true;
        }
    }
    false
}

fn read_interpreter_bytes(path: &str) -> Result<Vec<u8>, ProcessError> {
    if let Ok(bytes) = crate::vfs::read_root(path) {
        return Ok(bytes);
    }
    if let Some(stripped) = path.strip_prefix('/') {
        if let Ok(bytes) = crate::vfs::read_root(stripped) {
            return Ok(bytes);
        }
    }
    let compat_path = alloc::format!("/compat/linux{}", if path.starts_with('/') { path } else { "" });
    if let Ok(bytes) = crate::vfs::read_root(&compat_path) {
        return Ok(bytes);
    }
    crate::serial_println!("[process] failed to read interpreter from '{}'", path);
    Err(ProcessError::Elf(ElfError::DynamicInterpreter))
}

fn load_elf_with_personality(
    bytes: &[u8],
    args: &[&[u8]],
    environment: &[&[u8]],
    personality: ProcessPersonality,
) -> Result<Process, ProcessError> {
    let main_image = ElfImage::parse(bytes).map_err(ProcessError::Elf)?;
    let main_base = if main_image.is_pie() && is_zero_base(&main_image) {
        DEFAULT_PIE_BASE
    } else {
        0
    };
    let main_entry = main_base.wrapping_add(main_image.entry);

    let interp_data = if let Some(interp_path) = main_image.interpreter() {
        let interp_bytes = read_interpreter_bytes(interp_path)?;
        Some(interp_bytes)
    } else {
        None
    };

    let interp_image = if let Some(ref interp_bytes) = interp_data {
        Some(ElfImage::parse(interp_bytes).map_err(ProcessError::Elf)?)
    } else {
        None
    };

    let interp_base = if interp_image.is_some() {
        INTERP_BASE
    } else {
        0
    };

    let execution_entry = if let Some(ref interp) = interp_image {
        interp_base.wrapping_add(interp.entry)
    } else {
        main_entry
    };

    let mut plans = plan_pages_with_base(main_image, main_base)?;
    if let Some(ref interp) = interp_image {
        let interp_plans = plan_pages_with_base(*interp, interp_base)?;
        for plan in interp_plans {
            if let Some(existing) = plans.iter_mut().find(|p| p.virtual_address == plan.virtual_address) {
                existing.flags |= plan.flags;
                existing.executable |= plan.executable;
            } else {
                plans.push(plan);
            }
        }
    }

    let space = paging::create_address_space().map_err(ProcessError::Map)?;

    let mut max_main_segment_end = 0u64;
    for header in main_image
        .program_headers()
        .filter(|header| header.kind == elf::PT_LOAD)
    {
        let end = main_base
            .wrapping_add(header.virtual_address)
            .saturating_add(header.memory_size);
        if end > max_main_segment_end {
            max_main_segment_end = end;
        }
    }
    let brk_start = (max_main_segment_end + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

    let mut process = Process {
        space,
        entry: execution_entry,
        personality,
        fs_base: 0,
        user_stack_top: USER_STACK_TOP,
        brk_start,
        brk_current: brk_start,
        mmap_next: 0x0000_7000_0000_0000,
        mappings: Vec::new(),
        destroyed: false,
    };

    for plan in plans {
        let frame = memory::alloc_frame().ok_or(ProcessError::OutOfMemory)?;
        let physical_address = frame.start_address();
        let Some(virtual_address) = paging::phys_to_virt(physical_address) else {
            let _ = memory::free_frame(frame);
            return Err(ProcessError::Map(MapError::NoHhdm));
        };
        unsafe {
            ptr::write_bytes(virtual_address as *mut u8, 0, PAGE_SIZE as usize);
        }
        if let Err(error) = paging::map(space, plan.virtual_address, physical_address, plan.flags) {
            let _ = memory::free_frame(frame);
            return Err(ProcessError::Map(error));
        }
        process.mappings.push(MappedPage {
            virtual_address: plan.virtual_address,
            physical_address,
        });
    }

    for header in main_image
        .program_headers()
        .filter(|header| header.kind == elf::PT_LOAD)
    {
        copy_segment_with_base(&mut process, main_image, header, main_base)?;
    }

    if let Some(ref interp) = interp_image {
        for header in interp
            .program_headers()
            .filter(|header| header.kind == elf::PT_LOAD)
        {
            copy_segment_with_base(&mut process, *interp, header, interp_base)?;
        }
    }

    let stack_start = USER_STACK_START;
    for index in 0..USER_STACK_PAGES {
        let virtual_address = stack_start + index as u64 * PAGE_SIZE;
        let frame = memory::alloc_frame().ok_or(ProcessError::OutOfMemory)?;
        let physical_address = frame.start_address();
        let Some(physical_virtual_address) = paging::phys_to_virt(physical_address) else {
            let _ = memory::free_frame(frame);
            return Err(ProcessError::Map(MapError::NoHhdm));
        };
        unsafe {
            ptr::write_bytes(physical_virtual_address as *mut u8, 0, PAGE_SIZE as usize);
        }
        if let Err(error) = paging::map(
            process.space,
            virtual_address,
            physical_address,
            paging::MAP_USER | paging::MAP_WRITABLE | paging::MAP_NO_EXECUTE,
        ) {
            let _ = memory::free_frame(frame);
            return Err(ProcessError::Map(error));
        }
        process.mappings.push(MappedPage {
            virtual_address,
            physical_address,
        });
    }

    let phdr_vaddr = main_image.phdr_virtual_address(main_base);
    let phent = main_image.program_header_size() as u64;
    let phnum = main_image.program_header_count() as u64;

    initialize_stack(
        &mut process,
        args,
        environment,
        phdr_vaddr,
        phent,
        phnum,
        main_entry,
        interp_base,
    )?;
    Ok(process)
}

fn initialize_stack(
    process: &mut Process,
    args: &[&[u8]],
    environment: &[&[u8]],
    phdr_vaddr: u64,
    phent: u64,
    phnum: u64,
    main_entry: u64,
    interp_base: u64,
) -> Result<(), ProcessError> {
    let mut stack_pointer = USER_STACK_TOP;
    let mut argument_pointers = Vec::new();
    let mut environment_pointers = Vec::new();
    for argument in args {
        let size = argument
            .len()
            .checked_add(1)
            .ok_or(ProcessError::InvalidUserAddress)?;
        stack_pointer = stack_pointer
            .checked_sub(size as u64)
            .ok_or(ProcessError::InvalidUserAddress)?;
        write_user_bytes(process.space, stack_pointer, argument)?;
        write_user_byte(process.space, stack_pointer + argument.len() as u64, 0)?;
        argument_pointers.push(stack_pointer);
    }
    for value in environment {
        let size = value
            .len()
            .checked_add(1)
            .ok_or(ProcessError::InvalidUserAddress)?;
        stack_pointer = stack_pointer
            .checked_sub(size as u64)
            .ok_or(ProcessError::InvalidUserAddress)?;
        write_user_bytes(process.space, stack_pointer, value)?;
        write_user_byte(process.space, stack_pointer + value.len() as u64, 0)?;
        environment_pointers.push(stack_pointer);
    }

    // 16 random bytes for AT_RANDOM
    stack_pointer = stack_pointer
        .checked_sub(16)
        .ok_or(ProcessError::InvalidUserAddress)?;
    let random_bytes = [0x5au8; 16];
    write_user_bytes(process.space, stack_pointer, &random_bytes)?;
    let random_ptr = stack_pointer;

    stack_pointer &= !15;

    // Push auxiliary vector (auxv)
    let auxv: &[(u64, u64)] = &[
        (0, 0),                       // AT_NULL (0)
        (25, random_ptr),             // AT_RANDOM (25)
        (23, 0),                      // AT_SECURE (23)
        (17, 100),                    // AT_CLKTCK (17)
        (14, 0),                      // AT_EGID (14)
        (13, 0),                      // AT_GID (13)
        (12, 0),                      // AT_EUID (12)
        (11, 0),                      // AT_UID (11)
        (9, main_entry),              // AT_ENTRY (9)
        (8, 0),                       // AT_FLAGS (8)
        (7, interp_base),             // AT_BASE (7)
        (6, PAGE_SIZE),               // AT_PAGESZ (6)
        (5, phnum),                   // AT_PHNUM (5)
        (4, phent),                   // AT_PHENT (4)
        (3, phdr_vaddr),              // AT_PHDR (3)
    ];

    for (key, val) in auxv {
        stack_pointer = stack_pointer
            .checked_sub(8)
            .ok_or(ProcessError::InvalidUserAddress)?;
        write_user_u64(process.space, stack_pointer, *val)?;
        stack_pointer = stack_pointer
            .checked_sub(8)
            .ok_or(ProcessError::InvalidUserAddress)?;
        write_user_u64(process.space, stack_pointer, *key)?;
    }

    // Terminate envp
    stack_pointer = stack_pointer
        .checked_sub(8)
        .ok_or(ProcessError::InvalidUserAddress)?;
    write_user_u64(process.space, stack_pointer, 0)?;
    for pointer in environment_pointers.iter().rev() {
        stack_pointer = stack_pointer
            .checked_sub(8)
            .ok_or(ProcessError::InvalidUserAddress)?;
        write_user_u64(process.space, stack_pointer, *pointer)?;
    }

    // Terminate argv
    stack_pointer = stack_pointer
        .checked_sub(8)
        .ok_or(ProcessError::InvalidUserAddress)?;
    write_user_u64(process.space, stack_pointer, 0)?;
    for pointer in argument_pointers.iter().rev() {
        stack_pointer = stack_pointer
            .checked_sub(8)
            .ok_or(ProcessError::InvalidUserAddress)?;
        write_user_u64(process.space, stack_pointer, *pointer)?;
    }

    // argc
    stack_pointer = stack_pointer
        .checked_sub(8)
        .ok_or(ProcessError::InvalidUserAddress)?;
    write_user_u64(process.space, stack_pointer, args.len() as u64)?;

    process.user_stack_top = stack_pointer;
    Ok(())
}

pub fn write_user_byte_in(space: AddressSpace, address: u64, value: u8) -> Result<(), ProcessError> {
    let translation =
        paging::translate_in(space, address).ok_or(ProcessError::InvalidUserAddress)?;
    let physical = paging::phys_to_virt(translation.physical_address)
        .ok_or(ProcessError::InvalidUserAddress)?;
    unsafe { (physical as *mut u8).write(value) };
    Ok(())
}

pub fn write_user_bytes_in(space: AddressSpace, address: u64, bytes: &[u8]) -> Result<(), ProcessError> {
    for (offset, byte) in bytes.iter().enumerate() {
        write_user_byte_in(space, address + offset as u64, *byte)?;
    }
    Ok(())
}

pub fn write_user_u32_in(space: AddressSpace, address: u64, value: u32) -> Result<(), ProcessError> {
    write_user_bytes_in(space, address, &value.to_ne_bytes())
}

pub fn write_user_u64_in(space: AddressSpace, address: u64, value: u64) -> Result<(), ProcessError> {
    write_user_bytes_in(space, address, &value.to_ne_bytes())
}

#[allow(dead_code)]
pub fn read_user_byte_in(space: AddressSpace, address: u64) -> Result<u8, ProcessError> {
    let translation =
        paging::translate_in(space, address).ok_or(ProcessError::InvalidUserAddress)?;
    let physical = paging::phys_to_virt(translation.physical_address)
        .ok_or(ProcessError::InvalidUserAddress)?;
    Ok(unsafe { (physical as *const u8).read() })
}

#[allow(dead_code)]
pub fn read_user_bytes_in(space: AddressSpace, address: u64, buf: &mut [u8]) -> Result<(), ProcessError> {
    for (offset, byte) in buf.iter_mut().enumerate() {
        *byte = read_user_byte_in(space, address + offset as u64)?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn read_user_u32_in(space: AddressSpace, address: u64) -> Result<u32, ProcessError> {
    let mut bytes = [0u8; 4];
    read_user_bytes_in(space, address, &mut bytes)?;
    Ok(u32::from_ne_bytes(bytes))
}

fn write_user_byte(space: AddressSpace, address: u64, value: u8) -> Result<(), ProcessError> {
    write_user_byte_in(space, address, value)
}

fn write_user_bytes(space: AddressSpace, address: u64, bytes: &[u8]) -> Result<(), ProcessError> {
    write_user_bytes_in(space, address, bytes)
}

fn write_user_u64(space: AddressSpace, address: u64, value: u64) -> Result<(), ProcessError> {
    write_user_u64_in(space, address, value)
}

#[allow(dead_code)]
fn plan_pages(image: ElfImage<'_>) -> Result<Vec<PagePlan>, ProcessError> {
    plan_pages_with_base(image, 0)
}

fn plan_pages_with_base(
    image: ElfImage<'_>,
    base_address: u64,
) -> Result<Vec<PagePlan>, ProcessError> {
    let mut plans: Vec<PagePlan> = Vec::new();
    let mut executable_entry = false;
    let entry_vaddr = base_address.wrapping_add(image.entry);

    for header in image
        .program_headers()
        .filter(|header| header.kind == elf::PT_LOAD)
    {
        validate_segment_with_base(image, header, base_address)?;
        let seg_vaddr = base_address.wrapping_add(header.virtual_address);
        if header.flags & elf::PF_X != 0
            && entry_vaddr >= seg_vaddr
            && entry_vaddr < seg_vaddr + header.memory_size
        {
            executable_entry = true;
        }

        let segment_end = seg_vaddr
            .checked_add(header.memory_size)
            .ok_or(ProcessError::InvalidLoadSegment)?;
        let first_page = seg_vaddr & !(PAGE_SIZE - 1);
        let last_page = align_up(segment_end).ok_or(ProcessError::InvalidLoadSegment)?;
        let mut virtual_address = first_page;
        while virtual_address < last_page {
            let executable = header.flags & elf::PF_X != 0;
            let writable = header.flags & elf::PF_W != 0;
            let flags = paging::MAP_USER
                | if writable { paging::MAP_WRITABLE } else { 0 }
                | if executable {
                    0
                } else {
                    paging::MAP_NO_EXECUTE
                };
            if let Some(plan) = plans
                .iter_mut()
                .find(|plan| plan.virtual_address == virtual_address)
            {
                plan.flags |= flags;
                plan.executable |= executable;
                if plan.executable {
                    plan.flags &= !paging::MAP_NO_EXECUTE;
                } else {
                    plan.flags |= paging::MAP_NO_EXECUTE;
                }
            } else {
                plans.push(PagePlan {
                    virtual_address,
                    flags,
                    executable,
                });
            }
            virtual_address = virtual_address
                .checked_add(PAGE_SIZE)
                .ok_or(ProcessError::InvalidLoadSegment)?;
        }
    }

    if !executable_entry || plans.is_empty() {
        return Err(ProcessError::EntryNotExecutable);
    }
    Ok(plans)
}

#[allow(dead_code)]
fn validate_segment(image: ElfImage<'_>, header: ProgramHeader) -> Result<(), ProcessError> {
    validate_segment_with_base(image, header, 0)
}

fn validate_segment_with_base(
    image: ElfImage<'_>,
    header: ProgramHeader,
    base_address: u64,
) -> Result<(), ProcessError> {
    if header.memory_size < header.file_size || image.file_bytes(header).is_none() {
        return Err(ProcessError::InvalidLoadSegment);
    }
    let seg_vaddr = base_address.wrapping_add(header.virtual_address);
    let end = seg_vaddr
        .checked_add(header.memory_size)
        .ok_or(ProcessError::InvalidLoadSegment)?;
    if header.memory_size == 0
        || seg_vaddr < PAGE_SIZE
        || end > USER_ADDRESS_LIMIT
        || end <= seg_vaddr
        || (seg_vaddr < USER_STACK_TOP && end > USER_STACK_START)
    {
        return Err(ProcessError::InvalidUserAddress);
    }
    Ok(())
}

#[allow(dead_code)]
fn copy_segment(
    process: &mut Process,
    image: ElfImage<'_>,
    header: ProgramHeader,
) -> Result<(), ProcessError> {
    copy_segment_with_base(process, image, header, 0)
}

fn copy_segment_with_base(
    process: &mut Process,
    image: ElfImage<'_>,
    header: ProgramHeader,
    base_address: u64,
) -> Result<(), ProcessError> {
    let Some(file_bytes) = image.file_bytes(header) else {
        return Err(ProcessError::InvalidLoadSegment);
    };
    let mut copied = 0u64;
    while copied < header.file_size {
        let virtual_address = base_address
            .wrapping_add(header.virtual_address)
            .checked_add(copied)
            .ok_or(ProcessError::InvalidLoadSegment)?;
        let page = virtual_address & !(PAGE_SIZE - 1);
        let in_page = virtual_address - page;
        let count = (PAGE_SIZE - in_page).min(header.file_size - copied);
        let Some(mapping) = process
            .mappings
            .iter()
            .find(|mapping| mapping.virtual_address == page)
        else {
            return Err(ProcessError::InvalidLoadSegment);
        };
        let Some(destination) = paging::phys_to_virt(mapping.physical_address) else {
            return Err(ProcessError::Map(MapError::NoHhdm));
        };
        unsafe {
            ptr::copy_nonoverlapping(
                file_bytes.as_ptr().add(copied as usize),
                (destination + in_page) as *mut u8,
                count as usize,
            );
        }
        copied += count;
    }
    Ok(())
}

fn align_up(address: u64) -> Option<u64> {
    address
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value & !(PAGE_SIZE - 1))
}
