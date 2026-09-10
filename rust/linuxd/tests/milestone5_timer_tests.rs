use vanta_linuxd::{
    broker, translate, BrokerDecision, LinuxOp, LinuxSyscallRequest, Translation,
};

#[test]
fn test_timer_syscall_translations() {
    // sys_nanosleep = 35
    assert_eq!(
        translate(35).unwrap(),
        Translation {
            linux_number: 35,
            operation: LinuxOp::Nanosleep,
            native: None,
        }
    );

    // sys_getitimer = 36
    assert_eq!(
        translate(36).unwrap(),
        Translation {
            linux_number: 36,
            operation: LinuxOp::GetITimer,
            native: None,
        }
    );

    // sys_setitimer = 38
    assert_eq!(
        translate(38).unwrap(),
        Translation {
            linux_number: 38,
            operation: LinuxOp::SetITimer,
            native: None,
        }
    );

    // sys_clock_settime = 227
    assert_eq!(
        translate(227).unwrap(),
        Translation {
            linux_number: 227,
            operation: LinuxOp::ClockSetTime,
            native: None,
        }
    );

    // sys_clock_gettime = 228
    assert_eq!(
        translate(228).unwrap(),
        Translation {
            linux_number: 228,
            operation: LinuxOp::ClockGetTime,
            native: None,
        }
    );

    // sys_clock_getres = 229
    assert_eq!(
        translate(229).unwrap(),
        Translation {
            linux_number: 229,
            operation: LinuxOp::ClockGetRes,
            native: None,
        }
    );
}

#[test]
fn test_broker_routing_for_timer_syscalls() {
    let numbers = [35, 36, 38, 227, 228, 229];
    let expected_ops = [
        LinuxOp::Nanosleep,
        LinuxOp::GetITimer,
        LinuxOp::SetITimer,
        LinuxOp::ClockSetTime,
        LinuxOp::ClockGetTime,
        LinuxOp::ClockGetRes,
    ];

    for (num, op) in numbers.iter().zip(expected_ops.iter()) {
        let req = LinuxSyscallRequest {
            number: *num,
            args: [0; 6],
            authority: vanta_abi::CapabilityId::INVALID,
        };
        assert_eq!(
            broker(req),
            BrokerDecision::ProcessPrimitive {
                operation: *op,
            }
        );
    }
}
