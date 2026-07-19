//! Single-CPU syscall ABI and dispatch path.

use core::arch::global_asm;

use spin::Mutex;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask, Star};
use x86_64::registers::rflags::RFlags;
use x86_64::VirtAddr;

use crate::paging::{self, AddressSpace};

pub const SYS_WRITE: u64 = 1;
pub const SYS_EXIT: u64 = 60;
const SYSCALL_RETURN_EXIT: u64 = u64::MAX;
const SYSCALL_ERROR: u64 = u64::MAX - 1;
const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;
const SYSCALL_STACK_SIZE: usize = 4096 * 2;

#[no_mangle]
static mut VANTA_SYSCALL_STACK: [u8; SYSCALL_STACK_SIZE] = [0; SYSCALL_STACK_SIZE];

#[no_mangle]
static mut VANTA_SYSCALL_USER_RSP: u64 = 0;

#[no_mangle]
static mut VANTA_SYSCALL_EXIT_CODE: u64 = 0;

struct CurrentProcess {
    process: usize,
    kernel_space: AddressSpace,
}

static CURRENT_PROCESS: Mutex<Option<CurrentProcess>> = Mutex::new(None);

global_asm!(
    r#"
    .global vanta_syscall_entry
    .extern vanta_syscall_dispatch
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
    mov r11, [rsp + 48]
    mov rcx, [rsp + 40]
    add rsp, 56
    mov rsp, [rip + VANTA_SYSCALL_USER_RSP]
    sysretq
vanta_syscall_exit_path:
    mov rdi, [rip + VANTA_SYSCALL_EXIT_CODE]
    call vanta_syscall_exit
    ud2
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

pub fn register_process(process: *mut crate::process::Process, kernel_space: AddressSpace) {
    *CURRENT_PROCESS.lock() = Some(CurrentProcess {
        process: process as usize,
        kernel_space,
    });
}

pub fn take_current_process() -> Option<(usize, AddressSpace)> {
    CURRENT_PROCESS
        .lock()
        .take()
        .map(|current| (current.process, current.kernel_space))
}

#[no_mangle]
extern "C" fn vanta_syscall_dispatch(
    number: u64,
    arg1: u64,
    arg2: u64,
    _arg3: u64,
    _arg4: u64,
) -> u64 {
    match number {
        SYS_WRITE => write_user(arg1, arg2),
        SYS_EXIT => {
            unsafe {
                VANTA_SYSCALL_EXIT_CODE = arg1;
            }
            SYSCALL_RETURN_EXIT
        }
        _ => SYSCALL_ERROR,
    }
}

fn write_user(pointer: u64, length: u64) -> u64 {
    if length > 256 {
        return SYSCALL_ERROR;
    }
    for offset in 0..length {
        let Some(address) = pointer.checked_add(offset) else {
            return SYSCALL_ERROR;
        };
        if address >= USER_ADDRESS_LIMIT {
            return SYSCALL_ERROR;
        }
        let Some(flags) = paging::flags_in(paging::current_address_space(), address) else {
            return SYSCALL_ERROR;
        };
        if flags & paging::MAP_USER == 0 {
            return SYSCALL_ERROR;
        }
        let Some(translation) = paging::translate(address) else {
            return SYSCALL_ERROR;
        };
        let Some(physical) = paging::phys_to_virt(translation.physical_address) else {
            return SYSCALL_ERROR;
        };
        let byte = unsafe { (physical as *const u8).read_volatile() };
        crate::serial::_print(format_args!("{}", byte as char));
    }
    length
}

#[no_mangle]
extern "C" fn vanta_syscall_exit(code: u64) -> ! {
    unsafe { crate::process::exit_current(code) }
}
