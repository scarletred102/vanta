//! Syscall ABI with GS-selected per-CPU entry state.

use core::arch::asm;
use core::arch::global_asm;

use alloc::vec::Vec;
use vanta_abi::{AbiInfo, Syscall};
use vanta_linuxd;
use x86_64::registers::model_specific::{Efer, EferFlags, LStar, Msr, SFMask, Star};
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
pub const SYS_IPC_PAIR: u64 = Syscall::IpcPair.number() as u64;
pub const SYS_IPC_SEND: u64 = Syscall::IpcSend.number() as u64;
pub const SYS_IPC_RECV: u64 = Syscall::IpcRecv.number() as u64;
pub const SYS_IPC_REVOKE: u64 = Syscall::IpcRevoke.number() as u64;
pub const SYS_BRK: u64 = Syscall::Brk.number() as u64;
pub const SYS_MMAP: u64 = Syscall::MMap.number() as u64;
pub const SYS_MUNMAP: u64 = Syscall::MUnmap.number() as u64;
pub const SYS_DISPLAY_INFO: u64 = Syscall::DisplayInfo.number() as u64;
pub const SYS_DISPLAY_BLIT: u64 = Syscall::DisplayBlit.number() as u64;
pub const SYS_DISPLAY_FLUSH: u64 = Syscall::DisplayFlush.number() as u64;
pub const SYS_INPUT_POLL: u64 = Syscall::InputPoll.number() as u64;
pub const SYS_AUDIO_PLAY: u64 = Syscall::AudioPlay.number() as u64;
const SYSCALL_RETURN_EXIT: u64 = u64::MAX;
const SYSCALL_RETURN_YIELD: u64 = u64::MAX - 2;
const SYSCALL_RETURN_WAIT: u64 = u64::MAX - 3;
const SYSCALL_RETURN_EXEC: u64 = u64::MAX - 4;
const SYSCALL_RETURN_BLOCK: u64 = u64::MAX - 6;
const SYSCALL_RETURN_FUTEX_WAIT: u64 = u64::MAX - 7;
const SYSCALL_RETURN_THREAD_EXIT: u64 = u64::MAX - 8;
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
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub instruction_pointer: u64,
    pub flags: u64,
    pub stack_pointer: u64,
}

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct CpuLocal {
    self_pointer: u64,
    _pad: u64,
    syscall_stack: [u8; SYSCALL_STACK_SIZE],
    syscall_stack_top: u64,
    user_rsp: u64,
    exit_code: u64,
    next_context: UserContext,
    cpu_index: usize,
    block_descriptor: u64,
    futex_uaddr: u64,
    futex_bitset: u32,
    _pad2: u32,
}

const EMPTY_CPU_LOCAL: CpuLocal = CpuLocal {
    self_pointer: 0,
    _pad: 0,
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
        rdi: 0,
        rsi: 0,
        rdx: 0,
        r8: 0,
        r9: 0,
        r10: 0,
        instruction_pointer: 0,
        flags: 0,
        stack_pointer: 0,
    },
    cpu_index: 0,
    block_descriptor: 0,
    futex_uaddr: 0,
    futex_bitset: 0,
    _pad2: 0,
};

static mut CPU_LOCALS: [CpuLocal; MAX_CPUS] = [EMPTY_CPU_LOCAL; MAX_CPUS];

const SYSCALL_STACK_TOP_OFFSET: usize = core::mem::offset_of!(CpuLocal, syscall_stack_top);
const USER_RSP_OFFSET: usize = core::mem::offset_of!(CpuLocal, user_rsp);
const EXIT_CODE_OFFSET: usize = core::mem::offset_of!(CpuLocal, exit_code);

