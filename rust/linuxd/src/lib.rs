#![no_std]

use vanta_abi::Syscall;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxOp {
    Read,
    Write,
    Open,
    Close,
    FStat,
    LSeek,
    MMap,
    MUnmap,
    Brk,
    Pipe,
    Dup2,
    GetPid,
    Fork,
    ExecVe,
    Exit,
    Wait4,
    Kill,
    RtSigAction,
    Unsupported(u64),
}

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
        2 => (LinuxOp::Open, Some(Syscall::OpenAt)),
        3 => (LinuxOp::Close, Some(Syscall::Close)),
        5 => (LinuxOp::FStat, Some(Syscall::FStat)),
        8 => (LinuxOp::LSeek, Some(Syscall::LSeek)),
        9 => (LinuxOp::MMap, Some(Syscall::MMap)),
        11 => (LinuxOp::MUnmap, Some(Syscall::MUnmap)),
        12 => (LinuxOp::Brk, Some(Syscall::Brk)),
        13 => (LinuxOp::RtSigAction, Some(Syscall::SigAction)),
        22 => (LinuxOp::Pipe, Some(Syscall::Pipe2)),
        33 => (LinuxOp::Dup2, Some(Syscall::Dup3)),
        39 => (LinuxOp::GetPid, Some(Syscall::GetPid)),
        56 => (LinuxOp::Fork, None),
        57 => (LinuxOp::Fork, None),
        59 => (LinuxOp::ExecVe, Some(Syscall::ExecVe)),
        60 => (LinuxOp::Exit, Some(Syscall::Exit)),
        61 => (LinuxOp::Wait4, Some(Syscall::WaitPid)),
        62 => (LinuxOp::Kill, Some(Syscall::Kill)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_static_cli_syscalls_to_native_operations() {
        assert_eq!(translate(0).unwrap().native, Some(Syscall::Read));
        assert_eq!(translate(1).unwrap().native, Some(Syscall::Write));
        assert_eq!(translate(59).unwrap().native, Some(Syscall::ExecVe));
        assert_eq!(translate(61).unwrap().native, Some(Syscall::WaitPid));
    }

    #[test]
    fn keeps_fork_explicit_until_native_semantics_exist() {
        let fork = translate(57).unwrap();
        assert_eq!(fork.operation, LinuxOp::Fork);
        assert_eq!(fork.native, None);
    }

    #[test]
    fn unsupported_syscalls_are_deterministic() {
        assert_eq!(translate(9999), Err(UnsupportedSyscall { number: 9999 }));
    }

    #[test]
    fn dynamic_interpreters_are_rejected_by_the_first_spike() {
        assert!(is_static_elf_supported(None));
        assert!(!is_static_elf_supported(Some(
            b"/lib64/ld-linux-x86-64.so.2"
        )));
    }
}
