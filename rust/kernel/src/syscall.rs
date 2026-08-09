//! Syscall ABI with GS-selected per-CPU entry state.

use core::arch::asm;
use core::arch::global_asm;

use alloc::vec::Vec;
use vanta_abi::{AbiInfo, SignalAction, Syscall};
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, SFMask, Star};
use x86_64::registers::rflags::RFlags;
use x86_64::VirtAddr;

use crate::paging::{self, AddressSpace};

pub const SYS_READ: u64 = Syscall::Read.number() as u64;
pub const SYS_WRITE: u64 = Syscall::Write.number() as u64;
pub const SYS_OPEN: u64 = Syscall::OpenAt.number() as u64;
pub const SYS_CLOSE: u64 = Syscall::Close.number() as u64;
pub const SYS_LSEEK: u64 = Syscall::LSeek.number() as u64;
pub const SYS_FSTAT: u64 = Syscall::FStat.number() as u64;
pub const SYS_GETDENTS: u64 = Syscall::GetDents.number() as u64;
pub const SYS_MKDIR: u64 = Syscall::MkDirAt.number() as u64;
pub const SYS_UNLINK: u64 = Syscall::UnlinkAt.number() as u64;
pub const SYS_RENAME: u64 = Syscall::RenameAt.number() as u64;
pub const SYS_YIELD: u64 = Syscall::Yield.number() as u64;
pub const SYS_DUP: u64 = Syscall::Dup3.number() as u64;
pub const SYS_PIPE: u64 = Syscall::Pipe2.number() as u64;
pub const SYS_GETPID: u64 = Syscall::GetPid.number() as u64;
pub const SYS_SOCKET: u64 = Syscall::Socket.number() as u64;
pub const SYS_CONNECT: u64 = Syscall::Connect.number() as u64;
pub const SYS_GETPPID: u64 = Syscall::GetPpid.number() as u64;
pub const SYS_EXEC: u64 = Syscall::ExecVe.number() as u64;
pub const SYS_EXIT: u64 = Syscall::Exit.number() as u64;
pub const SYS_WAITPID: u64 = Syscall::WaitPid.number() as u64;
pub const SYS_KILL: u64 = Syscall::Kill.number() as u64;
pub const SYS_SIGACTION: u64 = Syscall::SigAction.number() as u64;
pub const SYS_SPAWN: u64 = Syscall::SpawnVe.number() as u64;
pub const SYS_GET_ABI_INFO: u64 = Syscall::GetAbiInfo.number() as u64;
const SYSCALL_RETURN_EXIT: u64 = u64::MAX;
const SYSCALL_RETURN_YIELD: u64 = u64::MAX - 2;
const SYSCALL_RETURN_WAIT: u64 = u64::MAX - 3;
const SYSCALL_RETURN_EXEC: u64 = u64::MAX - 4;
const SYSCALL_RETURN_BLOCK: u64 = u64::MAX - 6;
const SYSCALL_ERROR: u64 = u64::MAX - 1;
pub(crate) const SYSCALL_WOULD_BLOCK: u64 = u64::MAX - 5;
const USER_ADDRESS_LIMIT: u64 = 0x0000_8000_0000_0000;
// RedoxFS transactions are stack-intensive, so descriptor syscalls require a
// real kernel stack rather than the former 8 KiB bootstrap-sized buffer.
const SYSCALL_STACK_SIZE: usize = 128 * 1024;
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
    native_abi: u64,
    block_descriptor: u64,
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
    native_abi: 0,
    block_descriptor: 0,
};

static mut CPU_LOCALS: [CpuLocal; MAX_CPUS] = [EMPTY_CPU_LOCAL; MAX_CPUS];

const SYSCALL_STACK_TOP_OFFSET: usize = core::mem::offset_of!(CpuLocal, syscall_stack_top);
const USER_RSP_OFFSET: usize = core::mem::offset_of!(CpuLocal, user_rsp);
const EXIT_CODE_OFFSET: usize = core::mem::offset_of!(CpuLocal, exit_code);
const NATIVE_ABI_OFFSET: usize = core::mem::offset_of!(CpuLocal, native_abi);

