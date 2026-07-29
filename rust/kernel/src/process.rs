//! First user-process lifecycle: ELF segments, user stack, and ring-3 entry.

use alloc::vec::Vec;
use core::ptr;

use crate::elf::{self, ElfError, ElfImage, ProgramHeader};
use crate::memory::{self, PhysFrame, PAGE_SIZE};
use crate::paging::{self, AddressSpace, MapError};

const USER_STACK_TOP: u64 = 0x0000_7fff_ffff_f000;
const USER_STACK_PAGES: usize = 4;
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
    Map(MapError),
    OutOfMemory,
    InvalidLoadSegment,
    InvalidUserAddress,
    EntryNotExecutable,
    FrameReleaseFailed,
}

pub struct Process {
    space: AddressSpace,
    entry: u64,
    user_stack_top: u64,
    mappings: Vec<MappedPage>,
    destroyed: bool,
}

impl Process {
    pub fn entry(&self) -> u64 {
        self.entry
    }

    pub fn address_space(&self) -> AddressSpace {
        self.space
    }

    pub fn user_stack_top(&self) -> u64 {
        self.user_stack_top
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
    let image = ElfImage::parse(bytes).map_err(ProcessError::Elf)?;
    let plans = plan_pages(image)?;
    let space = paging::create_address_space().map_err(ProcessError::Map)?;
    let mut process = Process {
        space,
        entry: image.entry,
        user_stack_top: USER_STACK_TOP,
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

    for header in image
        .program_headers()
        .filter(|header| header.kind == elf::PT_LOAD)
    {
        copy_segment(&mut process, image, header)?;
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

    initialize_stack(&mut process, args)?;
    Ok(process)
}

fn initialize_stack(process: &mut Process, args: &[&[u8]]) -> Result<(), ProcessError> {
    let mut stack_pointer = USER_STACK_TOP;
    let mut argument_pointers = Vec::new();
    for argument in args.iter().rev() {
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
    stack_pointer &= !15;
    stack_pointer = stack_pointer
        .checked_sub(8)
        .ok_or(ProcessError::InvalidUserAddress)?;
    write_user_u64(process.space, stack_pointer, 0)?;
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
    stack_pointer = stack_pointer
        .checked_sub(8)
        .ok_or(ProcessError::InvalidUserAddress)?;
    write_user_u64(process.space, stack_pointer, args.len() as u64)?;
    process.user_stack_top = stack_pointer;
    Ok(())
}

fn write_user_byte(space: AddressSpace, address: u64, value: u8) -> Result<(), ProcessError> {
    let translation =
        paging::translate_in(space, address).ok_or(ProcessError::InvalidUserAddress)?;
    let physical = paging::phys_to_virt(translation.physical_address)
        .ok_or(ProcessError::InvalidUserAddress)?;
    unsafe { (physical as *mut u8).write(value) };
    Ok(())
}

fn write_user_bytes(space: AddressSpace, address: u64, bytes: &[u8]) -> Result<(), ProcessError> {
    for (offset, byte) in bytes.iter().enumerate() {
        write_user_byte(space, address + offset as u64, *byte)?;
    }
    Ok(())
}

fn write_user_u64(space: AddressSpace, address: u64, value: u64) -> Result<(), ProcessError> {
    write_user_bytes(space, address, &value.to_ne_bytes())
}

fn plan_pages(image: ElfImage<'_>) -> Result<Vec<PagePlan>, ProcessError> {
    let mut plans: Vec<PagePlan> = Vec::new();
    let mut executable_entry = false;

    for header in image
        .program_headers()
        .filter(|header| header.kind == elf::PT_LOAD)
    {
        validate_segment(image, header)?;
        if header.flags & elf::PF_X != 0
            && image.entry >= header.virtual_address
            && image.entry < header.virtual_address + header.memory_size
        {
            executable_entry = true;
        }

        let segment_end = header
            .virtual_address
            .checked_add(header.memory_size)
            .ok_or(ProcessError::InvalidLoadSegment)?;
        let first_page = header.virtual_address & !(PAGE_SIZE - 1);
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

fn validate_segment(image: ElfImage<'_>, header: ProgramHeader) -> Result<(), ProcessError> {
    if header.memory_size < header.file_size || image.file_bytes(header).is_none() {
        return Err(ProcessError::InvalidLoadSegment);
    }
    let end = header
        .virtual_address
        .checked_add(header.memory_size)
        .ok_or(ProcessError::InvalidLoadSegment)?;
    if header.memory_size == 0
        || header.virtual_address < PAGE_SIZE
        || end > USER_ADDRESS_LIMIT
        || end <= header.virtual_address
        || (header.virtual_address < USER_STACK_TOP && end > USER_STACK_START)
    {
        return Err(ProcessError::InvalidUserAddress);
    }
    Ok(())
}

fn copy_segment(
    process: &mut Process,
    image: ElfImage<'_>,
    header: ProgramHeader,
) -> Result<(), ProcessError> {
    let Some(file_bytes) = image.file_bytes(header) else {
        return Err(ProcessError::InvalidLoadSegment);
    };
    let mut copied = 0u64;
    while copied < header.file_size {
        let virtual_address = header
            .virtual_address
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
