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

    /// Switch to this process and enter its ELF entry point at CPL3.
    pub unsafe fn run(self: alloc::boxed::Box<Self>) -> ! {
        let process_ptr = alloc::boxed::Box::into_raw(self);
        let process = unsafe { &*process_ptr };
        let entry = process.entry;
        let stack = process.user_stack_top;
        let space = process.space;
        crate::syscall::register_process(process_ptr, paging::current_address_space());
        unsafe {
            paging::activate(space);
            crate::gdt::enter_user(entry, stack)
        }
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

pub unsafe fn exit_current(code: u64) -> ! {
    let Some((process_ptr, kernel_space)) = crate::syscall::take_current_process() else {
        panic!("syscall exit without a current process");
    };
    unsafe {
        paging::activate(kernel_space);
        let mut process = alloc::boxed::Box::from_raw(process_ptr as *mut Process);
        let cleanup = process.destroy();
        drop(process);
        match cleanup {
            Ok(freed_tables) => crate::serial_println!(
                "[proc] user process exited: code={} tables-freed={}",
                code,
                freed_tables
            ),
            Err(error) => crate::serial_println!(
                "[proc] user process exit cleanup failed: code={} error={:?}",
                code,
                error
            ),
        }
    }
    x86_64::instructions::interrupts::enable();
    crate::shell::run()
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub fn load_elf(bytes: &[u8]) -> Result<Process, ProcessError> {
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

    Ok(process)
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
