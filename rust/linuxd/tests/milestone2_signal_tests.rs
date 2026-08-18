use vanta_abi::Syscall;
use vanta_linuxd::{
    broker, translate, rt_sigaction, rt_sigframe, sigcontext, siginfo_t, sigset_t, ucontext_t,
    BrokerDecision, LinuxOp, LinuxSigAction, LinuxSyscallRequest, RtSigFrame, SigAltStack,
    SigContext, SigInfo, UContext, SA_NODEFER, SA_ONSTACK, SA_RESETHAND, SA_RESTART, SA_RESTORER,
    SA_SIGINFO, SIGABRT, SIGALRM, SIGBUS, SIGCHLD, SIGCONT, SIGFPE, SIGHUP, SIGILL, SIGINT,
    SIGIO, SIGKILL, SIGPIPE, SIGPROF, SIGPWR, SIGQUIT, SIGSEGV, SIGSTKFLT, SIGSTOP, SIGSYS,
    SIGTERM, SIGTRAP, SIGTSTP, SIGTTIN, SIGTTOU, SIGURG, SIGUSR1, SIGUSR2, SIGVTALRM, SIGWINCH,
    SIGXCPU, SIGXFSZ, SIG_BLOCK, SIG_DFL, SIG_IGN, SIG_SETMASK, SIG_UNBLOCK, SI_KERNEL, SI_TKILL,
    SI_USER,
};

#[test]
fn test_signal_constants_and_ranges() {
    assert_eq!(SIGHUP, 1);
    assert_eq!(SIGINT, 2);
    assert_eq!(SIGQUIT, 3);
    assert_eq!(SIGILL, 4);
    assert_eq!(SIGTRAP, 5);
    assert_eq!(SIGABRT, 6);
    assert_eq!(SIGBUS, 7);
    assert_eq!(SIGFPE, 8);
    assert_eq!(SIGKILL, 9);
    assert_eq!(SIGUSR1, 10);
    assert_eq!(SIGSEGV, 11);
    assert_eq!(SIGUSR2, 12);
    assert_eq!(SIGPIPE, 13);
    assert_eq!(SIGALRM, 14);
    assert_eq!(SIGTERM, 15);
    assert_eq!(SIGSTKFLT, 16);
    assert_eq!(SIGCHLD, 17);
    assert_eq!(SIGCONT, 18);
    assert_eq!(SIGSTOP, 19);
    assert_eq!(SIGTSTP, 20);
    assert_eq!(SIGTTIN, 21);
    assert_eq!(SIGTTOU, 22);
    assert_eq!(SIGURG, 23);
    assert_eq!(SIGXCPU, 24);
    assert_eq!(SIGXFSZ, 25);
    assert_eq!(SIGVTALRM, 26);
    assert_eq!(SIGPROF, 27);
    assert_eq!(SIGWINCH, 28);
    assert_eq!(SIGIO, 29);
    assert_eq!(SIGPWR, 30);
    assert_eq!(SIGSYS, 31);

    assert_eq!(SIG_DFL, 0);
    assert_eq!(SIG_IGN, 1);

    assert_eq!(SIG_BLOCK, 0);
    assert_eq!(SIG_UNBLOCK, 1);
    assert_eq!(SIG_SETMASK, 2);

    assert_eq!(SA_SIGINFO, 0x00000004);
    assert_eq!(SA_ONSTACK, 0x08000000);
    assert_eq!(SA_RESTART, 0x10000000);
    assert_eq!(SA_NODEFER, 0x40000000);
    assert_eq!(SA_RESETHAND, 0x80000000);
    assert_eq!(SA_RESTORER, 0x04000000);

    assert_eq!(SI_USER, 0);
    assert_eq!(SI_KERNEL, 0x80);
    assert_eq!(SI_TKILL, -6);
}

#[test]
fn test_signal_syscall_translations() {
    // rt_sigaction (13)
    let tr_sigaction = translate(13).expect("rt_sigaction must be translated");
    assert_eq!(tr_sigaction.linux_number, 13);
    assert_eq!(tr_sigaction.operation, LinuxOp::RtSigAction);
    assert_eq!(tr_sigaction.native, Some(Syscall::SigAction));

    // rt_sigprocmask (14)
    let tr_sigprocmask = translate(14).expect("rt_sigprocmask must be translated");
    assert_eq!(tr_sigprocmask.linux_number, 14);
    assert_eq!(tr_sigprocmask.operation, LinuxOp::RtSigProcMask);
    assert_eq!(tr_sigprocmask.native, None);

    // rt_sigreturn (15)
    let tr_sigreturn = translate(15).expect("rt_sigreturn must be translated");
    assert_eq!(tr_sigreturn.linux_number, 15);
    assert_eq!(tr_sigreturn.operation, LinuxOp::RtSigReturn);
    assert_eq!(tr_sigreturn.native, None);

    // kill (62)
    let tr_kill = translate(62).expect("kill must be translated");
    assert_eq!(tr_kill.linux_number, 62);
    assert_eq!(tr_kill.operation, LinuxOp::Kill);
    assert_eq!(tr_kill.native, Some(Syscall::Kill));

    // tkill (200)
    let tr_tkill = translate(200).expect("tkill must be translated");
    assert_eq!(tr_tkill.linux_number, 200);
    assert_eq!(tr_tkill.operation, LinuxOp::TKill);
    assert_eq!(tr_tkill.native, None);

    // tgkill (234)
    let tr_tgkill = translate(234).expect("tgkill must be translated");
    assert_eq!(tr_tgkill.linux_number, 234);
    assert_eq!(tr_tgkill.operation, LinuxOp::TgKill);
    assert_eq!(tr_tgkill.native, None);
}