global_asm!(
    r#"
    .global vanta_syscall_entry
    .extern vanta_syscall_dispatch
    .extern vanta_syscall_yield
    .extern vanta_syscall_wait
    .extern vanta_syscall_exec
    .extern vanta_syscall_block
    .extern vanta_syscall_futex_wait
    .extern vanta_syscall_exit
    .extern vanta_syscall_thread_exit
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
    push r9
    push r8
    push rdx
    push rsi
    push rdi
    push rax
    mov rdi, [rsp]
    mov rsi, [rsp + 8]
    mov rdx, [rsp + 16]
    mov rcx, [rsp + 24]
    mov r8, [rsp + 48]
    mov r9, [rsp + 32]
    push qword ptr [rsp + 40]
    call vanta_syscall_dispatch
    add rsp, 8
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
    cmp rax, -7
    je vanta_syscall_futex_wait_path
    cmp rax, -9
    je vanta_syscall_thread_exit_path
vanta_syscall_raw_return:
    mov [rsp], rax
    pop rax
    pop rdi
    pop rsi
    pop rdx
    pop r8
    pop r9
    pop r10
    pop rcx
    pop r11
    pop rbx
    pop rbp
    pop r12
    pop r13
    pop r14
    pop r15
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
vanta_syscall_futex_wait_path:
    mov rdi, rsp
    mov rsi, gs:[{user_rsp_offset}]
    call vanta_syscall_futex_wait
    jmp vanta_syscall_restore_context
vanta_syscall_exec_error:
    mov r11, [rsp + 64]
    mov rcx, [rsp + 56]
    add rsp, 120
    mov rsp, gs:[{user_rsp_offset}]
    mov rax, -2
    swapgs
    sysretq
vanta_syscall_exit_path:
    mov rdi, gs:[{exit_code_offset}]
    call vanta_syscall_exit
    jmp vanta_syscall_restore_context
vanta_syscall_thread_exit_path:
    mov rdi, gs:[{exit_code_offset}]
    call vanta_syscall_thread_exit
    jmp vanta_syscall_restore_context
vanta_syscall_restore_context:
    mov r10, rax
    mov rbx, [r10 + 8]
    mov rbp, [r10 + 16]
    mov r12, [r10 + 24]
    mov r13, [r10 + 32]
    mov r14, [r10 + 40]
    mov r15, [r10 + 48]
    mov rdi, [r10 + 56]
    mov rsi, [r10 + 64]
    mov rdx, [r10 + 72]
    mov r8,  [r10 + 80]
    mov r9,  [r10 + 88]
    mov rcx, [r10 + 104]
    mov r11, [r10 + 112]
    mov rsp, [r10 + 120]
    mov rax, [r10]
    mov r10, [r10 + 96]
    swapgs
    sysretq
"#,
    syscall_stack_top_offset = const SYSCALL_STACK_TOP_OFFSET,
    user_rsp_offset = const USER_RSP_OFFSET,
    exit_code_offset = const EXIT_CODE_OFFSET,
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
        (*local)._pad = 0;
        (*local).syscall_stack_top = (core::ptr::addr_of!((*local).syscall_stack)
            .cast::<u8>()
            .add(SYSCALL_STACK_SIZE) as u64)
            & !15;
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

#[no_mangle]
extern "C" fn vanta_syscall_dispatch(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) -> u64 {
    let linux_personality = crate::scheduler::current_personality()
        == crate::process::ProcessPersonality::LinuxX86_64Static;
    let result = if linux_personality {
        dispatch_linux(number, arg1, arg2, arg3, arg4, arg5, arg6)
    } else {
        dispatch_native(number, arg1, arg2, arg3, arg4, arg5, arg6)
    };

    if result != SYSCALL_RETURN_EXIT
        && result != SYSCALL_RETURN_THREAD_EXIT
        && result != SYSCALL_RETURN_YIELD
        && result != SYSCALL_RETURN_WAIT
        && result != SYSCALL_RETURN_EXEC
        && result != SYSCALL_RETURN_BLOCK
        && result != SYSCALL_RETURN_FUTEX_WAIT
    {
        let frame_ptr = (current_cpu_local().syscall_stack_top - 120) as *mut u64;
        let user_rsp = &mut current_cpu_local().user_rsp;
        unsafe {
            *frame_ptr.add(0) = result;
        }
        if let Some((signo, action)) = crate::scheduler::check_pending_signal_to_deliver() {
            if action.sa_handler > 1 {
                let blocked = crate::scheduler::current_blocked_mask();
                if inject_signal_frame(signo, action, frame_ptr, user_rsp, blocked).is_ok() {
                    let mut new_mask = blocked | action.sa_mask;
                    if action.sa_flags & vanta_linuxd::SA_NODEFER == 0 {
                        new_mask |= 1 << (signo - 1);
                    }
                    crate::scheduler::set_current_blocked_mask(new_mask);
                    if action.sa_flags & vanta_linuxd::SA_RESETHAND != 0 {
                        crate::scheduler::reset_signal_action(signo);
                    }
                    return unsafe { *frame_ptr.add(0) };
                }
            } else if action.sa_handler == 0 {
                let default_act = crate::scheduler::default_signal_action(signo);
                if default_act == crate::scheduler::SignalDefaultAction::Terminate
                    || default_act == crate::scheduler::SignalDefaultAction::CoreDump
                {
                    current_cpu_local().exit_code = 128 + signo;
                    return SYSCALL_RETURN_EXIT;
                }
            }
        }
    }

    result
}

fn dispatch_linux(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) -> u64 {
    crate::serial_println!(
        "[linux-syscall] nr={} a1={:#x} a2={:#x} a3={:#x} a4={:#x} a5={:#x} a6={:#x}",
        number,
        arg1,
        arg2,
        arg3,
        arg4,
        arg5,
        arg6
    );
    let request = vanta_linuxd::LinuxSyscallRequest {
        number,
        args: [arg1, arg2, arg3, arg4, arg5, arg6],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    match vanta_linuxd::broker(request) {
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::OpenAt,
            ..
        } => {
            if number == 2 {
                linux_openat_user(0, arg1, arg2)
            } else {
                linux_openat_user(arg1, arg2, arg3)
            }
        }
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::Pipe2,
            ..
        } => {
            if number == 22 {
                pipe_user(arg1, 0)
            } else {
                pipe_user(arg1, arg2)
            }
        }
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::FStat,
            ..
        } => {
            if number == 4 || number == 6 {
                linux_stat_user(arg1, arg2)
            } else if number == 262 {
                if arg2 != 0 {
                    linux_stat_user(arg2, arg3)
                } else {
                    linux_fstat_user(arg1, arg3)
                }
            } else {
                linux_fstat_user(arg1, arg2)
            }
        }
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::GetDents,
            ..
        } => {
            if number == 217 || number == 78 {
                linux_getdents64_user(arg1, arg2, arg3)
            } else {
                read_user(arg1, arg2, arg3)
            }
        }
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::Dup3,
            ..
        } => {
            if number == 33 || number == 292 {
                crate::scheduler::duplicate_to_current(arg1, arg2).unwrap_or(SYSCALL_ERROR)
            } else {
                duplicate_legacy_user(arg1)
            }
        }
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::Brk,
            ..
        } => crate::scheduler::brk_current(arg1),
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::MMap,
            ..
        } => linux_mmap_user(arg1, arg2, arg3, arg4, arg5, arg6),
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::MUnmap,
            ..
        } => crate::scheduler::munmap_current(arg1, arg2).map(|()| 0).unwrap_or(SYSCALL_ERROR),
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::SigAction,
            ..
        } => linux_rt_sigaction_user(arg1, arg2, arg3, arg4),
        vanta_linuxd::BrokerDecision::Native {
            syscall: vanta_abi::Syscall::Kill,
            ..
        } => kill_user(arg1, arg2),
        vanta_linuxd::BrokerDecision::Native { syscall, args } => {
            dispatch_native(syscall.number() as u64, args[0], args[1], args[2], args[3], 0, 0)
        }
        vanta_linuxd::BrokerDecision::ProcessPrimitive { operation } => match operation {
            vanta_linuxd::LinuxOp::GetPid => crate::scheduler::current_pid(),
            vanta_linuxd::LinuxOp::GetTid => crate::scheduler::current_tid(),
            vanta_linuxd::LinuxOp::GetPPid => crate::scheduler::current_parent_pid(),
            vanta_linuxd::LinuxOp::GetUid | vanta_linuxd::LinuxOp::GetEUid => {
                crate::scheduler::current_credentials().uid as u64
            }
            vanta_linuxd::LinuxOp::GetGid | vanta_linuxd::LinuxOp::GetEGid => {
                crate::scheduler::current_credentials().gid as u64
            }
            vanta_linuxd::LinuxOp::SetPGid | vanta_linuxd::LinuxOp::GetPGrp => 0,
            vanta_linuxd::LinuxOp::Clone => {
                linux_clone_user(arg1, arg2, arg3, arg4, arg5)
            }
            vanta_linuxd::LinuxOp::Clone3 => {
                linux_clone3_user(arg1, arg2)
            }
            vanta_linuxd::LinuxOp::Futex => {
                linux_futex_user(arg1, arg2, arg3, arg4, arg5, arg6)
            }
            vanta_linuxd::LinuxOp::Exit => {
                current_cpu_local().exit_code = arg1;
                SYSCALL_RETURN_THREAD_EXIT
            }
            vanta_linuxd::LinuxOp::ExitGroup => {
                current_cpu_local().exit_code = arg1;
                SYSCALL_RETURN_EXIT
            }
            vanta_linuxd::LinuxOp::ExecVe => SYSCALL_RETURN_EXEC,
            vanta_linuxd::LinuxOp::Wait4 => linux_wait4_user(arg1, arg2, arg3),
            vanta_linuxd::LinuxOp::ArchPrctl => linux_arch_prctl_user(arg1, arg2),
            vanta_linuxd::LinuxOp::SetTidAddress => {
                crate::scheduler::set_current_clear_child_tid(arg1)
            }
            vanta_linuxd::LinuxOp::MProtect => linux_mprotect_user(arg1, arg2, arg3),
            vanta_linuxd::LinuxOp::RtSigAction => {
                linux_rt_sigaction_user(arg1, arg2, arg3, arg4)
            }
            vanta_linuxd::LinuxOp::RtSigProcMask => {
                linux_rt_sigprocmask_user(arg1, arg2, arg3, arg4)
            }
            vanta_linuxd::LinuxOp::RtSigReturn => {
                let frame_ptr = (current_cpu_local().syscall_stack_top - 120) as *mut u64;
                let user_rsp = &mut current_cpu_local().user_rsp;
                linux_rt_sigreturn_user(frame_ptr, user_rsp)
            }
            vanta_linuxd::LinuxOp::Kill => kill_user(arg1, arg2),
            vanta_linuxd::LinuxOp::TKill => {
                if crate::scheduler::kill_thread(arg1, arg2).is_ok() {
                    0
                } else {
                    SYSCALL_ERROR
                }
            }
            vanta_linuxd::LinuxOp::TgKill => {
                if crate::scheduler::kill_thread(arg2, arg3).is_ok() {
                    0
                } else {
                    SYSCALL_ERROR
                }
            }
            vanta_linuxd::LinuxOp::SetRobustList
            | vanta_linuxd::LinuxOp::Rseq
            | vanta_linuxd::LinuxOp::SigAltStack => 0,
            vanta_linuxd::LinuxOp::GetRandom => {
                let count = arg2.min(256);
                let bytes = [0x5au8; 256];
                if copy_to_user(arg1, &bytes[..count as usize]).is_err() {
                    SYSCALL_ERROR
                } else {
                    count
                }
            }
            vanta_linuxd::LinuxOp::SchedGetAffinity => SYSCALL_ERROR,
            vanta_linuxd::LinuxOp::Writev => linux_writev_user(arg1, arg2, arg3),
            vanta_linuxd::LinuxOp::Readv => linux_readv_user(arg1, arg2, arg3),
            vanta_linuxd::LinuxOp::Access | vanta_linuxd::LinuxOp::FAccessAt => {
                if operation == vanta_linuxd::LinuxOp::FAccessAt && arg2 != 0 {
                    linux_access_user(arg1, arg2, arg3)
                } else {
                    linux_access_user(0, arg1, arg2)
                }
            }
            vanta_linuxd::LinuxOp::Uname => linux_uname_user(arg1),
            vanta_linuxd::LinuxOp::GetCwd => linux_getcwd_user(arg1, arg2),
            vanta_linuxd::LinuxOp::ChDir | vanta_linuxd::LinuxOp::FChDir => 0,
            vanta_linuxd::LinuxOp::ReadLink | vanta_linuxd::LinuxOp::ReadLinkAt => SYSCALL_ERROR,
            vanta_linuxd::LinuxOp::ClockGetTime => linux_clock_gettime_user(arg1, arg2),
            vanta_linuxd::LinuxOp::GetTimeOfDay => linux_gettimeofday_user(arg1, arg2),
            vanta_linuxd::LinuxOp::Fcntl => linux_fcntl_user(arg1, arg2, arg3),
            vanta_linuxd::LinuxOp::Ioctl => linux_ioctl_user(arg1, arg2, arg3),
            vanta_linuxd::LinuxOp::Nanosleep => 0,
            vanta_linuxd::LinuxOp::SendTo | vanta_linuxd::LinuxOp::SendMsg => {
                write_user(arg1, arg2, arg3)
            }
            vanta_linuxd::LinuxOp::RecvFrom | vanta_linuxd::LinuxOp::RecvMsg => {
                read_user(arg1, arg2, arg3)
            }
            vanta_linuxd::LinuxOp::Bind
            | vanta_linuxd::LinuxOp::Listen
            | vanta_linuxd::LinuxOp::Accept
            | vanta_linuxd::LinuxOp::Accept4 => 0,
            vanta_linuxd::LinuxOp::GetSockName
            | vanta_linuxd::LinuxOp::GetPeerName
            | vanta_linuxd::LinuxOp::SetSockOpt
            | vanta_linuxd::LinuxOp::GetSockOpt => 0,
            vanta_linuxd::LinuxOp::Fork | vanta_linuxd::LinuxOp::VFork => {
                linux_clone_user(0, 0, 0, 0, 0)
            }
            vanta_linuxd::LinuxOp::EPollCreate | vanta_linuxd::LinuxOp::EPollCreate1 => {
                crate::scheduler::epoll_create1_current(arg1 as u32).unwrap_or(SYSCALL_ERROR)
            }
            vanta_linuxd::LinuxOp::EPollCtl => {
                linux_epoll_ctl_user(arg1, arg2 as u32, arg3, arg4)
            }
            vanta_linuxd::LinuxOp::EPollWait | vanta_linuxd::LinuxOp::EPollPWait => {
                linux_epoll_wait_user(arg1, arg2, arg3 as usize, arg4)
            }
            vanta_linuxd::LinuxOp::EventFd | vanta_linuxd::LinuxOp::EventFd2 => {
                crate::scheduler::eventfd_current(arg1, arg2 as u32).unwrap_or(SYSCALL_ERROR)
            }
            vanta_linuxd::LinuxOp::Poll | vanta_linuxd::LinuxOp::PPoll => {
                linux_poll_user(arg1, arg2 as usize, arg3)
            }
            vanta_linuxd::LinuxOp::Select | vanta_linuxd::LinuxOp::PSelect6 => 0,
            _ => SYSCALL_ERROR,
        },
        vanta_linuxd::BrokerDecision::Unsupported { number } => {
            crate::serial_println!("[linuxd] unsupported syscall number={}", number);
            if number == 9999 {
                SYSCALL_ERROR
            } else {
                current_cpu_local().exit_code = 127;
                SYSCALL_RETURN_EXIT
            }
        }
    }
}

