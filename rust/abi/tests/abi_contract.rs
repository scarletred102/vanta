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
fn errno_rejects_non_error_returns() {
    assert_eq!(Errno::from_return_value(0), None);
    assert_eq!(Errno::from_return_value(1), None);
    assert_eq!(Errno::from_return_value(isize::MIN), None);
}

#[test]
fn capability_boundaries_round_trip() {
    for (slot, generation) in [(0, 0), (42, 7), (u32::MAX, u32::MAX)] {
        let id = CapabilityId::from_parts(slot, generation);
        assert_eq!((id.slot(), id.generation()), (slot, generation));
    }
}

#[test]
fn syscall_numbers_are_frozen() {
    let vectors = [
        (Syscall::Read, 0x0001),
        (Syscall::Write, 0x0002),
        (Syscall::OpenAt, 0x0003),
        (Syscall::Close, 0x0004),
        (Syscall::Dup3, 0x0005),
        (Syscall::Pipe2, 0x0006),
        (Syscall::LSeek, 0x0007),
        (Syscall::FStat, 0x0008),
        (Syscall::GetDents, 0x0009),
        (Syscall::MkDirAt, 0x000A),
        (Syscall::UnlinkAt, 0x000B),
        (Syscall::RenameAt, 0x000C),
        (Syscall::TtyIoctl, 0x000D),
        (Syscall::SpawnVe, 0x0011),
        (Syscall::ExecVe, 0x0012),
        (Syscall::WaitPid, 0x0013),
        (Syscall::Exit, 0x0014),
        (Syscall::Kill, 0x0015),
        (Syscall::SigAction, 0x0016),
        (Syscall::Brk, 0x0017),
        (Syscall::MMap, 0x0018),
        (Syscall::MUnmap, 0x0019),
        (Syscall::GetPid, 0x001A),
        (Syscall::GetPpid, 0x001B),
        (Syscall::Yield, 0x001C),
        (Syscall::Socket, 0x0020),
        (Syscall::Connect, 0x0021),
    ];

    for (syscall, number) in vectors {
        assert_eq!(syscall.number(), number);
    }
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
