#![no_std]
#![allow(non_camel_case_types)]

use vanta_abi::{CapabilityId, Syscall};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxOp {
    Read,
    Write,
    Open,
    Close,
    Stat,
    FStat,
    LStat,
    LSeek,
    GetDents,
    GetDents64,
    MMap,
    MProtect,
    MUnmap,
    Brk,
    Pipe,
    Pipe2,
    Dup,
    Dup2,
    Dup3,
    Fcntl,
    Ioctl,
    Readv,
    Writev,
    Access,
    FAccessAt,
    GetPid,
    GetPPid,
    GetUid,
    GetGid,
    GetEUid,
    GetEGid,
    SetPGid,
    GetPGrp,
    Uname,
    GetCwd,
    ChDir,
    FChDir,
    ReadLink,
    ReadLinkAt,
    ClockGetTime,
    GetTimeOfDay,
    Nanosleep,
    RtSigAction,
    RtSigProcMask,
    RtSigReturn,
    SigAltStack,
    Socket,
    Connect,
    Accept,
    Accept4,
    SendTo,
    RecvFrom,
    SendMsg,
    RecvMsg,
    Bind,
    Listen,
    GetSockName,
    GetPeerName,
    SetSockOpt,
    GetSockOpt,
    Fork,
    VFork,
    Clone,
    Clone3,
    ExecVe,
    Exit,
    ExitGroup,
    Wait4,
    Kill,
    TKill,
    TgKill,
    ArchPrctl,
    SetTidAddress,
    GetTid,
    Futex,
    SetRobustList,
    Rseq,
    GetRandom,
    SchedGetAffinity,
    SchedYield,
    Poll,
    PPoll,
    Select,
    PSelect6,
    EPollCreate,
    EPollCreate1,
    EPollCtl,
    EPollWait,
    EPollPWait,
    EventFd,
    EventFd2,
    Unsupported(u64),
}

pub const EPOLL_CTL_ADD: u32 = 1;
pub const EPOLL_CTL_DEL: u32 = 2;
pub const EPOLL_CTL_MOD: u32 = 3;

pub const EPOLLIN: u32 = 0x00000001;
pub const EPOLLPRI: u32 = 0x00000002;
pub const EPOLLOUT: u32 = 0x00000004;
pub const EPOLLERR: u32 = 0x00000008;
pub const EPOLLHUP: u32 = 0x00000010;
pub const EPOLLRDHUP: u32 = 0x00002000;
pub const EPOLLET: u32 = 0x80000000;

pub const EFD_SEMAPHORE: u32 = 1;
pub const EFD_CLOEXEC: u32 = 0x00080000;
pub const EFD_NONBLOCK: u32 = 0x00000800;

pub const CLONE_VM: u64 = 0x00000100;
pub const CLONE_FS: u64 = 0x00000200;
pub const CLONE_FILES: u64 = 0x00000400;
pub const CLONE_SIGHAND: u64 = 0x00000800;
pub const CLONE_THREAD: u64 = 0x00010000;
pub const CLONE_SETTLS: u64 = 0x00080000;
pub const CLONE_PARENT_SETTID: u64 = 0x00100000;
pub const CLONE_CHILD_CLEARTID: u64 = 0x00200000;
pub const CLONE_CHILD_SETTID: u64 = 0x01000000;

pub const ARCH_SET_GS: u64 = 0x1001;
pub const ARCH_SET_FS: u64 = 0x1002;
pub const ARCH_GET_FS: u64 = 0x1003;
pub const ARCH_GET_GS: u64 = 0x1004;

pub const FUTEX_WAIT: u32 = 0;
pub const FUTEX_WAKE: u32 = 1;
pub const FUTEX_FD: u32 = 2;
pub const FUTEX_REQUEUE: u32 = 3;
pub const FUTEX_CMP_REQUEUE: u32 = 4;
pub const FUTEX_WAKE_OP: u32 = 5;
pub const FUTEX_WAIT_BITSET: u32 = 9;
pub const FUTEX_WAKE_BITSET: u32 = 10;