fn linux_mprotect_user(addr: u64, length: u64, prot: u64) -> u64 {
    if addr & (paging::PAGE_SIZE - 1) != 0 {
        return SYSCALL_ERROR;
    }
    if length == 0 {
        return 0;
    }
    let Some(aligned_len) = (length.checked_add(paging::PAGE_SIZE - 1)).map(|l| l & !(paging::PAGE_SIZE - 1)) else {
        return SYSCALL_ERROR;
    };
    if addr.checked_add(aligned_len).is_none() || addr + aligned_len >= USER_ADDRESS_LIMIT {
        return SYSCALL_ERROR;
    }
    let mut flags = paging::MAP_USER;
    if prot & 2 != 0 {
        flags |= paging::MAP_WRITABLE;
    }
    if prot & 4 == 0 {
        flags |= paging::MAP_NO_EXECUTE;
    }
    let pages = (aligned_len / paging::PAGE_SIZE) as usize;
    if paging::protect(paging::current_address_space(), addr, pages, flags).is_ok() {
        0
    } else {
        SYSCALL_ERROR
    }
}

fn linux_mmap_user(
    addr: u64,
    length: u64,
    prot: u64,
    flags: u64,
    fd: u64,
    offset: u64,
) -> u64 {
    if length == 0 {
        return SYSCALL_ERROR;
    }
    let is_anonymous = flags & 0x20 != 0 || fd == u64::MAX || fd as i64 == -1;
    let alloc_prot = if !is_anonymous { prot | 2 } else { prot };
    let mapped = crate::scheduler::mmap_current(addr, length, alloc_prot, flags);
    let Ok(base_vaddr) = mapped else {
        return SYSCALL_ERROR;
    };
    if !is_anonymous {
        let mut total_read = 0u64;
        let _ = crate::scheduler::seek_current(fd, offset as i64, 0);
        while total_read < length {
            let chunk_len = (length - total_read).min(256);
            let Ok(bytes) = crate::scheduler::read_current(fd, chunk_len as usize) else {
                break;
            };
            if bytes.is_empty() {
                break;
            }
            let count = bytes.len() as u64;
            if copy_to_user(base_vaddr + total_read, &bytes).is_err() {
                break;
            }
            total_read += count;
            if count < chunk_len {
                break;
            }
        }
        if prot & 2 == 0 {
            let aligned_len = (length.checked_add(paging::PAGE_SIZE - 1).unwrap_or(length)) & !(paging::PAGE_SIZE - 1);
            let pages = (aligned_len / paging::PAGE_SIZE) as usize;
            let mut final_flags = paging::MAP_USER;
            if prot & 4 == 0 {
                final_flags |= paging::MAP_NO_EXECUTE;
            }
            let _ = paging::protect(paging::current_address_space(), base_vaddr, pages, final_flags);
        }
    }
    base_vaddr
}

fn generate_procfs_content(path: &str) -> Option<alloc::vec::Vec<u8>> {
    use alloc::format;
    if path == "/proc/cpuinfo" {
        Some(alloc::vec::Vec::from(
            "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Vanta Virtual CPU\ncpu MHz\t\t: 3000.000\n\n"
        ))
    } else if path == "/proc/meminfo" {
        Some(alloc::vec::Vec::from(
            "MemTotal:       2097152 kB\nMemFree:        1843200 kB\nMemAvailable:   1843200 kB\nBuffers:           1024 kB\nCached:           16384 kB\n"
        ))
    } else if path == "/proc/version" {
        Some(alloc::vec::Vec::from(
            "Linux version 6.1.0-vanta (vanta@build) (gcc 12.2.0) #1 SMP PREEMPT\n"
        ))
    } else if path == "/proc/uptime" {
        Some(alloc::vec::Vec::from("10.00 10.00\n"))
    } else if path.ends_with("/status") {
        let pid = crate::scheduler::current_pid();
        let ppid = crate::scheduler::current_parent_pid();
        let s = format!("Name:\tvanta-app\nState:\tR (running)\nTgid:\t{}\nPid:\t{}\nPPid:\t{}\nThreads:\t1\n", pid, pid, ppid);
        Some(s.into_bytes())
    } else if path.ends_with("/cmdline") {
        Some(alloc::vec::Vec::from("vanta-app\0"))
    } else if path.ends_with("/maps") {
        Some(alloc::vec::Vec::from(
            "00400000-00450000 r-xp 00000000 00:00 0 [text]\n700000000000-700000020000 rw-p 00000000 00:00 0 [heap]\n7fffffff0000-800000000000 rw-p 00000000 00:00 0 [stack]\n"
        ))
    } else if path.starts_with("/sys/class/net/") {
        Some(alloc::vec::Vec::from("up\n"))
    } else {
        None
    }
}

