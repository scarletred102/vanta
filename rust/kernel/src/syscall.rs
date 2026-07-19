//! Single-CPU syscall ABI and dispatch path.

use core::arch::global_asm;

use alloc::vec::Vec;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask, Star};
use x86_64::registers::rflags::RFlags;
use x86_64::VirtAddr;

use crate::paging::{self, AddressSpace};

pub const SYS_READ: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_YIELD: u64 = 24;
pub const SYS_GETPID: u64 = 39;
pub const SYS_EXIT: u64 = 60;
const SYSCALL_RETURN_EXIT: u64 = u64::MAX;
const SYSCALL_RETURN_YIELD: u64 = u64::MAX - 2;
const SYSCALL_ERROR: u64 = u64::MAX - 1;
const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;
const SYSCALL_STACK_SIZE: usize = 4096 * 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserContext {
    pub instruction_pointer: u64,
    pub flags: u64,
    pub stack_pointer: u64,
}

#[no_mangle]
static mut VANTA_SYSCALL_STACK: [u8; SYSCALL_STACK_SIZE] = [0; SYSCALL_STACK_SIZE];

#[no_mangle]
static mut VANTA_SYSCALL_USER_RSP: u64 = 0;

#[no_mangle]
static mut VANTA_SYSCALL_EXIT_CODE: u64 = 0;

global_asm!(
    r#"
    .global vanta_syscall_entry
    .extern vanta_syscall_dispatch
    .extern vanta_syscall_yield
    .extern vanta_syscall_exit
vanta_syscall_entry:
    mov [rip + VANTA_SYSCALL_USER_RSP], rsp
    lea rsp, [rip + VANTA_SYSCALL_STACK]
    add rsp, 8192
    push r11
    push rcx
    push r10
    push rdx
    push rsi
    push rdi
    push rax
    mov rdi, [rsp]
    mov rsi, [rsp + 8]
    mov rdx, [rsp + 16]
    mov rcx, [rsp + 24]
    mov r8, [rsp + 32]
    call vanta_syscall_dispatch
    cmp rax, -1
    je vanta_syscall_exit_path
    cmp rax, -3
    je vanta_syscall_yield_path
    mov r11, [rsp + 48]
    mov rcx, [rsp + 40]
    add rsp, 56
    mov rsp, [rip + VANTA_SYSCALL_USER_RSP]
    sysretq
vanta_syscall_yield_path:
    mov rdi, [rsp + 40]
    mov rsi, [rsp + 48]
    mov rdx, [rip + VANTA_SYSCALL_USER_RSP]
    call vanta_syscall_yield
    mov rcx, [rax]
    mov r11, [rax + 8]
    mov rsp, [rax + 16]
    sysretq
vanta_syscall_exit_path:
    mov rdi, [rip + VANTA_SYSCALL_EXIT_CODE]
    call vanta_syscall_exit
    mov rcx, [rax]
    mov r11, [rax + 8]
    mov rsp, [rax + 16]
    sysretq
"#
);

extern "C" {
    fn vanta_syscall_entry();
}

pub fn init() -> bool {
    let (user_code, user_data, kernel_code, kernel_data) = crate::gdt::syscall_selectors();
    if Star::write(user_code, user_data, kernel_code, kernel_data).is_err() {
        return false;
    }
    LStar::write(VirtAddr::new(
        vanta_syscall_entry as *const () as usize as u64,
    ));
    SFMask::write(RFlags::INTERRUPT_FLAG);
    unsafe {
        Efer::update(|flags| flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS));
    }
    true
}

#[no_mangle]
extern "C" fn vanta_syscall_dispatch(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    _arg4: u64,
) -> u64 {
    match number {
        SYS_READ => read_user(arg1, arg2, arg3),
        SYS_WRITE => write_user(arg1, arg2),
        SYS_OPEN => open_user(arg1, arg2),
        SYS_CLOSE => close_user(arg1),
        SYS_YIELD => SYSCALL_RETURN_YIELD,
        SYS_GETPID => crate::scheduler::current_pid(),
        SYS_EXIT => {
            unsafe {
                VANTA_SYSCALL_EXIT_CODE = arg1;
            }
            SYSCALL_RETURN_EXIT
        }
        _ => SYSCALL_ERROR,
    }
}