global_asm!(
    r#"
    .global vanta_syscall_entry
    .extern vanta_syscall_dispatch
    .extern vanta_syscall_yield
    .extern vanta_syscall_wait
    .extern vanta_syscall_exec
    .extern vanta_syscall_block
    .extern vanta_syscall_exit
vanta_syscall_entry:
    swapgs
    mov gs:[{user_rsp_offset}], rsp
    mov rsp, gs:[{syscall_stack_top_offset}]
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
    cmp rax, -6
    je vanta_syscall_block_path
    mov r11, [rsp + 48]
    mov rcx, [rsp + 40]
    cmp qword ptr gs:[{native_abi_offset}], 0
    je vanta_syscall_raw_return
    mov rdx, [rsp + 24]
    mov rsi, [rsp + 16]
    mov rdi, [rsp + 8]
vanta_syscall_raw_return:
    add rsp, 104
    mov rsp, gs:[{user_rsp_offset}]
    swapgs
    sysretq
vanta_syscall_yield_path:
    mov rdi, rsp
    mov rsi, gs:[{user_rsp_offset}]
    call vanta_syscall_yield
    jmp vanta_syscall_restore_context
vanta_syscall_wait_path:
    mov rdi, rsp
    mov rsi, gs:[{user_rsp_offset}]
    call vanta_syscall_wait
    jmp vanta_syscall_restore_context
vanta_syscall_exec_path:
    mov rdi, rsp
    call vanta_syscall_exec
    test rax, rax
    jz vanta_syscall_exec_error
    jmp vanta_syscall_restore_context
vanta_syscall_block_path:
    mov rdi, rsp
    mov rsi, gs:[{user_rsp_offset}]
    call vanta_syscall_block
    jmp vanta_syscall_restore_context
vanta_syscall_exec_error:
    mov r11, [rsp + 48]
    mov rcx, [rsp + 40]
    add rsp, 104
    mov rsp, gs:[{user_rsp_offset}]
    mov rax, -2
    swapgs
    sysretq
vanta_syscall_exit_path:
    mov rdi, gs:[{exit_code_offset}]
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
    swapgs
    sysretq
"#,
    syscall_stack_top_offset = const SYSCALL_STACK_TOP_OFFSET,
    user_rsp_offset = const USER_RSP_OFFSET,
    exit_code_offset = const EXIT_CODE_OFFSET,
    native_abi_offset = const NATIVE_ABI_OFFSET,
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
    let mut kernel_gs_base = x86_64::registers::model_specific::Msr::new(0xc000_0102);
    unsafe {
        gs_base.write(local as u64);
        kernel_gs_base.write(0);
    }
    true
}

pub fn current_cpu_index() -> usize {
    current_cpu_local().cpu_index
}

pub fn set_native_abi(enabled: bool) {
    current_cpu_local().native_abi = u64::from(enabled);
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
        SYS_OPEN => {
            if arg3 & 0x1f != 0 {
                open_native_user(arg1, arg2, arg3)
            } else {
                open_user(arg1, arg2)
            }
        }
        SYS_CLOSE => close_user(arg1),
        SYS_LSEEK => seek_user(arg1, arg2 as i64, arg3),
        SYS_FSTAT => fstat_user(arg1, arg2),
        SYS_GETDENTS => read_user(arg1, arg2, arg3),
        SYS_MKDIR => path_mutation_user(arg1, arg2, 0, 0),
        SYS_UNLINK => path_mutation_user(arg1, arg2, 1, 0),
        SYS_RENAME => path_mutation_user(arg1, arg2, arg3, _arg4),
        SYS_DUP => duplicate_legacy_user(arg1),
        SYS_PIPE => pipe_user(arg1, arg2),
        SYS_SOCKET => socket_user(arg1, arg2, arg3),
        SYS_CONNECT => connect_user(arg1, arg2, arg3),
        SYS_SPAWN => {
            if arg3 == 0 {
                spawn_legacy_user(arg1, arg2)
            } else {
                let native = spawn_native_user(arg1, arg2, arg3, _arg4);
                if native == SYSCALL_ERROR {
                    spawn_legacy_user(arg1, arg2)
                } else {
                    native
                }
            }
        }
        SYS_WAITPID => waitpid_user(arg1),
        SYS_KILL => kill_user(arg1, arg2),
        SYS_SIGACTION => sigaction_user(arg1, arg2, arg3),
        SYS_GET_ABI_INFO => abi_info_user(arg1, arg2),
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
    let Ok(bytes) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    if descriptor == 1 || descriptor == 2 {
        if crate::scheduler::write_current(descriptor, &bytes).is_ok() {
            return length;
        }
        for byte in bytes {
            crate::serial::_print(format_args!("{}", byte as char));
        }
        return length;
    }
    match crate::scheduler::write_current(descriptor, &bytes) {
        Ok(()) => length,
        Err(()) => SYSCALL_ERROR,
    }
}

