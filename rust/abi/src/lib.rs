#![no_std]

use core::ops::{BitOr, BitOrAssign};

pub const ABI_VERSION: u32 = 0;
pub const MAX_GROUPS: usize = 8;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeatureSet(u64);

impl FeatureSet {
    pub const EMPTY: Self = Self(0);

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn unknown_mandatory_bits(self, required: Self) -> Self {
        Self(required.0 & !self.0)
    }
}

pub const FEATURE_NATIVE_TERMINAL: FeatureSet = FeatureSet(1 << 0);
pub const FEATURE_REDOXFS_ROOT: FeatureSet = FeatureSet(1 << 1);
pub const FEATURE_PIPE_WAKEUP: FeatureSet = FeatureSet(1 << 2);
pub const FEATURE_C_ABI_BOOTSTRAP: FeatureSet = FeatureSet(1 << 3);

pub const SUPPORTED_FEATURES: FeatureSet = FeatureSet(
    FEATURE_NATIVE_TERMINAL.bits()
        | FEATURE_REDOXFS_ROOT.bits()
        | FEATURE_PIPE_WAKEUP.bits()
        | FEATURE_C_ABI_BOOTSTRAP.bits(),
);

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbiInfo {
    pub abi_version: u32,
    pub struct_size: u32,
    pub features: u64,
}

impl AbiInfo {
    pub const fn current() -> Self {
        Self {
            abi_version: ABI_VERSION,
            struct_size: core::mem::size_of::<Self>() as u32,
            features: SUPPORTED_FEATURES.bits(),
        }
    }
}

pub const MAX_DIRECTORY_NAME: usize = 256;

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryRecord {
    pub inode: u64,
    pub file_type: u8,
    pub name_len: u8,
    pub record_len: u16,
    pub name: [u8; MAX_DIRECTORY_NAME],
}

impl DirectoryRecord {
    pub const fn empty(inode: u64, file_type: u8) -> Self {
        Self {
            inode,
            file_type,
            name_len: 0,
            record_len: core::mem::size_of::<Self>() as u16,
            name: [0; MAX_DIRECTORY_NAME],
        }
    }

    pub fn set_name(&mut self, name: &[u8]) {
        let length = core::cmp::min(name.len(), MAX_DIRECTORY_NAME);
        self.name[..length].copy_from_slice(&name[..length]);
        self.name_len = length as u8;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignalAction {
    pub handler: u64,
    pub flags: u64,
}

impl SignalAction {
    pub const fn default() -> Self {
        Self {
            handler: 0,
            flags: 0,
        }
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Syscall {
    Read = 0x0001,
    Write = 0x0002,
    OpenAt = 0x0003,
    Close = 0x0004,
    Dup3 = 0x0005,
    Pipe2 = 0x0006,
    LSeek = 0x0007,
    FStat = 0x0008,
    GetDents = 0x0009,
    MkDirAt = 0x000A,
    UnlinkAt = 0x000B,
    RenameAt = 0x000C,
    TtyIoctl = 0x000D,
    GetAbiInfo = 0x000E,
    SpawnVe = 0x0011,
    ExecVe = 0x0012,
    WaitPid = 0x0013,
    Exit = 0x0014,
    Kill = 0x0015,
    SigAction = 0x0016,
    Brk = 0x0017,
    MMap = 0x0018,
    MUnmap = 0x0019,
    GetPid = 0x001A,
    GetPpid = 0x001B,
    Yield = 0x001C,
    Socket = 0x0020,
    Connect = 0x0021,
}

impl Syscall {
    pub const fn number(self) -> u16 {
        self as u16
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Errno(pub i32);

impl Errno {
    pub const IO: Self = Self(5);
    pub const BADF: Self = Self(9);
    pub const INVAL: Self = Self(22);
    pub const NOSYS: Self = Self(38);

    pub const fn into_return_value(self) -> isize {
        -(self.0 as isize)
    }

    pub const fn from_return_value(value: isize) -> Option<Self> {
        if value >= 0 {
            return None;
        }

        let magnitude = match value.checked_neg() {
            Some(magnitude) => magnitude,
            None => return None,
        };
        if magnitude > i32::MAX as isize {
            return None;
        }

        Some(Self(magnitude as i32))
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rights(u32);

impl Rights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const TRANSFER: Self = Self(1 << 3);
    pub const MOUNT: Self = Self(1 << 4);
    pub const DEVICE: Self = Self(1 << 5);
    pub const PROCESS_ADMIN: Self = Self(1 << 6);
    pub const CONNECT: Self = Self(1 << 7);

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

impl BitOr for Rights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityId(u64);

impl CapabilityId {
    pub const INVALID: Self = Self(0);

    pub const fn from_parts(slot: u32, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    pub const fn slot(self) -> u32 {
        self.0 as u32
    }

    pub const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn is_invalid(self) -> bool {
        self.0 == 0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub groups: [u32; MAX_GROUPS],
    pub group_count: u8,
    pub umask: u16,
}

impl Credentials {
    pub const fn root() -> Self {
        Self {
            uid: 0,
            gid: 0,
            groups: [0; MAX_GROUPS],
            group_count: 1,
            umask: 0o022,
        }
    }

    pub const fn vanta() -> Self {
        Self {
            uid: 1000,
            gid: 1000,
            groups: [1000, 0, 0, 0, 0, 0, 0, 0],
            group_count: 1,
            umask: 0o022,
        }
    }

    pub const fn is_root(&self) -> bool {
        self.uid == 0
    }
}
