//! Syscall ABI with GS-selected per-CPU entry state.

use core::arch::asm;
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
pub const SYS_LSEEK: u64 = 8;
pub const SYS_YIELD: u64 = 24;
pub const SYS_DUP: u64 = 32;
pub const SYS_GETPID: u64 = 39;
pub const SYS_GETPPID: u64 = 110;
pub const SYS_EXEC: u64 = 59;
pub const SYS_EXIT: u64 = 60;
pub const SYS_WAITPID: u64 = 61;
pub const SYS_SPAWN: u64 = 400;
const SYSCALL_RETURN_EXIT: u64 = u64::MAX;
const SYSCALL_RETURN_YIELD: u64 = u64::MAX - 2;
const SYSCALL_RETURN_WAIT: u64 = u64::MAX - 3;
const SYSCALL_RETURN_EXEC: u64 = u64::MAX - 4;
const SYSCALL_ERROR: u64 = u64::MAX - 1;
const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;
const SYSCALL_STACK_SIZE: usize = 4096 * 2;
const MAX_CPUS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserContext {
    pub return_value: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub instruction_pointer: u64,
    pub flags: u64,
    pub stack_pointer: u64,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct CpuLocal {
    self_pointer: u64,
    syscall_stack: [u8; SYSCALL_STACK_SIZE],
    syscall_stack_top: u64,
    user_rsp: u64,
    exit_code: u64,
    next_context: UserContext,
    cpu_index: usize,
}

const EMPTY_CPU_LOCAL: CpuLocal = CpuLocal {
    self_pointer: 0,
    syscall_stack: [0; SYSCALL_STACK_SIZE],
    syscall_stack_top: 0,
    user_rsp: 0,
    exit_code: 0,
    next_context: UserContext {
        return_value: 0,
        rbx: 0,
        rbp: 0,
        r12: 0,
        r13: 0,
        r14: 0,
        r15: 0,
        instruction_pointer: 0,
        flags: 0,
        stack_pointer: 0,
    },
    cpu_index: 0,
};

static mut CPU_LOCALS: [CpuLocal; MAX_CPUS] = [EMPTY_CPU_LOCAL; MAX_CPUS];

global_asm!(
    r#"
    .global vanta_syscall_entry
    .extern vanta_syscall_dispatch
    .extern vanta_syscall_yield
    .extern vanta_syscall_wait
    .extern vanta_syscall_exec
    .extern vanta_syscall_exit
vanta_syscall_entry:
    mov gs:[8208], rsp
    mov rsp, gs:[8200]
    push r15
    push r14
    push r13
    push r12
    push rbp
    push rbx
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
    cmp rax, -4
    je vanta_syscall_wait_path
    cmp rax, -5
    je vanta_syscall_exec_path
    mov r11, [rsp + 48]
    mov rcx, [rsp + 40]
    add rsp, 104
    mov rsp, gs:[8208]
    sysretq
vanta_syscall_yield_path:
    mov rdi, rsp
    mov rsi, gs:[8208]
    call vanta_syscall_yield
    jmp vanta_syscall_restore_context
vanta_syscall_wait_path:
    mov rdi, rsp
    mov rsi, gs:[8208]
    call vanta_syscall_wait
    jmp vanta_syscall_restore_context
vanta_syscall_exec_path:
    mov rdi, rsp
    call vanta_syscall_exec
    test rax, rax
    jz vanta_syscall_exec_error
    jmp vanta_syscall_restore_context
vanta_syscall_exec_error:
    mov r11, [rsp + 48]
    mov rcx, [rsp + 40]
    add rsp, 104
    mov rsp, gs:[8208]
    mov rax, -2
    sysretq
vanta_syscall_exit_path:
    mov rdi, gs:[8216]
    call vanta_syscall_exit
vanta_syscall_restore_context:
    mov r10, rax
    mov rbx, [r10 + 8]
    mov rbp, [r10 + 16]
    mov r12, [r10 + 24]
    mov r13, [r10 + 32]
    mov r14, [r10 + 40]
    mov r15, [r10 + 48]
    mov rcx, [r10 + 56]
    mov r11, [r10 + 64]
    mov rsp, [r10 + 72]
    mov rax, [r10]
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

pub fn initialize_cpu_local(index: usize) -> bool {
    if index >= MAX_CPUS {
        return false;
    }
    let local = unsafe {
        core::ptr::addr_of_mut!(CPU_LOCALS)
            .cast::<CpuLocal>()
            .add(index)
    };
    unsafe {
        (*local).self_pointer = local as u64;
        (*local).syscall_stack_top = core::ptr::addr_of!((*local).syscall_stack)
            .cast::<u8>()
            .add(SYSCALL_STACK_SIZE) as u64;
        (*local).cpu_index = index;
    }
    let mut gs_base = x86_64::registers::model_specific::Msr::new(0xc000_0101);
    unsafe {
        gs_base.write(local as u64);
    }
    true
}

pub fn current_cpu_index() -> usize {
    current_cpu_local().cpu_index
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
        SYS_WRITE => write_user(arg1, arg2, arg3),
        SYS_OPEN => open_user(arg1, arg2),
        SYS_CLOSE => close_user(arg1),
        SYS_LSEEK => seek_user(arg1, arg2 as i64, arg3),
        SYS_DUP => duplicate_user(arg1),
        SYS_SPAWN => spawn_user(arg1, arg2),
        SYS_WAITPID => waitpid_user(arg1),
        SYS_EXEC => SYSCALL_RETURN_EXEC,
        SYS_YIELD => SYSCALL_RETURN_YIELD,
        SYS_GETPID => crate::scheduler::current_pid(),
        SYS_GETPPID => crate::scheduler::current_parent_pid(),
        SYS_EXIT => {
            current_cpu_local().exit_code = arg1;
            SYSCALL_RETURN_EXIT
        }
        _ => SYSCALL_ERROR,
    }
}

pub fn prepare_user_return(context: UserContext, space: AddressSpace) -> *const UserContext {
    current_cpu_local().next_context = context;
    unsafe {
        paging::activate(space);
    }
    core::ptr::addr_of!(current_cpu_local().next_context)
}

fn current_cpu_local() -> &'static mut CpuLocal {
    let pointer: u64;
    unsafe {
        asm!("mov {pointer}, gs:[0]", pointer = out(reg) pointer, options(nostack, preserves_flags));
        &mut *(pointer as *mut CpuLocal)
    }
}