fn open_user(pointer: u64, length: u64) -> u64 {
    let Ok(path) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        return SYSCALL_ERROR;
    };
    let credentials = crate::scheduler::current_credentials();
    let Ok(contents) = crate::vfs::read_root_as(path, &credentials) else {
        return SYSCALL_ERROR;
    };
    crate::scheduler::open_current(contents).unwrap_or(SYSCALL_ERROR)
}

fn open_native_user(pointer: u64, length: u64, flags: u64) -> u64 {
    let Ok(path_bytes) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path_bytes) else {
        return SYSCALL_ERROR;
    };
    let credentials = crate::scheduler::current_credentials();
    if let Ok(info) = crate::vfs::file_info_root_as(path, &credentials) {
        if info.is_directory {
            let Ok(entries) = crate::vfs::list_dir_root_as(path, &credentials) else {
                return SYSCALL_ERROR;
            };
            return crate::scheduler::open_directory_current(entries).unwrap_or(SYSCALL_ERROR);
        }
    }
    let writable = flags & 1 != 0;
    let create = flags & 2 != 0;
    let truncate = flags & 4 != 0;
    let append = flags & 8 != 0;
    if writable && !crate::scheduler::can_mutate_path(path) {
        return SYSCALL_ERROR;
    }
    let mut contents = match crate::vfs::read_root_as(path, &credentials) {
        Ok(contents) => contents,
        Err(_) if writable && create => Vec::new(),
        Err(_) => return SYSCALL_ERROR,
    };
    if writable && truncate {
        contents.clear();
        if crate::vfs::write_root_as(path, &contents, &credentials).is_err() {
            return SYSCALL_ERROR;
        }
    } else if writable
        && create
        && crate::vfs::file_info_root_as(path, &credentials).is_err()
        && crate::vfs::write_root_as(path, &contents, &credentials).is_err()
    {
        return SYSCALL_ERROR;
    }
    let descriptor = crate::scheduler::open_native_current(
        alloc::string::String::from(path),
        contents,
        writable,
        append,
    )
    .unwrap_or(SYSCALL_ERROR);
    descriptor
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
    if bytes.is_empty() && crate::scheduler::read_would_block(descriptor) {
        current_cpu_local().block_descriptor = descriptor;
        return SYSCALL_RETURN_BLOCK;
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

fn path_mutation_user(old_pointer: u64, old_length: u64, operation: u64, new_length: u64) -> u64 {
    let Ok(old_bytes) = copy_from_user(old_pointer, old_length, false) else {
        return SYSCALL_ERROR;
    };
    let Ok(old_path) = core::str::from_utf8(&old_bytes) else {
        return SYSCALL_ERROR;
    };
    if !crate::scheduler::can_mutate_path(old_path) {
        return SYSCALL_ERROR;
    }
    let credentials = crate::scheduler::current_credentials();
    let result = match operation {
        0 => crate::vfs::create_dir_root_as(old_path, &credentials),
        1 => crate::vfs::remove_root_as(old_path, &credentials),
        _ => {
            let Ok(new_bytes) = copy_from_user(operation, new_length, false) else {
                return SYSCALL_ERROR;
            };
            let Ok(new_path) = core::str::from_utf8(&new_bytes) else {
                return SYSCALL_ERROR;
            };
            if !crate::scheduler::can_mutate_path(new_path) {
                return SYSCALL_ERROR;
            }
            crate::vfs::rename_root_as(old_path, new_path, &credentials)
        }
    };
    match result {
        Ok(()) => 0,
        Err(_) => SYSCALL_ERROR,
    }
}

fn fstat_user(descriptor: u64, pointer: u64) -> u64 {
    let Ok((length, mode)) = crate::scheduler::stat_current(descriptor) else {
        return SYSCALL_ERROR;
    };
    let mut stat = [0_u8; 16];
    stat[..8].copy_from_slice(&length.to_ne_bytes());
    stat[8..16].copy_from_slice(&mode.to_ne_bytes());
    match copy_to_user(pointer, &stat) {
        Ok(()) => 0,
        Err(()) => SYSCALL_ERROR,
    }
}

fn abi_info_user(pointer: u64, length: u64) -> u64 {
    let size = core::mem::size_of::<AbiInfo>();
    if length < size as u64 {
        return SYSCALL_ERROR;
    }
    let info = AbiInfo::current();
    let bytes = unsafe {
        core::slice::from_raw_parts(
            core::ptr::addr_of!(info).cast::<u8>(),
            core::mem::size_of::<AbiInfo>(),
        )
    };
    copy_to_user(pointer, bytes)
        .map(|()| 0)
        .unwrap_or(SYSCALL_ERROR)
}

fn duplicate_legacy_user(descriptor: u64) -> u64 {
    crate::scheduler::duplicate_current(descriptor).unwrap_or(SYSCALL_ERROR)
}

fn pipe_user(pointer: u64, flags: u64) -> u64 {
    if flags != 0 {
        return SYSCALL_ERROR;
    }
    let Ok((reader, writer)) = crate::scheduler::open_pipe_current() else {
        return SYSCALL_ERROR;
    };
    let mut descriptors = [0_u8; 8];
    descriptors[..4].copy_from_slice(&(reader as u32).to_ne_bytes());
    descriptors[4..].copy_from_slice(&(writer as u32).to_ne_bytes());
    copy_to_user(pointer, &descriptors)
        .map(|()| 0)
        .unwrap_or(SYSCALL_ERROR)
}

fn socket_user(domain: u64, socket_type: u64, protocol: u64) -> u64 {
    if domain != 2 || socket_type != 1 || protocol != 0 {
        return SYSCALL_ERROR;
    }
    crate::scheduler::open_socket_current().unwrap_or(SYSCALL_ERROR)
}

fn connect_user(descriptor: u64, pointer: u64, length: u64) -> u64 {
    if length != 16 {
        return SYSCALL_ERROR;
    }
    let Ok(address) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    if u16::from_ne_bytes([address[0], address[1]]) != 2 {
        return SYSCALL_ERROR;
    }
    let port = u16::from_be_bytes([address[2], address[3]]);
    let remote_ip = [address[4], address[5], address[6], address[7]];
    crate::scheduler::connect_socket_current(descriptor, remote_ip, port)
        .map(|()| 0)
        .unwrap_or(SYSCALL_ERROR)
}

fn spawn_legacy_user(pointer: u64, length: u64) -> u64 {
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

fn spawn_native_user(pointer: u64, length: u64, stdio_pointer: u64, with_args: u64) -> u64 {
    let Ok(path) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        return SYSCALL_ERROR;
    };
    let stdio_length = match with_args {
        1 => 40,
        2 => 56,
        _ => 24,
    };
    let Ok(stdio) = copy_from_user(stdio_pointer, stdio_length, false) else {
        return SYSCALL_ERROR;
    };
    let Ok(image) = crate::vfs::read_root(path) else {
        return SYSCALL_ERROR;
    };
    let mut arguments = Vec::new();
    let mut environment = Vec::new();
    if with_args == 1 || with_args == 2 {
        let argv_pointer = u64::from_ne_bytes(stdio[24..32].try_into().unwrap());
        let argc = u64::from_ne_bytes(stdio[32..40].try_into().unwrap()).min(8);
        for index in 0..argc {
            let Ok(pointer_bytes) = copy_from_user(argv_pointer + index * 8, 8, false) else {
                return SYSCALL_ERROR;
            };
            let pointer = u64::from_ne_bytes(pointer_bytes.try_into().unwrap());
            let Ok(argument) = copy_cstring(pointer, 128) else {
                return SYSCALL_ERROR;
            };
            arguments.push(argument);
        }
        if with_args == 2 {
            let envp_pointer = u64::from_ne_bytes(stdio[40..48].try_into().unwrap());
            let envc = u64::from_ne_bytes(stdio[48..56].try_into().unwrap()).min(16);
            for index in 0..envc {
                let Ok(pointer_bytes) = copy_from_user(envp_pointer + index * 8, 8, false) else {
                    return SYSCALL_ERROR;
                };
                let pointer = u64::from_ne_bytes(pointer_bytes.try_into().unwrap());
                let Ok(value) = copy_cstring(pointer, 256) else {
                    return SYSCALL_ERROR;
                };
                environment.push(value);
            }
        }
    }
    let Ok(process) = (if with_args == 2 {
        let argument_references = arguments.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let environment_references = environment.iter().map(Vec::as_slice).collect::<Vec<_>>();
        crate::process::load_elf_with_args_and_env(
            &image,
            &argument_references,
            &environment_references,
        )
    } else if with_args == 1 {
        let references = arguments.iter().map(Vec::as_slice).collect::<Vec<_>>();
        crate::process::load_elf_with_args(&image, &references)
    } else {
        crate::process::load_elf(&image)
    }) else {
        return SYSCALL_ERROR;
    };
    let stdin = u64::from_ne_bytes(stdio[0..8].try_into().unwrap());
    let stdout = u64::from_ne_bytes(stdio[8..16].try_into().unwrap());
    let stderr = u64::from_ne_bytes(stdio[16..24].try_into().unwrap());
    crate::scheduler::spawn_with_stdio_current(
        alloc::boxed::Box::new(process),
        stdin,
        stdout,
        stderr,
    )
    .unwrap_or(SYSCALL_ERROR)
}

fn copy_cstring(pointer: u64, limit: u64) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::new();
    for offset in 0..limit {
        let byte = copy_from_user(pointer + offset, 1, false)?[0];
        if byte == 0 {
            return Ok(bytes);
        }
        bytes.push(byte);
    }
    Err(())
}