pub fn prepare_user_return(context: UserContext, space: AddressSpace) -> *const UserContext {
    unsafe {
        NEXT_CONTEXT = context;
    }
    unsafe {
        paging::activate(space);
    }
    core::ptr::addr_of!(NEXT_CONTEXT)
}

static mut NEXT_CONTEXT: UserContext = UserContext {
    instruction_pointer: 0,
    flags: 0,
    stack_pointer: 0,
};

fn write_user(pointer: u64, length: u64) -> u64 {
    let Ok(bytes) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    for byte in bytes {
        crate::serial::_print(format_args!("{}", byte as char));
    }
    length
}

fn open_user(pointer: u64, length: u64) -> u64 {
    let Ok(path) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        return SYSCALL_ERROR;
    };
    let Ok(contents) = crate::vfs::read_root(path) else {
        return SYSCALL_ERROR;
    };
    crate::scheduler::open_current(contents).unwrap_or(SYSCALL_ERROR)
}

fn read_user(descriptor: u64, pointer: u64, length: u64) -> u64 {
    if length > 256 {
        return SYSCALL_ERROR;
    }
    let Ok(bytes) = crate::scheduler::read_current(descriptor, length as usize) else {
        return SYSCALL_ERROR;
    };
    if copy_to_user(pointer, &bytes).is_err() {
        return SYSCALL_ERROR;
    }
    bytes.len() as u64
}

fn close_user(descriptor: u64) -> u64 {
    crate::scheduler::close_current(descriptor)
        .map(|()| 0)
        .unwrap_or(SYSCALL_ERROR)
}

fn copy_from_user(pointer: u64, length: u64, writable: bool) -> Result<Vec<u8>, ()> {
    if length > 256 {
        return Err(());
    }
    let mut bytes = Vec::with_capacity(length as usize);
    for offset in 0..length {
        bytes.push(read_user_byte(
            pointer.checked_add(offset).ok_or(())?,
            writable,
        )?);
    }
    Ok(bytes)
}

fn copy_to_user(pointer: u64, bytes: &[u8]) -> Result<(), ()> {
    for (offset, byte) in bytes.iter().enumerate() {
        let address = pointer.checked_add(offset as u64).ok_or(())?;
        let physical = user_physical_address(address, true)?;
        unsafe { (physical as *mut u8).write_volatile(*byte) };
    }
    Ok(())
}

fn read_user_byte(address: u64, writable: bool) -> Result<u8, ()> {
    let physical = user_physical_address(address, writable)?;
    Ok(unsafe { (physical as *const u8).read_volatile() })
}

fn user_physical_address(address: u64, writable: bool) -> Result<u64, ()> {
    if address >= USER_ADDRESS_LIMIT {
        return Err(());
    }
    let flags = paging::flags_in(paging::current_address_space(), address).ok_or(())?;
    if flags & paging::MAP_USER == 0 || (writable && flags & paging::MAP_WRITABLE == 0) {
        return Err(());
    }
    let translation = paging::translate(address).ok_or(())?;
    paging::phys_to_virt(translation.physical_address).ok_or(())
}

#[no_mangle]
extern "C" fn vanta_syscall_yield(
    instruction_pointer: u64,
    flags: u64,
    stack_pointer: u64,
) -> *const UserContext {
    crate::scheduler::yield_current(UserContext {
        instruction_pointer,
        flags,
        stack_pointer,
    })
}

#[no_mangle]
extern "C" fn vanta_syscall_exit(code: u64) -> *const UserContext {
    crate::scheduler::exit_current(code)
}