fn linux_openat_user(_directory_fd: u64, path_pointer: u64, flags: u64) -> u64 {
    let Ok(path) = copy_cstring(path_pointer, 256) else {
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        return SYSCALL_ERROR;
    };
    if path.starts_with("/proc/") || path.starts_with("/sys/") {
        if let Some(contents) = generate_procfs_content(path) {
            return crate::scheduler::open_native_current(
                alloc::string::String::from(path),
                contents,
                false,
                false,
            )
            .unwrap_or(SYSCALL_ERROR);
        }
    }
    if let Ok(entries) = crate::vfs::list_dir_root(path) {
        return crate::scheduler::open_directory_current(entries).unwrap_or(SYSCALL_ERROR);
    }
    let Ok(contents) = crate::vfs::read_root(path) else {
        return SYSCALL_ERROR;
    };
    let writable = flags & 3 != 0;
    crate::scheduler::open_native_current(
        alloc::string::String::from(path),
        contents,
        writable,
        flags & 1024 != 0,
    )
    .unwrap_or(SYSCALL_ERROR)
}

fn linux_fstat_user(descriptor: u64, pointer: u64) -> u64 {
    let Ok(stat) = crate::scheduler::stat_linux_current(descriptor) else {
        return SYSCALL_ERROR;
    };
    if copy_to_user(pointer, &stat).is_err() {
        return SYSCALL_ERROR;
    }
    0
}

fn linux_stat_user(path_pointer: u64, pointer: u64) -> u64 {
    let Ok(path) = copy_cstring(path_pointer, 256) else {
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        return SYSCALL_ERROR;
    };
    if path.starts_with("/proc/") || path.starts_with("/sys/") {
        let size = generate_procfs_content(path).map(|c| c.len() as i64).unwrap_or(64);
        let mut stat = [0u8; 144];
        let mode = 0o100444u32;
        let dev = 1u64;
        let ino = 1u64;
        let nlink = 1u64;
        let uid = 0u32;
        let gid = 0u32;
        let blksize = 4096i64;
        let blocks = (size + 511) / 512;
        stat[0..8].copy_from_slice(&dev.to_ne_bytes());
        stat[8..16].copy_from_slice(&ino.to_ne_bytes());
        stat[16..24].copy_from_slice(&nlink.to_ne_bytes());
        stat[24..28].copy_from_slice(&mode.to_ne_bytes());
        stat[28..32].copy_from_slice(&uid.to_ne_bytes());
        stat[32..36].copy_from_slice(&gid.to_ne_bytes());
        stat[48..56].copy_from_slice(&size.to_ne_bytes());
        stat[56..64].copy_from_slice(&blksize.to_ne_bytes());
        stat[64..72].copy_from_slice(&blocks.to_ne_bytes());
        if copy_to_user(pointer, &stat).is_err() {
            return SYSCALL_ERROR;
        }
        return 0;
    }
    let credentials = crate::scheduler::current_credentials();
    let Ok(info) = crate::vfs::file_info_root_as(path, &credentials) else {
        return SYSCALL_ERROR;
    };
    let mut stat = [0u8; 144];
    let mode = if info.is_directory {
        0o040755u32
    } else {
        0o100644u32
    };
    let size = info.length as i64;
    let dev = 1u64;
    let ino = 1u64;
    let nlink = 1u64;
    let uid = 1000u32;
    let gid = 1000u32;
    let blksize = 4096i64;
    let blocks = (size + 511) / 512;
    stat[0..8].copy_from_slice(&dev.to_ne_bytes());
    stat[8..16].copy_from_slice(&ino.to_ne_bytes());
    stat[16..24].copy_from_slice(&nlink.to_ne_bytes());
    stat[24..28].copy_from_slice(&mode.to_ne_bytes());
    stat[28..32].copy_from_slice(&uid.to_ne_bytes());
    stat[32..36].copy_from_slice(&gid.to_ne_bytes());
    stat[48..56].copy_from_slice(&size.to_ne_bytes());
    stat[56..64].copy_from_slice(&blksize.to_ne_bytes());
    stat[64..72].copy_from_slice(&blocks.to_ne_bytes());
    if copy_to_user(pointer, &stat).is_err() {
        return SYSCALL_ERROR;
    }
    0
}

fn linux_getdents64_user(descriptor: u64, pointer: u64, length: u64) -> u64 {
    if length > 4096 {
        return SYSCALL_ERROR;
    }
    let Ok(bytes) = crate::scheduler::read_dir_linux_current(descriptor, length as usize) else {
        return SYSCALL_ERROR;
    };
    if bytes.is_empty() {
        return 0;
    }
    if copy_to_user(pointer, &bytes).is_err() {
        return SYSCALL_ERROR;
    }
    bytes.len() as u64
}

fn linux_uname_user(pointer: u64) -> u64 {
    let mut uts = [0u8; 390];
    let sysname = b"Linux\0";
    let nodename = b"vanta\0";
    let release = b"6.1.0-vanta\0";
    let version = b"#1 SMP\0";
    let machine = b"x86_64\0";
    uts[0..sysname.len()].copy_from_slice(sysname);
    uts[65..65 + nodename.len()].copy_from_slice(nodename);
    uts[130..130 + release.len()].copy_from_slice(release);
    uts[195..195 + version.len()].copy_from_slice(version);
    uts[260..260 + machine.len()].copy_from_slice(machine);
    if copy_to_user(pointer, &uts).is_err() {
        return SYSCALL_ERROR;
    }
    0
}

fn linux_getcwd_user(pointer: u64, size: u64) -> u64 {
    let cwd = b"/home/vanta\0";
    if size < cwd.len() as u64 {
        return SYSCALL_ERROR;
    }
    if copy_to_user(pointer, cwd).is_err() {
        return SYSCALL_ERROR;
    }
    pointer
}

fn linux_clock_gettime_user(_clock_id: u64, pointer: u64) -> u64 {
    let mut timespec = [0u8; 16];
    let sec: i64 = 1700000000;
    let nsec: i64 = 0;
    timespec[0..8].copy_from_slice(&sec.to_ne_bytes());
    timespec[8..16].copy_from_slice(&nsec.to_ne_bytes());
    if copy_to_user(pointer, &timespec).is_err() {
        return SYSCALL_ERROR;
    }
    0
}

fn linux_gettimeofday_user(tv_pointer: u64, _tz_pointer: u64) -> u64 {
    if tv_pointer != 0 {
        let mut timeval = [0u8; 16];
        let sec: i64 = 1700000000;
        let usec: i64 = 0;
        timeval[0..8].copy_from_slice(&sec.to_ne_bytes());
        timeval[8..16].copy_from_slice(&usec.to_ne_bytes());
        if copy_to_user(tv_pointer, &timeval).is_err() {
            return SYSCALL_ERROR;
        }
    }
    0
}

fn linux_writev_user(descriptor: u64, iov_pointer: u64, iovcnt: u64) -> u64 {
    if iovcnt > 16 {
        return SYSCALL_ERROR;
    }
    let mut total = 0u64;
    for index in 0..iovcnt {
        let Ok(entry) = copy_from_user(iov_pointer + index * 16, 16, false) else {
            return SYSCALL_ERROR;
        };
        let base = u64::from_ne_bytes(entry[0..8].try_into().unwrap());
        let len = u64::from_ne_bytes(entry[8..16].try_into().unwrap());
        if len == 0 {
            continue;
        }
        let written = write_user(descriptor, base, len);
        if written == SYSCALL_ERROR {
            return if total > 0 { total } else { SYSCALL_ERROR };
        }
        total += written;
    }
    total
}

fn linux_readv_user(descriptor: u64, iov_pointer: u64, iovcnt: u64) -> u64 {
    if iovcnt > 16 {
        return SYSCALL_ERROR;
    }
    let mut total = 0u64;
    for index in 0..iovcnt {
        let Ok(entry) = copy_from_user(iov_pointer + index * 16, 16, false) else {
            return SYSCALL_ERROR;
        };
        let base = u64::from_ne_bytes(entry[0..8].try_into().unwrap());
        let len = u64::from_ne_bytes(entry[8..16].try_into().unwrap());
        if len == 0 {
            continue;
        }
        let read_count = read_user(descriptor, base, len);
        if read_count == SYSCALL_ERROR {
            return if total > 0 { total } else { SYSCALL_ERROR };
        }
        total += read_count;
        if read_count < len {
            break;
        }
    }
    total
}