pub const FUTEX_PRIVATE_FLAG: u32 = 128;
pub const FUTEX_CLOCK_REALTIME: u32 = 256;
pub const FUTEX_BITSET_MATCH_ANY: u32 = 0xffff_ffff;
pub const FUTEX_CMD_MASK: u32 = !(FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct clone_args {
    pub flags: u64,
    pub pidfd: u64,
    pub child_tid: u64,
    pub parent_tid: u64,
    pub exit_signal: u64,
    pub stack: u64,
    pub stack_size: u64,
    pub tls: u64,
    pub set_tid: u64,
    pub set_tid_size: u64,
    pub cgroup: u64,
}

#[allow(non_camel_case_types)]
pub type sigset_t = u64;

pub const SIGHUP: u64 = 1;
pub const SIGINT: u64 = 2;
pub const SIGQUIT: u64 = 3;
pub const SIGILL: u64 = 4;
pub const SIGTRAP: u64 = 5;
pub const SIGABRT: u64 = 6;
pub const SIGBUS: u64 = 7;
pub const SIGFPE: u64 = 8;
pub const SIGKILL: u64 = 9;
pub const SIGUSR1: u64 = 10;
pub const SIGSEGV: u64 = 11;
pub const SIGUSR2: u64 = 12;
pub const SIGPIPE: u64 = 13;
pub const SIGALRM: u64 = 14;
pub const SIGTERM: u64 = 15;
pub const SIGSTKFLT: u64 = 16;
pub const SIGCHLD: u64 = 17;
pub const SIGCONT: u64 = 18;
pub const SIGSTOP: u64 = 19;
pub const SIGTSTP: u64 = 20;
pub const SIGTTIN: u64 = 21;
pub const SIGTTOU: u64 = 22;
pub const SIGURG: u64 = 23;
pub const SIGXCPU: u64 = 24;
pub const SIGXFSZ: u64 = 25;
pub const SIGVTALRM: u64 = 26;
pub const SIGPROF: u64 = 27;
pub const SIGWINCH: u64 = 28;
pub const SIGIO: u64 = 29;
pub const SIGPWR: u64 = 30;
pub const SIGSYS: u64 = 31;

pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;

pub const SIG_BLOCK: u64 = 0;
pub const SIG_UNBLOCK: u64 = 1;
pub const SIG_SETMASK: u64 = 2;

pub const SA_NOCLDSTOP: u64 = 0x00000001;
pub const SA_NOCLDWAIT: u64 = 0x00000002;
pub const SA_SIGINFO: u64 = 0x00000004;
pub const SA_ONSTACK: u64 = 0x08000000;
pub const SA_RESTART: u64 = 0x10000000;
pub const SA_NODEFER: u64 = 0x40000000;
pub const SA_RESETHAND: u64 = 0x80000000;
pub const SA_RESTORER: u64 = 0x04000000;

pub const SI_USER: i32 = 0;
pub const SI_KERNEL: i32 = 0x80;
pub const SI_TKILL: i32 = -6;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LinuxSigAction {
    pub sa_handler: u64,
    pub sa_flags: u64,
    pub sa_restorer: u64,
    pub sa_mask: sigset_t,
}

#[allow(non_camel_case_types)]
pub type rt_sigaction = LinuxSigAction;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SigAltStack {
    pub ss_sp: u64,
    pub ss_flags: i32,
    pub _pad: i32,
    pub ss_size: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct epoll_event {
    pub events: u32,
    pub data: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct pollfd {
    pub fd: i32,
    pub events: i16,
    pub revents: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SigContext {
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rsp: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cs: u16,
    pub gs: u16,
    pub fs: u16,
    pub __pad0: u16,
    pub err: u64,
    pub trapno: u64,
    pub oldmask: u64,
    pub cr2: u64,
    pub fpstate: u64,
    pub reserved: [u64; 8],
}

#[allow(non_camel_case_types)]
pub type sigcontext = SigContext;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UContext {
    pub uc_flags: u64,
    pub uc_link: u64,
    pub uc_stack: SigAltStack,
    pub uc_mcontext: SigContext,
    pub uc_sigmask: sigset_t,
    pub __fpregs_mem: [u64; 64],
}

impl Default for UContext {
    fn default() -> Self {
        Self {
            uc_flags: 0,
            uc_link: 0,
            uc_stack: SigAltStack::default(),
            uc_mcontext: SigContext::default(),
            uc_sigmask: 0,
            __fpregs_mem: [0; 64],
        }
    }
}

#[allow(non_camel_case_types)]
pub type ucontext_t = UContext;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SigInfo {
    pub si_signo: i32,
    pub si_errno: i32,
    pub si_code: i32,
    pub _pad: [i32; 29],
}

#[allow(non_camel_case_types)]
pub type siginfo_t = SigInfo;

impl Default for SigInfo {
    fn default() -> Self {
        Self {
            si_signo: 0,
            si_errno: 0,
            si_code: 0,
            _pad: [0; 29],
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtSigFrame {
    pub pretcode: u64,
    pub uc: UContext,
    pub info: SigInfo,
    pub retcode: [u8; 16],
}

impl Default for RtSigFrame {
    fn default() -> Self {
        Self {
            pretcode: 0,
            uc: UContext::default(),
            info: SigInfo::default(),
            retcode: [0; 16],
        }
    }
}

#[allow(non_camel_case_types)]
pub type rt_sigframe = RtSigFrame;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Translation {
    pub linux_number: u64,
    pub operation: LinuxOp,
    pub native: Option<Syscall>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsupportedSyscall {
    pub number: u64,
}

pub fn translate(number: u64) -> Result<Translation, UnsupportedSyscall> {
    let (operation, native) = match number {
        0 => (LinuxOp::Read, Some(Syscall::Read)),
        1 => (LinuxOp::Write, Some(Syscall::Write)),
        2 | 257 => (LinuxOp::Open, Some(Syscall::OpenAt)),
        3 => (LinuxOp::Close, Some(Syscall::Close)),
        4 => (LinuxOp::Stat, Some(Syscall::FStat)),
        5 => (LinuxOp::FStat, Some(Syscall::FStat)),
        6 => (LinuxOp::LStat, Some(Syscall::FStat)),
        7 => (LinuxOp::Poll, None),
        262 => (LinuxOp::FStat, Some(Syscall::FStat)),
        8 => (LinuxOp::LSeek, Some(Syscall::LSeek)),
        9 => (LinuxOp::MMap, Some(Syscall::MMap)),
        10 => (LinuxOp::MProtect, None),
        11 => (LinuxOp::MUnmap, Some(Syscall::MUnmap)),
        12 => (LinuxOp::Brk, Some(Syscall::Brk)),
        13 => (LinuxOp::RtSigAction, Some(Syscall::SigAction)),
        14 => (LinuxOp::RtSigProcMask, None),
        15 => (LinuxOp::RtSigReturn, None),
        16 => (LinuxOp::Ioctl, Some(Syscall::TtyIoctl)),
        19 => (LinuxOp::Readv, None),
        20 => (LinuxOp::Writev, None),
        21 | 269 | 439 => (LinuxOp::Access, None),
        22 => (LinuxOp::Pipe, Some(Syscall::Pipe2)),
        23 => (LinuxOp::Select, None),
        24 => (LinuxOp::SchedYield, Some(Syscall::Yield)),
        293 => (LinuxOp::Pipe2, Some(Syscall::Pipe2)),
        32 => (LinuxOp::Dup, Some(Syscall::Dup3)),
        33 => (LinuxOp::Dup2, Some(Syscall::Dup3)),
        292 => (LinuxOp::Dup3, Some(Syscall::Dup3)),
        35 | 230 => (LinuxOp::Nanosleep, None),
        39 => (LinuxOp::GetPid, Some(Syscall::GetPid)),
        41 => (LinuxOp::Socket, Some(Syscall::Socket)),
        42 => (LinuxOp::Connect, Some(Syscall::Connect)),
        43 | 288 => (LinuxOp::Accept, None),
        44 => (LinuxOp::SendTo, None),
        45 => (LinuxOp::RecvFrom, None),
        49 => (LinuxOp::Bind, None),
        50 => (LinuxOp::Listen, None),
        51 => (LinuxOp::GetSockName, None),
        52 => (LinuxOp::GetPeerName, None),
        54 => (LinuxOp::SetSockOpt, None),
        55 => (LinuxOp::GetSockOpt, None),
        56 => (LinuxOp::Clone, None),
        57 => (LinuxOp::Fork, None),
        58 => (LinuxOp::VFork, None),
        59 => (LinuxOp::ExecVe, Some(Syscall::ExecVe)),
        60 => (LinuxOp::Exit, Some(Syscall::Exit)),
        61 => (LinuxOp::Wait4, None),
        62 => (LinuxOp::Kill, Some(Syscall::Kill)),
        63 => (LinuxOp::Uname, None),
        72 => (LinuxOp::Fcntl, None),
        78 => (LinuxOp::GetDents, Some(Syscall::GetDents)),
        213 => (LinuxOp::EPollCreate, None),
        217 => (LinuxOp::GetDents64, Some(Syscall::GetDents)),
        79 => (LinuxOp::GetCwd, None),
        80 | 81 => (LinuxOp::ChDir, None),
        89 | 267 => (LinuxOp::ReadLink, None),
        96 => (LinuxOp::GetTimeOfDay, None),
        102 | 107 => (LinuxOp::GetUid, None),
        104 | 108 => (LinuxOp::GetGid, None),
        109 => (LinuxOp::SetPGid, None),
        110 => (LinuxOp::GetPPid, Some(Syscall::GetPpid)),
        111 | 121 => (LinuxOp::GetPGrp, None),
        131 => (LinuxOp::SigAltStack, None),
        158 => (LinuxOp::ArchPrctl, None),
        186 => (LinuxOp::GetTid, None),
        200 => (LinuxOp::TKill, None),
        202 => (LinuxOp::Futex, None),
        204 => (LinuxOp::SchedGetAffinity, None),
        218 => (LinuxOp::SetTidAddress, None),
        228 => (LinuxOp::ClockGetTime, None),
        231 => (LinuxOp::ExitGroup, None),
        232 => (LinuxOp::EPollWait, None),
        233 => (LinuxOp::EPollCtl, None),
        234 => (LinuxOp::TgKill, None),
        270 => (LinuxOp::PSelect6, None),
        271 => (LinuxOp::PPoll, None),
        273 => (LinuxOp::SetRobustList, None),
        281 => (LinuxOp::EPollPWait, None),
        284 => (LinuxOp::EventFd, None),
        290 => (LinuxOp::EventFd2, None),
        291 => (LinuxOp::EPollCreate1, None),
        318 => (LinuxOp::GetRandom, None),
        334 => (LinuxOp::Rseq, None),
        435 => (LinuxOp::Clone3, None),
        number => return Err(UnsupportedSyscall { number }),
    };
    Ok(Translation {
        linux_number: number,
        operation,
        native,
    })
}

pub fn is_static_elf_supported(interpreter: Option<&[u8]>) -> bool {
    interpreter.is_none()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElfError {
    TooSmall,
    NotX86_64,
    UnsupportedType,
    InvalidProgramTable,
    DynamicInterpreter,
    NoLoadSegments,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadSegment {
    pub virtual_address: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticElf {
    pub entry: u64,
    pub segments: [Option<LoadSegment>; 16],
    pub segment_count: usize,
    pub interpreter_offset: Option<u64>,
    pub interpreter_size: Option<u64>,
}

impl StaticElf {
    pub fn parse(bytes: &[u8]) -> Result<Self, ElfError> {
        if bytes.len() < 64 || &bytes[..4] != b"\x7fELF" || bytes[4] != 2 || bytes[5] != 1 {
            return Err(ElfError::NotX86_64);
        }
        let elf_type = read_u16(bytes, 16).ok_or(ElfError::TooSmall)?;
        if elf_type != 2 && elf_type != 3 {
            return Err(ElfError::UnsupportedType);
        }
        let entry = read_u64(bytes, 24).ok_or(ElfError::TooSmall)?;
        let phoff = read_u64(bytes, 32).ok_or(ElfError::TooSmall)? as usize;
        let phentsize = read_u16(bytes, 54).ok_or(ElfError::TooSmall)? as usize;
        let phnum = read_u16(bytes, 56).ok_or(ElfError::TooSmall)? as usize;
        if phentsize < 56
            || phnum > 16
            || phoff
                .checked_add(
                    phentsize
                        .checked_mul(phnum)
                        .ok_or(ElfError::InvalidProgramTable)?,
                )
                .filter(|end| *end <= bytes.len())
                .is_none()
        {
            return Err(ElfError::InvalidProgramTable);
        }
        let mut segments = [None; 16];
        let mut segment_count = 0;
        let mut interpreter_offset = None;
        let mut interpreter_size = None;
        for index in 0..phnum {
            let offset = phoff + index * phentsize;
            let kind = read_u32(bytes, offset).ok_or(ElfError::InvalidProgramTable)?;
            if kind == 3 {
                let file_offset = read_u64(bytes, offset + 8).ok_or(ElfError::InvalidProgramTable)?;
                let file_size = read_u64(bytes, offset + 32).ok_or(ElfError::InvalidProgramTable)?;
                if file_offset
                    .checked_add(file_size)
                    .filter(|end| *end <= bytes.len() as u64)
                    .is_none()
                {
                    return Err(ElfError::InvalidProgramTable);
                }
                interpreter_offset = Some(file_offset);
                interpreter_size = Some(file_size);
                continue;
            }
            if kind != 1 {
                continue;
            }
            let file_offset = read_u64(bytes, offset + 8).ok_or(ElfError::InvalidProgramTable)?;
            let virtual_address =
                read_u64(bytes, offset + 16).ok_or(ElfError::InvalidProgramTable)?;
            let file_size = read_u64(bytes, offset + 32).ok_or(ElfError::InvalidProgramTable)?;
            let memory_size = read_u64(bytes, offset + 40).ok_or(ElfError::InvalidProgramTable)?;
            let flags = read_u32(bytes, offset + 4).ok_or(ElfError::InvalidProgramTable)?;
            if memory_size < file_size
                || file_offset
                    .checked_add(file_size)
                    .filter(|end| *end <= bytes.len() as u64)
                    .is_none()
            {
                return Err(ElfError::InvalidProgramTable);
            }
            segments[segment_count] = Some(LoadSegment {
                virtual_address,
                file_offset,
                file_size,
                memory_size,
                flags,
            });
            segment_count += 1;
        }
        if segment_count == 0 {
            return Err(ElfError::NoLoadSegments);
        }
        Ok(Self {
            entry,
            segments,
            segment_count,
            interpreter_offset,
            interpreter_size,
        })
    }

    pub fn interpreter<'a>(&self, bytes: &'a [u8]) -> Option<&'a str> {
        let offset = self.interpreter_offset? as usize;
        let size = self.interpreter_size? as usize;
        let raw = bytes.get(offset..offset + size)?;
        let trimmed = raw.strip_suffix(b"\0").unwrap_or(raw);
        core::str::from_utf8(trimmed).ok()
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}
fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinuxSyscallRequest {
    pub number: u64,
    pub args: [u64; 6],
    pub authority: CapabilityId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerDecision {
    Native { syscall: Syscall, args: [u64; 4] },
    ProcessPrimitive { operation: LinuxOp },
    Unsupported { number: u64 },
}

pub fn broker(request: LinuxSyscallRequest) -> BrokerDecision {
    let Ok(translation) = translate(request.number) else {
        return BrokerDecision::Unsupported {
            number: request.number,
        };
    };
    let Some(syscall) = translation.native else {
        return BrokerDecision::ProcessPrimitive {
            operation: translation.operation,
        };
    };
    BrokerDecision::Native {
        syscall,
        args: [
            request.args[0],
            request.args[1],
            request.args[2],
            request.args[3],
        ],
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    #[test]
    fn parses_static_x86_64_load_metadata() {
        let mut image = [0u8; 120];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&2u16.to_le_bytes());
        image[24..32].copy_from_slice(&0x401000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());
        image[64..68].copy_from_slice(&1u32.to_le_bytes());
        image[68..72].copy_from_slice(&5u32.to_le_bytes());
        image[72..80].copy_from_slice(&0u64.to_le_bytes());
        image[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        image[96..104].copy_from_slice(&1u64.to_le_bytes());
        image[104..112].copy_from_slice(&1u64.to_le_bytes());
        let elf = StaticElf::parse(&image).unwrap();
        assert_eq!(elf.entry, 0x401000);
        assert_eq!(elf.segment_count, 1);
    }

    #[test]
    fn broker_maps_file_io_and_reports_process_primitives() {
        let request = LinuxSyscallRequest {
            number: 1,
            args: [1, 2, 3, 4, 5, 6],
            authority: CapabilityId::INVALID,
        };
        assert_eq!(
            broker(request),
            BrokerDecision::Native {
                syscall: Syscall::Write,
                args: [1, 2, 3, 4]
            }
        );
        let fork = LinuxSyscallRequest {
            number: 57,
            args: [0; 6],
            authority: CapabilityId::INVALID,
        };
        assert_eq!(
            broker(fork),
            BrokerDecision::ProcessPrimitive {
                operation: LinuxOp::Fork
            }
        );
    }

    #[test]
    fn rejects_interpreters_and_unknown_syscalls() {
        assert!(!is_static_elf_supported(Some(
            b"/lib64/ld-linux-x86-64.so.2"
        )));
        assert_eq!(translate(9999), Err(UnsupportedSyscall { number: 9999 }));
    }

    #[test]
    fn maps_memory_and_posix_primitives() {
        assert_eq!(
            translate(9).unwrap(),
            Translation {
                linux_number: 9,
                operation: LinuxOp::MMap,
                native: Some(Syscall::MMap),
            }
        );
        assert_eq!(
            translate(12).unwrap(),
            Translation {
                linux_number: 12,
                operation: LinuxOp::Brk,
                native: Some(Syscall::Brk),
            }
        );
        assert_eq!(
            translate(63).unwrap(),
            Translation {
                linux_number: 63,
                operation: LinuxOp::Uname,
                native: None,
            }
        );
        assert_eq!(
            translate(228).unwrap(),
            Translation {
                linux_number: 228,
                operation: LinuxOp::ClockGetTime,
                native: None,
            }
        );
        assert_eq!(
            translate(20).unwrap(),
            Translation {
                linux_number: 20,
                operation: LinuxOp::Writev,
                native: None,
            }
        );
        assert_eq!(
            translate(41).unwrap(),
            Translation {
                linux_number: 41,
                operation: LinuxOp::Socket,
                native: Some(Syscall::Socket),
            }
        );
    }

    #[test]
    fn parses_dynamic_elf_with_interpreter() {
        let mut image = [0u8; 200];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&3u16.to_le_bytes()); // ET_DYN
        image[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&2u16.to_le_bytes());

        // Phdr 0: PT_INTERP (kind=3)
        image[64..68].copy_from_slice(&3u32.to_le_bytes());
        image[72..80].copy_from_slice(&180u64.to_le_bytes());
        image[96..104].copy_from_slice(&18u64.to_le_bytes());

        // Phdr 1: PT_LOAD (kind=1)
        image[120..124].copy_from_slice(&1u32.to_le_bytes());
        image[124..128].copy_from_slice(&5u32.to_le_bytes());
        image[128..136].copy_from_slice(&0u64.to_le_bytes());
        image[136..144].copy_from_slice(&0x400000u64.to_le_bytes());
        image[152..160].copy_from_slice(&100u64.to_le_bytes());
        image[160..168].copy_from_slice(&100u64.to_le_bytes());

        let interp = b"/lib/ld-musl.so.1\0";
        image[180..180 + interp.len()].copy_from_slice(interp);

        let elf = StaticElf::parse(&image).unwrap();
        assert_eq!(elf.interpreter(&image), Some("/lib/ld-musl.so.1"));
    }

    #[test]
    fn broker_routes_mprotect_as_primitive() {
        let request = LinuxSyscallRequest {
            number: 10,
            args: [0x400000, 4096, 7, 0, 0, 0],
            authority: CapabilityId::INVALID,
        };
        assert_eq!(
            broker(request),
            BrokerDecision::ProcessPrimitive {
                operation: LinuxOp::MProtect
            }
        );
    }

    #[test]
    fn rejects_out_of_bounds_interpreter_segment() {
        let mut image = [0u8; 120];
        image[..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[16..18].copy_from_slice(&3u16.to_le_bytes());
        image[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());

        // Phdr 0: PT_INTERP with file_offset + file_size exceeding image length
        image[64..68].copy_from_slice(&3u32.to_le_bytes());
        image[72..80].copy_from_slice(&100u64.to_le_bytes());
        image[96..104].copy_from_slice(&500u64.to_le_bytes()); // out of bounds

        assert_eq!(StaticElf::parse(&image), Err(ElfError::InvalidProgramTable));
    }

    #[test]
    fn maps_signal_syscalls_and_verifies_structures() {
        // rt_sigaction (13) -> LinuxOp::RtSigAction, native Some(Syscall::SigAction)
        assert_eq!(
            translate(13).unwrap(),
            Translation {
                linux_number: 13,
                operation: LinuxOp::RtSigAction,
                native: Some(Syscall::SigAction),
            }
        );

        // rt_sigprocmask (14) -> LinuxOp::RtSigProcMask, native None
        assert_eq!(
            translate(14).unwrap(),
            Translation {
                linux_number: 14,
                operation: LinuxOp::RtSigProcMask,
                native: None,
            }
        );

        // rt_sigreturn (15) -> LinuxOp::RtSigReturn, native None
        assert_eq!(
            translate(15).unwrap(),
            Translation {
                linux_number: 15,
                operation: LinuxOp::RtSigReturn,
                native: None,
            }
        );

        // kill (62) -> LinuxOp::Kill, native Some(Syscall::Kill)
        assert_eq!(
            translate(62).unwrap(),
            Translation {
                linux_number: 62,
                operation: LinuxOp::Kill,
                native: Some(Syscall::Kill),
            }
        );

        // tkill (200) -> LinuxOp::TKill, native None
        assert_eq!(
            translate(200).unwrap(),
            Translation {
                linux_number: 200,
                operation: LinuxOp::TKill,
                native: None,
            }
        );

        // tgkill (234) -> LinuxOp::TgKill, native None
        assert_eq!(
            translate(234).unwrap(),
            Translation {
                linux_number: 234,
                operation: LinuxOp::TgKill,
                native: None,
            }
        );

        // Verify structure sizes
        assert_eq!(core::mem::size_of::<LinuxSigAction>(), 32);
        assert_eq!(core::mem::size_of::<SigAltStack>(), 24);
        assert_eq!(core::mem::size_of::<SigContext>(), 256);
        assert_eq!(core::mem::size_of::<SigInfo>(), 128);
        assert_eq!(core::mem::size_of::<UContext>(), 816);

        // Verify broker decision for signals
        let req_sigreturn = LinuxSyscallRequest {
            number: 15,
            args: [0; 6],
            authority: CapabilityId::INVALID,
        };
        assert_eq!(
            broker(req_sigreturn),
            BrokerDecision::ProcessPrimitive {
                operation: LinuxOp::RtSigReturn
            }
        );

        let req_tkill = LinuxSyscallRequest {
            number: 200,
            args: [1, 9, 0, 0, 0, 0],
            authority: CapabilityId::INVALID,
        };
        assert_eq!(
            broker(req_tkill),
            BrokerDecision::ProcessPrimitive {
                operation: LinuxOp::TKill
            }
        );

        let req_tgkill = LinuxSyscallRequest {
            number: 234,
            args: [1, 1, 15, 0, 0, 0],
            authority: CapabilityId::INVALID,
        };
        assert_eq!(
            broker(req_tgkill),
            BrokerDecision::ProcessPrimitive {
                operation: LinuxOp::TgKill
            }
        );
    }
}


