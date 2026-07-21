// VantaOS — Linux Personality IPC layout
// Shared between kernel and personality server userspace.

pub const LINUX_PERSONALITY_SHM_VIRT: u64 = 0x6000_0000;
pub const PSERVER_SHM_VIRT: u64 = 0x7000_0000;
pub const MSG_PERSONALITY_SETUP: u32 = 0x30;

pub const SyscallShmBlock = extern struct {
    nr: u64 = 0,
    arg0: u64 = 0,
    arg1: u64 = 0,
    arg2: u64 = 0,
    arg3: u64 = 0,
    arg4: u64 = 0,
    arg5: u64 = 0,
    retval: i64 = 0,
};