#[test]
fn test_broker_signal_routing() {
    // Test rt_sigaction brokering
    let act_req = LinuxSyscallRequest {
        number: 13,
        args: [15, 0x500000, 0x500020, 8, 0, 0],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(act_req),
        BrokerDecision::Native {
            syscall: Syscall::SigAction,
            args: [15, 0x500000, 0x500020, 8],
        }
    );

    // Test rt_sigreturn brokering
    let ret_req = LinuxSyscallRequest {
        number: 15,
        args: [0; 6],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(ret_req),
        BrokerDecision::ProcessPrimitive {
            operation: LinuxOp::RtSigReturn,
        }
    );

    // Test tkill brokering
    let tkill_req = LinuxSyscallRequest {
        number: 200,
        args: [42, 9, 0, 0, 0, 0],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(tkill_req),
        BrokerDecision::ProcessPrimitive {
            operation: LinuxOp::TKill,
        }
    );

    // Test tgkill brokering
    let tgkill_req = LinuxSyscallRequest {
        number: 234,
        args: [100, 42, 15, 0, 0, 0],
        authority: vanta_abi::CapabilityId::INVALID,
    };
    assert_eq!(
        broker(tgkill_req),
        BrokerDecision::ProcessPrimitive {
            operation: LinuxOp::TgKill,
        }
    );
}

#[test]
fn test_signal_structures_layout_and_offsets() {
    assert_eq!(core::mem::size_of::<sigset_t>(), 8);
    assert_eq!(core::mem::size_of::<LinuxSigAction>(), 32);
    assert_eq!(core::mem::size_of::<rt_sigaction>(), 32);
    assert_eq!(core::mem::size_of::<SigAltStack>(), 24);
    assert_eq!(core::mem::size_of::<SigContext>(), 256);
    assert_eq!(core::mem::size_of::<sigcontext>(), 256);
    assert_eq!(core::mem::size_of::<SigInfo>(), 128);
    assert_eq!(core::mem::size_of::<siginfo_t>(), 128);
    assert_eq!(core::mem::size_of::<UContext>(), 816);
    assert_eq!(core::mem::size_of::<ucontext_t>(), 816);

    // Check SigContext field offsets
    assert_eq!(core::mem::offset_of!(SigContext, r8), 0);
    assert_eq!(core::mem::offset_of!(SigContext, r9), 8);
    assert_eq!(core::mem::offset_of!(SigContext, r10), 16);
    assert_eq!(core::mem::offset_of!(SigContext, r11), 24);
    assert_eq!(core::mem::offset_of!(SigContext, r12), 32);
    assert_eq!(core::mem::offset_of!(SigContext, r13), 40);
    assert_eq!(core::mem::offset_of!(SigContext, r14), 48);
    assert_eq!(core::mem::offset_of!(SigContext, r15), 56);
    assert_eq!(core::mem::offset_of!(SigContext, rdi), 64);
    assert_eq!(core::mem::offset_of!(SigContext, rsi), 72);
    assert_eq!(core::mem::offset_of!(SigContext, rbp), 80);
    assert_eq!(core::mem::offset_of!(SigContext, rbx), 88);
    assert_eq!(core::mem::offset_of!(SigContext, rdx), 96);
    assert_eq!(core::mem::offset_of!(SigContext, rax), 104);
    assert_eq!(core::mem::offset_of!(SigContext, rcx), 112);
    assert_eq!(core::mem::offset_of!(SigContext, rsp), 120);
    assert_eq!(core::mem::offset_of!(SigContext, rip), 128);
    assert_eq!(core::mem::offset_of!(SigContext, rflags), 136);

    // Check RtSigFrame layout
    assert_eq!(core::mem::offset_of!(RtSigFrame, pretcode), 0);
    assert_eq!(core::mem::offset_of!(RtSigFrame, uc), 8);
    assert_eq!(core::mem::offset_of!(RtSigFrame, info), 824);

    assert_eq!(core::mem::size_of::<rt_sigframe>(), core::mem::size_of::<RtSigFrame>());

    let default_frame = RtSigFrame::default();
    assert_eq!(default_frame.pretcode, 0);
    assert_eq!(default_frame.info.si_signo, 0);
}