fn linux_access_user(_dirfd: u64, path_pointer: u64, _mode: u64) -> u64 {
    let Ok(path) = copy_cstring(path_pointer, 256) else {
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        return SYSCALL_ERROR;
    };
    let credentials = crate::scheduler::current_credentials();
    if crate::vfs::file_info_root_as(path, &credentials).is_ok()
        || crate::vfs::list_dir_root_as(path, &credentials).is_ok()
    {
        0
    } else {
        SYSCALL_ERROR
    }
}

fn linux_wait4_user(pid: u64, status_pointer: u64, _options: u64) -> u64 {
    let target = if pid == u64::MAX || pid as i64 == -1 || pid == 0 {
        u64::MAX
    } else {
        pid
    };
    match crate::scheduler::wait_child_current(target) {
        Ok(Some((child_tgid, exit_code))) => {
            if status_pointer != 0 {
                let status: i32 = ((exit_code as i32) & 0xff) << 8;
                let _ = copy_to_user(status_pointer, &status.to_ne_bytes());
            }
            child_tgid
        }
        Ok(None) => SYSCALL_RETURN_WAIT,
        Err(()) => SYSCALL_ERROR,
    }
}

fn clone_interrupt_context(frame: *const u64, child_sp: u64) -> crate::scheduler::InterruptContext {
    let (code_segment, stack_segment) = crate::gdt::user_interrupt_selectors();
    unsafe {
        crate::scheduler::InterruptContext::new(
            *frame.add(14),        // r15 (rsp+112)
            *frame.add(13),        // r14 (rsp+104)
            *frame.add(12),        // r13 (rsp+96)
            *frame.add(11),        // r12 (rsp+88)
            *frame.add(8),         // r11 (rsp+64)
            *frame.add(6),         // r10 (rsp+48)
            *frame.add(5),         // r9 (rsp+40)
            *frame.add(4),         // r8 (rsp+32)
            *frame.add(1),         // rdi (rsp+8)
            *frame.add(2),         // rsi (rsp+16)
            *frame.add(10),        // rbp (rsp+80)
            *frame.add(3),         // rdx (rsp+24)
            *frame.add(7),         // rcx (rsp+56)
            *frame.add(9),         // rbx (rsp+72)
            0,                     // rax
            *frame.add(7),         // instruction_pointer (rcx)
            code_segment,
            *frame.add(8) | 0x202, // flags (r11)
            child_sp,
            stack_segment,
        )
    }
}

fn linux_clone_user(
    flags: u64,
    child_stack: u64,
    parent_tidptr: u64,
    child_tidptr: u64,
    tls: u64,
) -> u64 {
    let frame_ptr = (current_cpu_local().syscall_stack_top - 120) as *const u64;
    let stack_pointer = current_cpu_local().user_rsp;
    let context = user_context(frame_ptr, stack_pointer);
    let child_sp = if child_stack != 0 {
        child_stack
    } else {
        stack_pointer
    };
    let interrupt_context = clone_interrupt_context(frame_ptr, child_sp);
    crate::scheduler::clone_task_current(
        flags,
        child_stack,
        parent_tidptr,
        child_tidptr,
        tls,
        context,
        interrupt_context,
    )
    .unwrap_or(SYSCALL_ERROR)
}

fn linux_clone3_user(cl_args_ptr: u64, size: u64) -> u64 {
    if cl_args_ptr == 0 || size < 64 || cl_args_ptr >= USER_ADDRESS_LIMIT {
        return SYSCALL_ERROR;
    }
    let mut args = vanta_linuxd::clone_args::default();
    let copy_size = (size as usize).min(core::mem::size_of::<vanta_linuxd::clone_args>());
    let slice = unsafe {
        core::slice::from_raw_parts_mut(
            &mut args as *mut vanta_linuxd::clone_args as *mut u8,
            copy_size,
        )
    };
    if copy_from_user_into(cl_args_ptr, slice).is_err() {
        return SYSCALL_ERROR;
    }
    linux_clone_user(
        args.flags,
        args.stack.saturating_add(args.stack_size),
        args.parent_tid,
        args.child_tid,
        args.tls,
    )
}

fn linux_futex_user(
    uaddr: u64,
    op: u64,
    val: u64,
    _timeout: u64,
    _uaddr2: u64,
    val3: u64,
) -> u64 {
    if uaddr == 0 || uaddr >= USER_ADDRESS_LIMIT {
        return SYSCALL_ERROR;
    }
    let cmd = (op as u32) & vanta_linuxd::FUTEX_CMD_MASK;
    let bitset = if cmd == vanta_linuxd::FUTEX_WAIT_BITSET || cmd == vanta_linuxd::FUTEX_WAKE_BITSET {
        val3 as u32
    } else {
        vanta_linuxd::FUTEX_BITSET_MATCH_ANY
    };

    match cmd {
        vanta_linuxd::FUTEX_WAIT | vanta_linuxd::FUTEX_WAIT_BITSET => {
            let mut val_bytes = [0u8; 4];
            if copy_from_user_into(uaddr, &mut val_bytes).is_err() {
                return SYSCALL_ERROR;
            }
            let current_val = u32::from_ne_bytes(val_bytes);
            if current_val != (val as u32) {
                // EAGAIN = 11 -> return -11 as u64
                return (-(11 as i64)) as u64;
            }
            // Block task
            current_cpu_local().futex_uaddr = uaddr;
            current_cpu_local().futex_bitset = bitset;
            SYSCALL_RETURN_FUTEX_WAIT
        }
        vanta_linuxd::FUTEX_WAKE | vanta_linuxd::FUTEX_WAKE_BITSET => {
            let count = val as u32;
            crate::scheduler::futex_wake(uaddr, count, bitset)
        }
        vanta_linuxd::FUTEX_REQUEUE | vanta_linuxd::FUTEX_CMP_REQUEUE => {
            let count = val as u32;
            crate::scheduler::futex_wake(uaddr, count, vanta_linuxd::FUTEX_BITSET_MATCH_ANY)
        }
        _ => SYSCALL_ERROR,
    }
}

fn linux_arch_prctl_user(code: u64, addr: u64) -> u64 {
    match code {
        vanta_linuxd::ARCH_SET_FS => {
            if addr >= USER_ADDRESS_LIMIT {
                return SYSCALL_ERROR;
            }
            if crate::scheduler::set_current_fs_base(addr).is_ok() {
                set_user_fs_base(addr);
                0
            } else {
                SYSCALL_ERROR
            }
        }
        vanta_linuxd::ARCH_GET_FS => {
            if addr == 0 || addr >= USER_ADDRESS_LIMIT {
                return SYSCALL_ERROR;
            }
            let fs_base = crate::scheduler::current_fs_base();
            if copy_to_user(addr, &fs_base.to_ne_bytes()).is_ok() {
                0
            } else {
                SYSCALL_ERROR
            }
        }
        vanta_linuxd::ARCH_SET_GS => 0,
        vanta_linuxd::ARCH_GET_GS => {
            if addr == 0 || addr >= USER_ADDRESS_LIMIT {
                return SYSCALL_ERROR;
            }
            let zero = 0u64;
            if copy_to_user(addr, &zero.to_ne_bytes()).is_ok() {
                0
            } else {
                SYSCALL_ERROR
            }
        }
        _ => SYSCALL_ERROR,
    }
}

fn linux_fcntl_user(descriptor: u64, cmd: u64, arg: u64) -> u64 {
    match cmd {
        0 /* F_DUPFD */ | 1030 /* F_DUPFD_CLOEXEC */ => {
            crate::scheduler::duplicate_to_current(descriptor, arg).unwrap_or(SYSCALL_ERROR)
        }
        1 /* F_GETFD */ => 0,
        2 /* F_SETFD */ => 0,
        3 /* F_GETFL */ => 2 /* O_RDWR */,
        4 /* F_SETFL */ => 0,
        _ => 0,
    }
}