fn write_user(descriptor: u64, pointer: u64, length: u64) -> u64 {
    if descriptor != 1 && descriptor != 2 {
        return SYSCALL_ERROR;
    }
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

fn seek_user(descriptor: u64, offset: i64, whence: u64) -> u64 {
    crate::scheduler::seek_current(descriptor, offset, whence).unwrap_or(SYSCALL_ERROR)
}

fn duplicate_user(descriptor: u64) -> u64 {
    crate::scheduler::duplicate_current(descriptor).unwrap_or(SYSCALL_ERROR)
}

fn spawn_user(pointer: u64, length: u64) -> u64 {
    let Ok(path) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        return SYSCALL_ERROR;
    };
    let Ok(image) = crate::vfs::read_root(path) else {
        return SYSCALL_ERROR;
    };
    let Ok(process) = crate::process::load_elf(&image) else {
        return SYSCALL_ERROR;
    };
    crate::scheduler::spawn_current(alloc::boxed::Box::new(process)).unwrap_or(SYSCALL_ERROR)
}

fn waitpid_user(pid: u64) -> u64 {
    match crate::scheduler::wait_child_current(pid) {
        Ok(Some(code)) => code,
        Ok(None) => SYSCALL_RETURN_WAIT,
        Err(()) => SYSCALL_ERROR,
    }
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
extern "C" fn vanta_syscall_yield(frame: *const u64, stack_pointer: u64) -> *const UserContext {
    crate::scheduler::yield_current(user_context(frame, stack_pointer))
}

#[no_mangle]
extern "C" fn vanta_syscall_wait(frame: *const u64, stack_pointer: u64) -> *const UserContext {
    let child_pid = unsafe { *frame.add(1) };
    crate::scheduler::wait_current(child_pid, user_context(frame, stack_pointer))
}

#[no_mangle]
extern "C" fn vanta_syscall_exec(frame: *const u64) -> *const UserContext {
    let pointer = unsafe { *frame.add(1) };
    let length = unsafe { *frame.add(2) };
    let Ok(path) = copy_from_user(pointer, length, false) else {
        return core::ptr::null();
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        return core::ptr::null();
    };
    let Ok(image) = crate::vfs::read_root(path) else {
        return core::ptr::null();
    };
    let Ok(process) = crate::process::load_elf(&image) else {
        return core::ptr::null();
    };
    crate::scheduler::exec_current(alloc::boxed::Box::new(process))
}

fn user_context(frame: *const u64, stack_pointer: u64) -> UserContext {
    unsafe {
        UserContext {
            return_value: 0,
            rbx: *frame.add(7),
            rbp: *frame.add(8),
            r12: *frame.add(9),
            r13: *frame.add(10),
            r14: *frame.add(11),
            r15: *frame.add(12),
            instruction_pointer: *frame.add(5),
            flags: *frame.add(6),
            stack_pointer,
        }
    }
}

#[no_mangle]
extern "C" fn vanta_syscall_exit(code: u64) -> *const UserContext {
    crate::scheduler::exit_current(code)
}