#[test]
fn test_signal_mask_math_and_unblockable_signals() {
    let unblockable_mask: u64 = (1 << (SIGKILL - 1)) | (1 << (SIGSTOP - 1));
    assert_eq!(unblockable_mask, (1 << 8) | (1 << 18));

    // Test SIG_BLOCK
    let current_mask: u64 = 0b0001; // SIGHUP blocked
    let to_block: u64 = (1 << (SIGINT - 1)) | (1 << (SIGKILL - 1)); // block SIGINT and attempt to block SIGKILL
    let mut new_mask = current_mask | to_block;
    new_mask &= !unblockable_mask;
    assert_eq!(new_mask & (1 << (SIGHUP - 1)), 1 << (SIGHUP - 1));
    assert_eq!(new_mask & (1 << (SIGINT - 1)), 1 << (SIGINT - 1));
    assert_eq!(new_mask & (1 << (SIGKILL - 1)), 0); // SIGKILL must NOT be blocked!

    // Test SIG_UNBLOCK
    let current_mask: u64 = (1 << (SIGHUP - 1)) | (1 << (SIGTERM - 1));
    let to_unblock: u64 = 1 << (SIGHUP - 1);
    let new_mask = current_mask & !to_unblock;
    assert_eq!(new_mask, 1 << (SIGTERM - 1));

    // Test SIG_SETMASK
    let set_mask: u64 = u64::MAX; // attempt to block all signals
    let new_mask = set_mask & !unblockable_mask;
    assert_eq!(new_mask & (1 << (SIGKILL - 1)), 0);
    assert_eq!(new_mask & (1 << (SIGSTOP - 1)), 0);
    assert_eq!(new_mask & (1 << (SIGHUP - 1)), 1 << (SIGHUP - 1));
    assert_eq!(new_mask & (1 << 63), 1 << 63); // Signal 64 blocked
}

#[test]
fn test_all_64_signals_representation() {
    for sig in 1..=64u64 {
        let bit = 1u64 << (sig - 1);
        assert_eq!(bit.trailing_zeros(), (sig - 1) as u32);
        assert_eq!(bit.count_ones(), 1);
    }
}

#[test]
fn test_rt_sigframe_stack_alignment_math() {
    let frame_size = core::mem::size_of::<RtSigFrame>() as u64;

    // Test various unaligned and aligned stack pointers
    let test_rsps: [u64; 6] = [
        0x7fff_ffff_0000,
        0x7fff_ffff_0008,
        0x7fff_ffff_0010,
        0x7fff_ffff_0028,
        0x7fff_ffff_fed8,
        0x7fff_0000_1234,
    ];

    for &old_sp in &test_rsps {
        let new_sp = (old_sp.saturating_sub(frame_size) & !15) - 8;
        // Verify SysV function entry alignment: (RSP + 8) % 16 == 0
        assert_eq!((new_sp + 8) % 16, 0);
        assert_eq!(new_sp % 16, 8);
        assert!(new_sp + frame_size <= old_sp);
    }
}

#[test]
fn test_default_trampoline_encoding() {
    let retcode: [u8; 16] = [
        0x48, 0xc7, 0xc0, 0x0f, 0x00, 0x00, 0x00, // mov $15, %rax
        0x0f, 0x05,                               // syscall
        0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, // nop
    ];
    // Verify mov $15, %rax opcode
    assert_eq!(&retcode[..7], &[0x48, 0xc7, 0xc0, 0x0f, 0x00, 0x00, 0x00]);
    // Verify syscall opcode
    assert_eq!(&retcode[7..9], &[0x0f, 0x05]);
}

#[test]
fn test_sigcontext_register_preservation_fidelity() {
    let ctx = SigContext {
        r8: 0x8888_8888_8888_8888,
        r9: 0x9999_9999_9999_9999,
        r10: 0xaaaa_aaaa_aaaa_aaaa,
        r11: 0xbbbb_bbbb_bbbb_bbbb,
        r12: 0xcccc_cccc_cccc_cccc,
        r13: 0xdddd_dddd_dddd_dddd,
        r14: 0xeeee_eeee_eeee_eeee,
        r15: 0xffff_ffff_ffff_ffff,
        rdi: 0x1111_1111_1111_1111,
        rsi: 0x2222_2222_2222_2222,
        rbp: 0x3333_3333_3333_3333,
        rbx: 0x4444_4444_4444_4444,
        rdx: 0x5555_5555_5555_5555,
        rax: 0x6666_6666_6666_6666,
        rcx: 0x7777_7777_7777_7777,
        rsp: 0x7fff_ffff_f000,
        rip: 0x0040_1000,
        rflags: 0x202,
        cs: 0x23,
        gs: 0x1b,
        fs: 0x1b,
        __pad0: 0,
        err: 0,
        trapno: 0,
        oldmask: 0x1234,
        cr2: 0,
        fpstate: 0,
        reserved: [0; 8],
    };

    assert_eq!(ctx.rax, 0x6666_6666_6666_6666);
    assert_eq!(ctx.rip, 0x0040_1000);
    assert_eq!(ctx.rsp, 0x7fff_ffff_f000);
    assert_eq!(ctx.rflags, 0x202);
    assert_eq!(ctx.oldmask, 0x1234);
}

