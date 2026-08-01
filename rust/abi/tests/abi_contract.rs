use core::mem::{align_of, size_of};

use vanta_abi::{
    CapabilityId, Credentials, DirectoryRecord, Errno, FeatureSet, Rights, SignalAction, Syscall,
    ABI_VERSION,
    FEATURE_NATIVE_TERMINAL, FEATURE_REDOXFS_ROOT, SUPPORTED_FEATURES,
};

#[test]
fn feature_vectors_and_mandatory_bits_are_stable() {
    assert_eq!(FEATURE_NATIVE_TERMINAL.bits(), 1 << 0);
    assert_eq!(FEATURE_REDOXFS_ROOT.bits(), 1 << 1);
    assert!(SUPPORTED_FEATURES.contains(FEATURE_NATIVE_TERMINAL));
    assert_eq!(
        FeatureSet::EMPTY.unknown_mandatory_bits(FEATURE_NATIVE_TERMINAL),
        FEATURE_NATIVE_TERMINAL
    );
    assert_eq!(
        SUPPORTED_FEATURES.unknown_mandatory_bits(SUPPORTED_FEATURES),
        FeatureSet::EMPTY
    );
}

#[test]
fn wire_layouts_are_stable() {
    assert_eq!(size_of::<SignalAction>(), 16);
    assert_eq!(align_of::<SignalAction>(), 8);
    assert_eq!(size_of::<Credentials>(), 44);
    assert_eq!(align_of::<Credentials>(), 4);
    assert_eq!(size_of::<DirectoryRecord>(), 272);
    assert_eq!(align_of::<DirectoryRecord>(), 8);
}

#[test]
fn directory_record_preserves_bounded_name_data() {
    let mut record = DirectoryRecord::empty(42, 8);
    record.set_name(b"hello");
    assert_eq!(record.inode, 42);
    assert_eq!(record.name_len, 5);
    assert_eq!(&record.name[..5], b"hello");
}

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