fn linux_ioctl_user(_descriptor: u64, request: u64, pointer: u64) -> u64 {
    if request == 0x5413 && pointer != 0 {
        let winsize: [u16; 4] = [24, 80, 0, 0];
        let bytes = [
            winsize[0].to_ne_bytes(),
            winsize[1].to_ne_bytes(),
            winsize[2].to_ne_bytes(),
            winsize[3].to_ne_bytes(),
        ]
        .concat();
        let _ = copy_to_user(pointer, &bytes);
        return 0;
    }
    if request == 0x5401 && pointer != 0 {
        let termios = [0u8; 60];
        let _ = copy_to_user(pointer, &termios);
        return 0;
    }
    0
}

fn linux_rt_sigaction_user(
    signal: u64,
    new_action: u64,
    old_action: u64,
    sigsetsize: u64,
) -> u64 {
    if signal == 0 || signal > 64 {
        return SYSCALL_ERROR;
    }
    if sigsetsize != 0 && sigsetsize != 8 {
        return SYSCALL_ERROR;
    }
    if new_action != 0 && (signal == 9 || signal == 19) {
        return SYSCALL_ERROR;
    }
    let old = match crate::scheduler::signal_action(signal) {
        Some(action) => action,
        None => return SYSCALL_ERROR,
    };
    if old_action != 0 {
        let mut bytes = [0u8; 32];
        bytes[0..8].copy_from_slice(&old.sa_handler.to_ne_bytes());
        bytes[8..16].copy_from_slice(&old.sa_flags.to_ne_bytes());
        bytes[16..24].copy_from_slice(&old.sa_restorer.to_ne_bytes());
        bytes[24..32].copy_from_slice(&old.sa_mask.to_ne_bytes());
        if copy_to_user(old_action, &bytes).is_err() {
            return SYSCALL_ERROR;
        }
    }
    if new_action != 0 {
        let Ok(bytes) = copy_from_user(new_action, 32, false) else {
            return SYSCALL_ERROR;
        };
        let action = vanta_linuxd::LinuxSigAction {
            sa_handler: u64::from_ne_bytes(bytes[0..8].try_into().unwrap()),
            sa_flags: u64::from_ne_bytes(bytes[8..16].try_into().unwrap()),
            sa_restorer: u64::from_ne_bytes(bytes[16..24].try_into().unwrap()),
            sa_mask: u64::from_ne_bytes(bytes[24..32].try_into().unwrap()),
        };
        if crate::scheduler::set_signal_action(signal, action).is_err() {
            return SYSCALL_ERROR;
        }
    }
    0
}

fn linux_rt_sigprocmask_user(
    how: u64,
    new_set_ptr: u64,
    old_set_ptr: u64,
    sigsetsize: u64,
) -> u64 {
    if sigsetsize != 0 && sigsetsize != 8 {
        return SYSCALL_ERROR;
    }
    let current_mask = crate::scheduler::current_blocked_mask();
    if old_set_ptr != 0 {
        if copy_to_user(old_set_ptr, &current_mask.to_ne_bytes()).is_err() {
            return SYSCALL_ERROR;
        }
    }
    if new_set_ptr != 0 {
        let Ok(bytes) = copy_from_user(new_set_ptr, 8, false) else {
            return SYSCALL_ERROR;
        };
        let set = u64::from_ne_bytes(bytes.try_into().unwrap());
        let mut new_mask = match how {
            0 /* SIG_BLOCK */ => current_mask | set,
            1 /* SIG_UNBLOCK */ => current_mask & !set,
            2 /* SIG_SETMASK */ => set,
            _ => return SYSCALL_ERROR,
        };
        new_mask &= !crate::scheduler::UNBLOCKABLE_SIGNALS_MASK;
        crate::scheduler::set_current_blocked_mask(new_mask);
    }
    0
}

fn linux_rt_sigreturn_user(frame_ptr: *mut u64, user_rsp: &mut u64) -> u64 {
    let uc_ptr = *user_rsp;
    let uc_size = core::mem::size_of::<vanta_linuxd::UContext>() as u64;
    let uc_bytes = match copy_from_user(uc_ptr, uc_size, false) {
        Ok(bytes) => bytes,
        Err(_) => match copy_from_user(uc_ptr + 8, uc_size, false) {
            Ok(bytes) => bytes,
            Err(_) => {
                crate::serial_println!("[sigreturn] failed to copy UContext from user_rsp {:#x}", uc_ptr);
                return SYSCALL_ERROR;
            }
        },
    };
    let uc: vanta_linuxd::UContext = unsafe {
        core::ptr::read_unaligned(uc_bytes.as_ptr() as *const vanta_linuxd::UContext)
    };

    let restored_mask = uc.uc_sigmask & !crate::scheduler::UNBLOCKABLE_SIGNALS_MASK;
    crate::scheduler::set_current_blocked_mask(restored_mask);

    let ctx = &uc.uc_mcontext;
    unsafe {
        *frame_ptr.add(0) = ctx.rax;
        *frame_ptr.add(1) = ctx.rdi;
        *frame_ptr.add(2) = ctx.rsi;
        *frame_ptr.add(3) = ctx.rdx;
        *frame_ptr.add(4) = ctx.r8;
        *frame_ptr.add(5) = ctx.r9;
        *frame_ptr.add(6) = ctx.r10;
        *frame_ptr.add(7) = ctx.rip;
        *frame_ptr.add(8) = ctx.rflags | 0x202;
        *frame_ptr.add(9) = ctx.rbx;
        *frame_ptr.add(10) = ctx.rbp;
        *frame_ptr.add(11) = ctx.r12;
        *frame_ptr.add(12) = ctx.r13;
        *frame_ptr.add(13) = ctx.r14;
        *frame_ptr.add(14) = ctx.r15;
    }
    *user_rsp = ctx.rsp;

    crate::serial_println!(
        "[sigreturn] restored rip={:#x} rsp={:#x} rax={:#x} mask={:#x}",
        ctx.rip,
        ctx.rsp,
        ctx.rax,
        restored_mask
    );

    ctx.rax
}

fn inject_signal_frame(
    signo: u64,
    action: vanta_linuxd::LinuxSigAction,
    frame_ptr: *mut u64,
    user_rsp: &mut u64,
    current_blocked_mask: u64,
) -> Result<(), ()> {
    let old_user_sp = *user_rsp;
    let frame_size = core::mem::size_of::<vanta_linuxd::RtSigFrame>() as u64;
    let new_user_sp = (old_user_sp.saturating_sub(frame_size) & !15) - 8;

    let retcode: [u8; 16] = [
        0x48, 0xc7, 0xc0, 0x0f, 0x00, 0x00, 0x00, // mov $15, %rax
        0x0f, 0x05,                               // syscall
        0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, // nop
    ];

    let pretcode = if action.sa_flags & vanta_linuxd::SA_RESTORER != 0 && action.sa_restorer != 0 {
        action.sa_restorer
    } else {
        new_user_sp + core::mem::offset_of!(vanta_linuxd::RtSigFrame, retcode) as u64
    };

    let mut sigcontext = vanta_linuxd::SigContext::default();
    unsafe {
        sigcontext.rax = *frame_ptr.add(0);
        sigcontext.rdi = *frame_ptr.add(1);
        sigcontext.rsi = *frame_ptr.add(2);
        sigcontext.rdx = *frame_ptr.add(3);
        sigcontext.r8 = *frame_ptr.add(4);
        sigcontext.r9 = *frame_ptr.add(5);
        sigcontext.r10 = *frame_ptr.add(6);
        sigcontext.rip = *frame_ptr.add(7);
        sigcontext.rflags = *frame_ptr.add(8);
        sigcontext.rbx = *frame_ptr.add(9);
        sigcontext.rbp = *frame_ptr.add(10);
        sigcontext.r12 = *frame_ptr.add(11);
        sigcontext.r13 = *frame_ptr.add(12);
        sigcontext.r14 = *frame_ptr.add(13);
        sigcontext.r15 = *frame_ptr.add(14);
        sigcontext.rsp = old_user_sp;
        sigcontext.cs = 0x23;
        sigcontext.fs = 0x1b;
        sigcontext.gs = 0x1b;
        sigcontext.oldmask = current_blocked_mask;
    }

    let ucontext = vanta_linuxd::UContext {
        uc_flags: 0,
        uc_link: 0,
        uc_stack: vanta_linuxd::SigAltStack::default(),
        uc_mcontext: sigcontext,
        uc_sigmask: current_blocked_mask,
        __fpregs_mem: [0; 64],
    };

    let siginfo = vanta_linuxd::SigInfo {
        si_signo: signo as i32,
        si_errno: 0,
        si_code: vanta_linuxd::SI_USER,
        _pad: [0; 29],
    };

    let frame = vanta_linuxd::RtSigFrame {
        pretcode,
        uc: ucontext,
        info: siginfo,
        retcode,
    };

    let frame_bytes = unsafe {
        core::slice::from_raw_parts(
            core::ptr::addr_of!(frame).cast::<u8>(),
            core::mem::size_of::<vanta_linuxd::RtSigFrame>(),
        )
    };
    copy_to_user(new_user_sp, frame_bytes)?;

    unsafe {
        *frame_ptr.add(0) = 0; // rax
        *frame_ptr.add(1) = signo; // rdi = signo
        *frame_ptr.add(2) = new_user_sp + core::mem::offset_of!(vanta_linuxd::RtSigFrame, info) as u64; // rsi = &info
        *frame_ptr.add(3) = new_user_sp + core::mem::offset_of!(vanta_linuxd::RtSigFrame, uc) as u64; // rdx = &uc
        *frame_ptr.add(7) = action.sa_handler; // rcx / rip = handler entry
    }
    *user_rsp = new_user_sp;

    Ok(())
}

