use vanta_abi::{CapabilityId, Credentials, Errno, Rights, Syscall, ABI_VERSION};

#[test]
fn abi_v0_uses_vanta_owned_numbers() {
    assert_eq!(ABI_VERSION, 0);
    assert_eq!(Syscall::Read.number(), 0x0001);
    assert_eq!(Syscall::OpenAt.number(), 0x0003);
    assert_eq!(Syscall::SpawnVe.number(), 0x0011);
    assert_ne!(Syscall::GetPid.number(), 39);
}

#[test]
fn abi_v0_reserves_scheduler_and_network_operations() {
    assert_eq!(Syscall::Yield.number(), 0x001C);
    assert_eq!(Syscall::Socket.number(), 0x0020);
    assert_eq!(Syscall::Connect.number(), 0x0021);
}

#[test]
fn errno_round_trip_uses_negative_return_values() {
    let encoded = Errno::IO.into_return_value();

    assert_eq!(Errno::from_return_value(encoded), Some(Errno::IO));
    assert_eq!(Errno::from_return_value(23), None);
}

#[test]
fn credentials_keep_root_and_vanta_separate() {
    let root = Credentials::root();
    let vanta = Credentials::vanta();

    assert!(root.is_root());
    assert!(!vanta.is_root());
    assert_eq!(vanta.uid, 1000);
    assert_eq!(vanta.gid, 1000);
}

#[test]
fn rights_are_composable_without_granting_unrelated_authority() {
    let file_rights = Rights::READ | Rights::WRITE;

    assert!(file_rights.contains(Rights::READ));
    assert!(file_rights.contains(Rights::WRITE));
    assert!(!file_rights.contains(Rights::MOUNT));
}

#[test]
fn capability_ids_preserve_slot_and_generation() {
    let capability = CapabilityId::from_parts(42, 7);

    assert_eq!(capability.slot(), 42);
    assert_eq!(capability.generation(), 7);
    assert!(!capability.is_invalid());
    assert!(CapabilityId::INVALID.is_invalid());
}
