use vanta_linuxd::*;
use vanta_abi::Syscall;

#[test]
fn test_milestone6_syscall_translations() {
    assert_eq!(translate(17).unwrap().operation, LinuxOp::PRead64);
    assert_eq!(translate(18).unwrap().operation, LinuxOp::PWrite64);
    assert_eq!(translate(40).unwrap().operation, LinuxOp::SendFile);
    assert_eq!(translate(76).unwrap().operation, LinuxOp::Truncate);
    assert_eq!(translate(77).unwrap().operation, LinuxOp::FTruncate);
    assert_eq!(translate(83).unwrap().operation, LinuxOp::MkDir);
    assert_eq!(translate(83).unwrap().native, None);
    assert_eq!(translate(87).unwrap().operation, LinuxOp::Unlink);
    assert_eq!(translate(87).unwrap().native, None);
    assert_eq!(translate(88).unwrap().operation, LinuxOp::SymLink);
    assert_eq!(translate(89).unwrap().operation, LinuxOp::ReadLink);
    assert_eq!(translate(90).unwrap().operation, LinuxOp::Chmod);
    assert_eq!(translate(92).unwrap().operation, LinuxOp::Chown);
    assert_eq!(translate(127).unwrap().operation, LinuxOp::RtSigPending);
    assert_eq!(translate(130).unwrap().operation, LinuxOp::RtSigSuspend);
    assert_eq!(translate(131).unwrap().operation, LinuxOp::SigAltStack);
    assert_eq!(translate(137).unwrap().operation, LinuxOp::StatFs);
    assert_eq!(translate(138).unwrap().operation, LinuxOp::FStatFs);
    assert_eq!(translate(247).unwrap().operation, LinuxOp::WaitId);
    assert_eq!(translate(258).unwrap().operation, LinuxOp::MkDirAt);
    assert_eq!(translate(260).unwrap().operation, LinuxOp::FChownAt);
    assert_eq!(translate(263).unwrap().operation, LinuxOp::UnlinkAt);
    assert_eq!(translate(266).unwrap().operation, LinuxOp::SymLinkAt);
    assert_eq!(translate(267).unwrap().operation, LinuxOp::ReadLinkAt);
    assert_eq!(translate(268).unwrap().operation, LinuxOp::FChmodAt);
    assert_eq!(translate(280).unwrap().operation, LinuxOp::UtimensAt);
    assert_eq!(translate(289).unwrap().operation, LinuxOp::SignalFd4);
}

#[test]
fn test_milestone6_structure_layouts_and_constants() {
    assert_eq!(core::mem::size_of::<signalfd_siginfo>(), 128);
    assert_eq!(core::mem::size_of::<statfs>(), 120);
    assert_eq!(core::mem::size_of::<SigAltStack>(), 24);

    assert_eq!(SS_ONSTACK, 1);
    assert_eq!(SS_DISABLE, 2);
    assert_eq!(MINSIGSTKSZ, 2048);

    assert_eq!(PR_SET_PDEATHSIG, 1);
    assert_eq!(PR_GET_PDEATHSIG, 2);
    assert_eq!(PR_SET_NAME, 15);
    assert_eq!(PR_GET_NAME, 16);

    assert_eq!(RLIMIT_NOFILE, 7);
    assert_eq!(RLIMIT_STACK, 3);
    assert_eq!(RLIMIT_AS, 9);

    assert_eq!(P_ALL, 0);
    assert_eq!(P_PID, 1);
    assert_eq!(P_PGID, 2);
    assert_eq!(WNOHANG, 1);
    assert_eq!(WEXITED, 4);
    assert_eq!(CLD_EXITED, 1);
}
