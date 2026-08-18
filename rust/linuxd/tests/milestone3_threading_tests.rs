use vanta_linuxd::{
    broker, translate, clone_args, BrokerDecision, LinuxOp, LinuxSyscallRequest, ARCH_GET_FS,
    ARCH_GET_GS, ARCH_SET_FS, ARCH_SET_GS, CLONE_CHILD_CLEARTID, CLONE_CHILD_SETTID, CLONE_FILES,
    CLONE_FS, CLONE_PARENT_SETTID, CLONE_SETTLS, CLONE_SIGHAND, CLONE_THREAD, CLONE_VM,
    FUTEX_BITSET_MATCH_ANY, FUTEX_CLOCK_REALTIME, FUTEX_CMP_REQUEUE, FUTEX_FD, FUTEX_PRIVATE_FLAG,
    FUTEX_REQUEUE, FUTEX_WAIT, FUTEX_WAIT_BITSET, FUTEX_WAKE, FUTEX_WAKE_BITSET, FUTEX_WAKE_OP,
    FUTEX_CMD_MASK,
};

#[test]
fn test_clone_flags_constants() {
    assert_eq!(CLONE_VM, 0x00000100);
    assert_eq!(CLONE_FS, 0x00000200);
    assert_eq!(CLONE_FILES, 0x00000400);
    assert_eq!(CLONE_SIGHAND, 0x00000800);
    assert_eq!(CLONE_THREAD, 0x00010000);
    assert_eq!(CLONE_SETTLS, 0x00080000);
    assert_eq!(CLONE_PARENT_SETTID, 0x00100000);
    assert_eq!(CLONE_CHILD_CLEARTID, 0x00200000);
    assert_eq!(CLONE_CHILD_SETTID, 0x01000000);
}

#[test]
fn test_arch_prctl_constants() {
    assert_eq!(ARCH_SET_GS, 0x1001);
    assert_eq!(ARCH_SET_FS, 0x1002);
    assert_eq!(ARCH_GET_FS, 0x1003);
    assert_eq!(ARCH_GET_GS, 0x1004);
}

#[test]
fn test_futex_constants_and_masks() {
    assert_eq!(FUTEX_WAIT, 0);
    assert_eq!(FUTEX_WAKE, 1);
    assert_eq!(FUTEX_FD, 2);
    assert_eq!(FUTEX_REQUEUE, 3);
    assert_eq!(FUTEX_CMP_REQUEUE, 4);
    assert_eq!(FUTEX_WAKE_OP, 5);
    assert_eq!(FUTEX_WAIT_BITSET, 9);
    assert_eq!(FUTEX_WAKE_BITSET, 10);

    assert_eq!(FUTEX_PRIVATE_FLAG, 128);
    assert_eq!(FUTEX_CLOCK_REALTIME, 256);
    assert_eq!(FUTEX_BITSET_MATCH_ANY, 0xffff_ffff);

    // Verify masking of flags
    assert_eq!(
        (FUTEX_WAIT | FUTEX_PRIVATE_FLAG) & FUTEX_CMD_MASK,
        FUTEX_WAIT
    );
    assert_eq!(
        (FUTEX_WAKE | FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME) & FUTEX_CMD_MASK,
        FUTEX_WAKE
    );
    assert_eq!(
        (FUTEX_WAIT_BITSET | FUTEX_PRIVATE_FLAG) & FUTEX_CMD_MASK,
        FUTEX_WAIT_BITSET
    );
    assert_eq!(
        (FUTEX_WAKE_BITSET | FUTEX_PRIVATE_FLAG) & FUTEX_CMD_MASK,
        FUTEX_WAKE_BITSET
    );
}

#[test]
fn test_clone_args_layout() {
    assert_eq!(core::mem::size_of::<clone_args>(), 88);
    let cl = clone_args::default();
    assert_eq!(cl.flags, 0);
    assert_eq!(cl.stack, 0);
    assert_eq!(cl.tls, 0);
}

#[test]
fn test_threading_and_futex_syscall_translations() {
    // Clone (56)
    let tr_clone = translate(56).expect("clone must be translated");
    assert_eq!(tr_clone.linux_number, 56);
    assert_eq!(tr_clone.operation, LinuxOp::Clone);
    assert_eq!(tr_clone.native, None);

    // Clone3 (435)
    let tr_clone3 = translate(435).expect("clone3 must be translated");
    assert_eq!(tr_clone3.linux_number, 435);
    assert_eq!(tr_clone3.operation, LinuxOp::Clone3);
    assert_eq!(tr_clone3.native, None);

    // Futex (202)
    let tr_futex = translate(202).expect("futex must be translated");
    assert_eq!(tr_futex.linux_number, 202);
    assert_eq!(tr_futex.operation, LinuxOp::Futex);
    assert_eq!(tr_futex.native, None);

    // SetTidAddress (218)
    let tr_settid = translate(218).expect("set_tid_address must be translated");
    assert_eq!(tr_settid.linux_number, 218);
    assert_eq!(tr_settid.operation, LinuxOp::SetTidAddress);
    assert_eq!(tr_settid.native, None);

    // GetTid (186)
    let tr_gettid = translate(186).expect("gettid must be translated");
    assert_eq!(tr_gettid.linux_number, 186);
    assert_eq!(tr_gettid.operation, LinuxOp::GetTid);
    assert_eq!(tr_gettid.native, None);

    // ArchPrctl (158)
    let tr_arch = translate(158).expect("arch_prctl must be translated");
    assert_eq!(tr_arch.linux_number, 158);
    assert_eq!(tr_arch.operation, LinuxOp::ArchPrctl);
    assert_eq!(tr_arch.native, None);

    // Wait4 (61)
    let tr_wait4 = translate(61).expect("wait4 must be translated");
    assert_eq!(tr_wait4.linux_number, 61);
    assert_eq!(tr_wait4.operation, LinuxOp::Wait4);
    assert_eq!(tr_wait4.native, None);
}

#[test]
fn test_broker_routing_for_threading_and_futex() {
    let clone_req = LinuxSyscallRequest {
        number: 56,
        args: [CLONE_VM | CLONE_THREAD, 0x7fff_ffff_e000, 0x1000, 0x2000, 0x3000, 0],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(clone_req),
        BrokerDecision::ProcessPrimitive {
            operation: LinuxOp::Clone,
        }
    );

    let futex_req = LinuxSyscallRequest {
        number: 202,
        args: [0x500000, (FUTEX_WAIT | FUTEX_PRIVATE_FLAG) as u64, 42, 0, 0, 0],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(futex_req),
        BrokerDecision::ProcessPrimitive {
            operation: LinuxOp::Futex,
        }
    );

    let gettid_req = LinuxSyscallRequest {
        number: 186,
        args: [0; 6],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(gettid_req),
        BrokerDecision::ProcessPrimitive {
            operation: LinuxOp::GetTid,
        }
    );

    let wait4_req = LinuxSyscallRequest {
        number: 61,
        args: [u64::MAX, 0x600000, 0, 0, 0, 0],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(wait4_req),
        BrokerDecision::ProcessPrimitive {
            operation: LinuxOp::Wait4,
        }
    );
}