fn waitpid_user(pid: u64) -> u64 {
    match crate::scheduler::wait_child_current(pid) {
        Ok(Some(code)) => code,
        Ok(None) => SYSCALL_RETURN_WAIT,
        Err(()) => SYSCALL_ERROR,
    }
}

fn kill_user(pid: u64, signal: u64) -> u64 {
    if signal == 0 || signal > 31 {
        return SYSCALL_ERROR;
    }
    crate::scheduler::kill_process(pid, signal)
        .map(|_| 0)
        .unwrap_or(SYSCALL_ERROR)
}

fn sigaction_user(signal: u64, new_action: u64, old_action: u64) -> u64 {
    if signal == 0 || signal > 31 {
        return SYSCALL_ERROR;
    }
    let old = match crate::scheduler::signal_action(signal) {
        Some(action) => action,
        None => return SYSCALL_ERROR,
    };
    if old_action != 0 {
        let bytes = [old.handler.to_ne_bytes(), old.flags.to_ne_bytes()].concat();
        if copy_to_user(old_action, &bytes).is_err() {
            return SYSCALL_ERROR;
        }
    }
    if new_action != 0 {
        let Ok(bytes) = copy_from_user(new_action, 16, false) else {
            return SYSCALL_ERROR;
        };
        let action = SignalAction {
            handler: u64::from_ne_bytes(bytes[0..8].try_into().unwrap()),
            flags: u64::from_ne_bytes(bytes[8..16].try_into().unwrap()),
        };
        if action.handler != 0 && action.handler != 1 {
            return SYSCALL_ERROR;
        }
        if crate::scheduler::set_signal_action(signal, action).is_err() {
            return SYSCALL_ERROR;
        }
    }
    0
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
extern "C" fn vanta_syscall_block(frame: *const u64, stack_pointer: u64) -> *const UserContext {
    let descriptor = current_cpu_local().block_descriptor;
    current_cpu_local().block_descriptor = 0;
    crate::scheduler::block_pipe_current(descriptor, user_context(frame, stack_pointer))
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