fn dispatch_native(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    _arg6: u64,
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
        SYS_RENAME => path_mutation_user(arg1, arg2, arg3, arg4),
        SYS_DUP => {
            if arg2 != 0 {
                crate::scheduler::duplicate_to_current(arg1, arg2).unwrap_or(SYSCALL_ERROR)
            } else {
                duplicate_legacy_user(arg1)
            }
        }
        SYS_PIPE => pipe_user(arg1, arg2),
        SYS_SOCKET => socket_user(arg1, arg2, arg3),
        SYS_CONNECT => connect_user(arg1, arg2, arg3),
        SYS_SPAWN => {
            if arg3 == 0 {
                spawn_legacy_user(arg1, arg2)
            } else {
                let native = spawn_native_user(arg1, arg2, arg3, arg4);
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
        SYS_IPC_PAIR => ipc_pair_user(arg1),
        SYS_IPC_SEND => ipc_send_user(arg1, arg2, arg3),
        SYS_IPC_RECV => ipc_recv_user(arg1, arg2, arg3),
        SYS_IPC_REVOKE => crate::scheduler::ipc_revoke_current(arg1)
            .map(|()| 0)
            .unwrap_or(SYSCALL_ERROR),
        SYS_BRK => crate::scheduler::brk_current(arg1),
        SYS_MMAP => linux_mmap_user(arg1, arg2, arg3, arg4, arg5, _arg6),
        SYS_MUNMAP => {
            crate::scheduler::munmap_current(arg1, arg2).map(|()| 0).unwrap_or(SYSCALL_ERROR)
        }
        SYS_EXEC => SYSCALL_RETURN_EXEC,
        SYS_YIELD => SYSCALL_RETURN_YIELD,
        SYS_GETPID => crate::scheduler::current_pid(),
        SYS_GETPPID => crate::scheduler::current_parent_pid(),
        SYS_DISPLAY_INFO => display_info_user(arg1),
        SYS_DISPLAY_BLIT => display_blit_user(arg1, arg2, arg3, arg4, arg5),
        SYS_DISPLAY_FLUSH => display_flush_user(),
        SYS_INPUT_POLL => input_poll_user(arg1),
        SYS_AUDIO_PLAY => audio_play_user(arg1, arg2),
        SYS_EXIT => {
            current_cpu_local().exit_code = arg1;
            SYSCALL_RETURN_EXIT
        }
        _ => SYSCALL_ERROR,
    }
}

pub fn prepare_user_return(context: UserContext, space: AddressSpace) -> *const UserContext {
    current_cpu_local().next_context = context;
    set_user_fs_base(crate::scheduler::current_fs_base());
    unsafe {
        paging::activate(space);
    }
    core::ptr::addr_of!(current_cpu_local().next_context)
}

pub fn set_user_fs_base(fs_base: u64) {
    unsafe { Msr::new(0xc000_0100).write(fs_base) };
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
    let to_read = length.min(65536) as usize;
    let Ok(bytes) = crate::scheduler::read_current(descriptor, to_read) else {
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

fn ipc_pair_user(pointer: u64) -> u64 {
    let Ok((sender, receiver)) = crate::scheduler::open_ipc_pair_current() else {
        return SYSCALL_ERROR;
    };
    let bytes = [
        (sender as u32).to_ne_bytes(),
        (receiver as u32).to_ne_bytes(),
    ];
    if copy_to_user(pointer, &bytes.concat()).is_err() {
        return SYSCALL_ERROR;
    }
    0
}

fn ipc_send_user(descriptor: u64, pointer: u64, length: u64) -> u64 {
    if length > 256 {
        return SYSCALL_ERROR;
    }
    let Ok(bytes) = copy_from_user(pointer, length, false) else {
        return SYSCALL_ERROR;
    };
    crate::scheduler::ipc_send_current(descriptor, &bytes)
        .map(|()| length)
        .unwrap_or(SYSCALL_ERROR)
}

fn ipc_recv_user(descriptor: u64, pointer: u64, length: u64) -> u64 {
    if length > 256 {
        return SYSCALL_ERROR;
    }
    let result = match crate::scheduler::ipc_receive_current(descriptor) {
        Ok(result) => result,
        Err(()) => return SYSCALL_ERROR,
    };
    let Some(bytes) = result else {
        current_cpu_local().block_descriptor = descriptor;
        return SYSCALL_WOULD_BLOCK;
    };
    let bytes = &bytes[..bytes.len().min(length as usize)];
    if copy_to_user(pointer, bytes).is_err() {
        return SYSCALL_ERROR;
    }
    bytes.len() as u64
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
    const ALLOWED_FLAGS: u64 = 0x80000 | 0x800 | 0x4000;
    if flags & !ALLOWED_FLAGS != 0 {
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
        crate::serial_println!("[spawn] failed to copy path from user");
        return SYSCALL_ERROR;
    };
    let Ok(path) = core::str::from_utf8(&path) else {
        crate::serial_println!("[spawn] path is not valid utf-8");
        return SYSCALL_ERROR;
    };
    let stdio_length = match with_args {
        1 => 40,
        2 => 56,
        _ => 24,
    };
    let Ok(stdio) = copy_from_user(stdio_pointer, stdio_length, false) else {
        crate::serial_println!("[spawn] failed to copy stdio from user");
        return SYSCALL_ERROR;
    };
    let Ok(image) = crate::vfs::read_root(path) else {
        crate::serial_println!("[spawn] read_root failed for '{}'", path);
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
    let linux_personality = path.starts_with("/compat/linux/");
    let process = match if linux_personality {
        crate::process::load_linux_elf(&image)
    } else if with_args == 2 {
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
    } {
        Ok(proc) => proc,
        Err(err) => {
            crate::serial_println!("[spawn] load_elf failed for '{}': {:?}", path, err);
            return SYSCALL_ERROR;
        }
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
        Ok(Some((_child_tgid, code))) => code,
        Ok(None) => SYSCALL_RETURN_WAIT,
        Err(()) => SYSCALL_ERROR,
    }
}

fn kill_user(pid: u64, signal: u64) -> u64 {
    if signal > 64 {
        return SYSCALL_ERROR;
    }
    crate::scheduler::kill_process(pid, signal)
        .map(|_| 0)
        .unwrap_or(SYSCALL_ERROR)
}

fn sigaction_user(signal: u64, new_action: u64, old_action: u64) -> u64 {
    linux_rt_sigaction_user(signal, new_action, old_action, 8)
}

fn copy_from_user(pointer: u64, length: u64, writable: bool) -> Result<Vec<u8>, ()> {
    if length > 65536 {
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

fn copy_from_user_into(pointer: u64, buf: &mut [u8]) -> Result<(), ()> {
    for (offset, byte) in buf.iter_mut().enumerate() {
        *byte = read_user_byte(pointer.checked_add(offset as u64).ok_or(())?, false)?;
    }
    Ok(())
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
    let status_ptr = unsafe { *frame.add(2) };
    crate::scheduler::wait_current(child_pid, status_ptr, user_context(frame, stack_pointer))
}

#[no_mangle]
extern "C" fn vanta_syscall_block(frame: *const u64, stack_pointer: u64) -> *const UserContext {
    let descriptor = current_cpu_local().block_descriptor;
    current_cpu_local().block_descriptor = 0;
    crate::scheduler::block_pipe_current(descriptor, user_context(frame, stack_pointer))
}

#[no_mangle]
extern "C" fn vanta_syscall_futex_wait(frame: *const u64, stack_pointer: u64) -> *const UserContext {
    let uaddr = current_cpu_local().futex_uaddr;
    let bitset = current_cpu_local().futex_bitset;
    current_cpu_local().futex_uaddr = 0;
    current_cpu_local().futex_bitset = 0;
    crate::scheduler::futex_wait_current(uaddr, bitset, user_context(frame, stack_pointer))
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
            rbx: *frame.add(9),
            rbp: *frame.add(10),
            r12: *frame.add(11),
            r13: *frame.add(12),
            r14: *frame.add(13),
            r15: *frame.add(14),
            rdi: *frame.add(1),
            rsi: *frame.add(2),
            rdx: *frame.add(3),
            r8:  *frame.add(4),
            r9:  *frame.add(5),
            r10: *frame.add(6),
            instruction_pointer: *frame.add(7),
            flags: *frame.add(8),
            stack_pointer,
        }
    }
}

#[no_mangle]
extern "C" fn vanta_syscall_exit(code: u64) -> *const UserContext {
    crate::scheduler::exit_group_current(code)
}

#[no_mangle]
extern "C" fn vanta_syscall_thread_exit(code: u64) -> *const UserContext {
    crate::scheduler::exit_current(code)
}

fn linux_epoll_ctl_user(epfd: u64, op: u32, fd: u64, event_ptr: u64) -> u64 {
    let (events, data) = if op != vanta_linuxd::EPOLL_CTL_DEL {
        let Ok(ev_bytes) = copy_from_user(event_ptr, core::mem::size_of::<vanta_linuxd::epoll_event>() as u64, false) else {
            return SYSCALL_ERROR;
        };
        let ev = unsafe { *(ev_bytes.as_ptr() as *const vanta_linuxd::epoll_event) };
        (ev.events, ev.data)
    } else {
        (0, 0)
    };
    if crate::scheduler::epoll_ctl_current(epfd, op, fd, events, data).is_ok() {
        0
    } else {
        SYSCALL_ERROR
    }
}

fn linux_epoll_wait_user(epfd: u64, events_ptr: u64, maxevents: usize, _timeout: u64) -> u64 {
    let Ok(ready) = crate::scheduler::epoll_wait_current(epfd, maxevents) else {
        return SYSCALL_ERROR;
    };
    let mut out_events = alloc::vec::Vec::new();
    for (events, data) in &ready {
        out_events.push(vanta_linuxd::epoll_event {
            events: *events,
            data: *data,
        });
    }
    let byte_len = out_events.len() * core::mem::size_of::<vanta_linuxd::epoll_event>();
    if byte_len > 0 {
        let slice = unsafe {
            core::slice::from_raw_parts(out_events.as_ptr() as *const u8, byte_len)
        };
        if copy_to_user(events_ptr, slice).is_err() {
            return SYSCALL_ERROR;
        }
    }
    ready.len() as u64
}

fn linux_poll_user(fds_ptr: u64, nfds: usize, _timeout: u64) -> u64 {
    let elem_size = core::mem::size_of::<vanta_linuxd::pollfd>();
    let total_bytes = nfds * elem_size;
    let Ok(bytes) = copy_from_user(fds_ptr, total_bytes as u64, false) else {
        return SYSCALL_ERROR;
    };
    let mut fds: alloc::vec::Vec<vanta_linuxd::pollfd> = alloc::vec::Vec::with_capacity(nfds);
    for i in 0..nfds {
        let pfd = unsafe { *(bytes.as_ptr().add(i * elem_size) as *const vanta_linuxd::pollfd) };
        fds.push(pfd);
    }
    let mut ready_count = 0u64;
    for pfd in &mut fds {
        pfd.revents = 0;
        if pfd.fd >= 0 {
            let mut rev = 0i16;
            if pfd.events & (vanta_linuxd::EPOLLIN as i16) != 0 {
                rev |= vanta_linuxd::EPOLLIN as i16;
            }
            if pfd.events & (vanta_linuxd::EPOLLOUT as i16) != 0 {
                rev |= vanta_linuxd::EPOLLOUT as i16;
            }
            pfd.revents = rev;
            if rev != 0 {
                ready_count += 1;
            }
        }
    }
    let slice = unsafe {
        core::slice::from_raw_parts(fds.as_ptr() as *const u8, total_bytes)
    };
    let _ = copy_to_user(fds_ptr, slice);
    ready_count
}

fn display_info_user(info_ptr: u64) -> u64 {
    let info = crate::framebuffer::display_info();
    let slice = unsafe {
        core::slice::from_raw_parts(&info as *const _ as *const u8, core::mem::size_of::<vanta_abi::DisplayInfo>())
    };
    if copy_to_user(info_ptr, slice).is_ok() {
        0
    } else {
        SYSCALL_ERROR
    }
}

fn display_blit_user(x: u64, y: u64, w: u64, h: u64, buf_ptr: u64) -> u64 {
    let mut writer = crate::framebuffer::WRITER.lock();
    let Some(ref mut writer) = *writer else {
        return SYSCALL_ERROR;
    };
    let width = writer.width as u64;
    let height = writer.height as u64;
    let pitch = writer.pitch as u64;
    let bpp = writer.bpp as u64;
    if x >= width || y >= height {
        return SYSCALL_ERROR;
    }
    let actual_w = w.min(width - x);
    let actual_h = h.min(height - y);
    let row_bytes = actual_w * bpp;
    let src_stride = w * bpp;

    for row in 0..actual_h {
        let src_row_addr = match buf_ptr.checked_add(row * src_stride) {
            Some(a) => a,
            None => return SYSCALL_ERROR,
        };
        let dst_off = ((y + row) * pitch + x * bpp) as usize;

        let mut copied = 0u64;
        while copied < row_bytes {
            let cur_src = src_row_addr + copied;
            let cur_dst = dst_off + copied as usize;
            let page_offset = cur_src & 0xfff;
            let bytes_in_page = 0x1000 - page_offset;
            let chunk = (row_bytes - copied).min(bytes_in_page) as usize;

            let Ok(phys_virt) = user_physical_address(cur_src, false) else {
                return SYSCALL_ERROR;
            };
            unsafe {
                core::ptr::copy_nonoverlapping(
                    phys_virt as *const u8,
                    writer.addr.add(cur_dst),
                    chunk,
                );
            }
            copied += chunk as u64;
        }
    }
    0
}

fn display_flush_user() -> u64 {
    if crate::framebuffer::display_flush() {
        0
    } else {
        SYSCALL_ERROR
    }
}

fn input_poll_user(event_ptr: u64) -> u64 {
    if let Some(ev) = crate::input::poll_event() {
        let slice = unsafe {
            core::slice::from_raw_parts(&ev as *const _ as *const u8, core::mem::size_of::<vanta_abi::InputEvent>())
        };
        if copy_to_user(event_ptr, slice).is_ok() {
            1
        } else {
            SYSCALL_ERROR
        }
    } else {
        0
    }
}

fn audio_play_user(_buf_ptr: u64, len: u64) -> u64 {
    len
}
