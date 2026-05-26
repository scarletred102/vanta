// ============================================================================
// VantaOS — Linux Personality Server (Phase 10)
//
// Handles Linux syscall emulation for ELF binaries running under the
// personality layer. Communicates with the kernel via shared memory and
// notification caps.
// ============================================================================

const std = @import("std");
const libvanta = @import("../libvanta/libvanta.zig");

const PORT_CAP_HANDLE: u64 = 0x0001000000000001;
const REGISTRY_CAP_HANDLE: u64 = 0x0001000000000002;
const MSG_PERSONALITY_SETUP: u32 = 0x30;
const MAX_SLOTS: usize = 8;

// Linux syscall numbers (x86_64)
const SYS_READ: u64 = 0;
const SYS_WRITE: u64 = 1;
const SYS_OPEN: u64 = 2;
const SYS_CLOSE: u64 = 3;
const SYS_STAT: u64 = 4;
const SYS_FSTAT: u64 = 5;
const SYS_LSEEK: u64 = 8;
const SYS_MMAP: u64 = 9;
const SYS_MPROTECT: u64 = 10;
const SYS_MUNMAP: u64 = 11;
const SYS_BRK: u64 = 12;
const SYS_SIGACTION: u64 = 13;
const SYS_IOCTL: u64 = 16;
const SYS_WRITEV: u64 = 20;
const SYS_DUP: u64 = 32;
const SYS_DUP2: u64 = 33;
const SYS_GETPID: u64 = 39;
const SYS_CLONE: u64 = 56;
const SYS_EXIT: u64 = 60;
const SYS_UNAME: u64 = 63;
const SYS_GETUID: u64 = 102;
const SYS_GETGID: u64 = 104;
const SYS_GETEUID: u64 = 107;
const SYS_GETEGID: u64 = 108;
const SYS_GETPPID: u64 = 110;
const SYS_ARCH_PRCTL: u64 = 158;
const SYS_FUTEX: u64 = 202;
const SYS_SET_TID_ADDRESS: u64 = 218;
const SYS_EXIT_GROUP: u64 = 231;
const SYS_OPENAT: u64 = 257;
const SYS_NEWFSTATAT: u64 = 262;
const SYS_SET_ROBUST_LIST: u64 = 273;
const SYS_PRLIMIT64: u64 = 302;

// Linux errno values (negative)
const ENOSYS: i64 = -38;
const EBADF: i64 = -9;
const EINVAL: i64 = -22;
const ENOMEM: i64 = -12;
const ENOENT: i64 = -2;
const EPERM: i64 = -1;

// arch_prctl codes
const ARCH_SET_FS: u64 = 0x1002;
const ARCH_GET_FS: u64 = 0x1003;

// ── Message ──────────────────────────────────────────────────────────────────

const Message = struct {
    msg_type: u32 = 0,
    flags: u32 = 0,
    payload: [64]u8 = [_]u8{0} ** 64,
    caps: [4]u64 = [_]u64{0} ** 4,
    buffer_cap: u64 = 0,
};

// ── SHM block (must match kernel/ipc/personality.zig) ────────────────────────

const SyscallShmBlock = extern struct {
    nr: u64 = 0,
    arg0: u64 = 0,
    arg1: u64 = 0,
    arg2: u64 = 0,
    arg3: u64 = 0,
    arg4: u64 = 0,
    arg5: u64 = 0,
    retval: i64 = 0,
};

// ── Slot ─────────────────────────────────────────────────────────────────────

const LinuxSlot = struct {
    active: bool = false,
    shm_ptr: *volatile SyscallShmBlock = undefined,
    ping_cap: u64 = 0,
    pong_cap: u64 = 0,
    thread_cap: u64 = 0,
    pid: u32 = 0,
    tid: u32 = 0,
    fds_open: [64]bool = [_]bool{false} ** 64,
    brk_current: u64 = 0x4000_0000,
};

var slots: [MAX_SLOTS]LinuxSlot = [_]LinuxSlot{.{}} ** MAX_SLOTS;

// ── Helpers ───────────────────────────────────────────────────────────────────

inline fn readU64LE(bytes: []const u8) u64 {
    return @as(u64, bytes[0]) |
        (@as(u64, bytes[1]) << 8) |
        (@as(u64, bytes[2]) << 16) |
        (@as(u64, bytes[3]) << 24) |
        (@as(u64, bytes[4]) << 32) |
        (@as(u64, bytes[5]) << 40) |
        (@as(u64, bytes[6]) << 48) |
        (@as(u64, bytes[7]) << 56);
}

// ── Syscall handlers ──────────────────────────────────────────────────────────

fn handleWrite(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = slot;
    const fd: u32 = @truncate(blk.arg0);
    const buf_ptr = blk.arg1;
    const count = blk.arg2;
    if (count == 0) return 0;
    if (fd == 1 or fd == 2) {
        const max_len = @min(count, 256);
        const slice = @as([*]const u8, @ptrFromInt(buf_ptr))[0..max_len];
        libvanta.vanta_debug_print(slice);
        return @intCast(count);
    }
    return EBADF;
}

fn handleRead(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = slot;
    const fd: u32 = @truncate(blk.arg0);
    if (fd == 0) return 0; // stdin EOF
    return EBADF;
}

fn handleOpen(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = blk;
    var i: usize = 3;
    while (i < 64) : (i += 1) {
        if (!slot.fds_open[i]) {
            slot.fds_open[i] = true;
            return @intCast(i);
        }
    }
    return ENOMEM;
}

fn handleClose(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    const fd: u32 = @truncate(blk.arg0);
    if (fd < 3) return 0;
    if (fd >= 64) return EBADF;
    if (!slot.fds_open[fd]) return EBADF;
    slot.fds_open[fd] = false;
    return 0;
}

fn handleStat(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = slot;
    const stat_ptr = if (blk.nr == SYS_STAT or blk.nr == SYS_FSTAT) blk.arg1 else blk.arg2;
    if (stat_ptr != 0) {
        @memset(@as([*]u8, @ptrFromInt(stat_ptr))[0..144], 0);
    }
    return 0;
}

fn handleLseek(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = slot;
    _ = blk;
    return 0;
}

fn handleMmap(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    const length = blk.arg1;
    const prot = blk.arg2;
    const flags = blk.arg3;
    const fd_raw: i64 = @bitCast(blk.arg4);
    const hint = blk.arg0;
    _ = flags;
    if (fd_raw >= 0) return ENOSYS; // file-backed mmap not supported
    const n_pages = (length + 4095) / 4096;
    const res = libvanta.vanta_process_mmap(@as(u64, slot.pid), hint, n_pages, prot);
    if (res.err != 0) return ENOMEM;
    return @intCast(res.vaddr);
}

fn handleMunmap(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    const addr = blk.arg0;
    const length = blk.arg1;
    const n_pages = (length + 4095) / 4096;
    _ = libvanta.vanta_process_munmap(@as(u64, slot.pid), addr, n_pages);
    return 0;
}

fn handleBrk(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    const new_brk = blk.arg0;
    if (new_brk == 0) return @intCast(slot.brk_current);
    if (new_brk > slot.brk_current) {
        const n_pages = (new_brk - slot.brk_current + 4095) / 4096;
        const res = libvanta.vanta_process_mmap(
            @as(u64, slot.pid), slot.brk_current, n_pages, 3, // PROT_READ|PROT_WRITE
        );
        if (res.err != 0) return @intCast(slot.brk_current);
        slot.brk_current = res.vaddr + n_pages * 4096;
    }
    return @intCast(slot.brk_current);
}

fn handleIoctl(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = slot;
    _ = blk;
    return 0;
}

fn handleWritev(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = slot;
    const fd: u32 = @truncate(blk.arg0);
    if (fd != 1 and fd != 2) return EBADF;
    const iov_ptr = blk.arg1;
    const iovcnt: u32 = @truncate(blk.arg2);
    var total: i64 = 0;
    var i: u32 = 0;
    while (i < iovcnt) : (i += 1) {
        const iov_base = @as(*const u64, @ptrFromInt(iov_ptr + i * 16)).*;
        const iov_len = @as(*const u64, @ptrFromInt(iov_ptr + i * 16 + 8)).*;
        if (iov_len > 0 and iov_base != 0) {
            const max_len = @min(iov_len, 256);
            const slice = @as([*]const u8, @ptrFromInt(iov_base))[0..max_len];
            libvanta.vanta_debug_print(slice);
            total += @intCast(iov_len);
        }
    }
    return total;
}

fn handleDup(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    const oldfd: u32 = @truncate(blk.arg0);
    if (oldfd >= 64) return EBADF;
    if (oldfd < 3 or slot.fds_open[oldfd]) {
        var i: usize = 3;
        while (i < 64) : (i += 1) {
            if (!slot.fds_open[i]) {
                slot.fds_open[i] = true;
                return @intCast(i);
            }
        }
    }
    return EBADF;
}

fn handleDup2(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    const oldfd: u32 = @truncate(blk.arg0);
    const newfd: u32 = @truncate(blk.arg1);
    if (oldfd >= 64 or newfd >= 64) return EBADF;
    if (newfd < 3) return EINVAL;
    slot.fds_open[newfd] = true;
    return @intCast(newfd);
}

fn handleArchPrctl(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    const code = blk.arg0;
    const addr = blk.arg1;
    if (code == ARCH_SET_FS) {
        _ = libvanta.vanta_thread_set_fs_base(slot.thread_cap, addr);
        return 0;
    }
    if (code == ARCH_GET_FS) {
        if (addr != 0) {
            @as(*u64, @ptrFromInt(addr)).* = 0;
        }
        return 0;
    }
    return EINVAL;
}

fn handleFutex(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = slot;
    const op: u32 = @truncate(blk.arg1);
    const FUTEX_WAIT: u32 = 0;
    const FUTEX_WAKE: u32 = 1;
    return switch (op & 0x7F) {
        FUTEX_WAIT => EINVAL,
        FUTEX_WAKE => 0,
        else => ENOSYS,
    };
}

fn handleUname(slot: *LinuxSlot, blk: *volatile SyscallShmBlock) i64 {
    _ = slot;
    const ptr = blk.arg0;
    if (ptr == 0) return EINVAL;
    @memset(@as([*]u8, @ptrFromInt(ptr))[0..390], 0);
    @memcpy(@as([*]u8, @ptrFromInt(ptr))[0..5], "Linux");
    @memcpy(@as([*]u8, @ptrFromInt(ptr + 65))[0..5], "vanta");
    @memcpy(@as([*]u8, @ptrFromInt(ptr + 130))[0..11], "6.1.0-vanta");
    @memcpy(@as([*]u8, @ptrFromInt(ptr + 260))[0..6], "x86_64");
    return 0;
}

// ── Slot dispatch ─────────────────────────────────────────────────────────────

fn handleLinuxSyscall(slot: *LinuxSlot) void {
    _ = libvanta.vanta_cap_wait(slot.ping_cap, 1);

    const blk = slot.shm_ptr;
    const nr = blk.nr;

    const retval: i64 = switch (nr) {
        SYS_READ => handleRead(slot, blk),
        SYS_WRITE => handleWrite(slot, blk),
        SYS_OPEN, SYS_OPENAT => handleOpen(slot, blk),
        SYS_CLOSE => handleClose(slot, blk),
        SYS_FSTAT, SYS_STAT, SYS_NEWFSTATAT => handleStat(slot, blk),
        SYS_LSEEK => handleLseek(slot, blk),
        SYS_MMAP => handleMmap(slot, blk),
        SYS_MUNMAP => handleMunmap(slot, blk),
        SYS_BRK => handleBrk(slot, blk),
        SYS_MPROTECT => 0,
        SYS_SIGACTION => 0,
        SYS_IOCTL => handleIoctl(slot, blk),
        SYS_WRITEV => handleWritev(slot, blk),
        SYS_DUP => handleDup(slot, blk),
        SYS_DUP2 => handleDup2(slot, blk),
        SYS_GETPID => @as(i64, @intCast(slot.pid)),
        SYS_GETPPID => 1,
        SYS_GETUID, SYS_GETEUID => 0,
        SYS_GETGID, SYS_GETEGID => 0,
        SYS_ARCH_PRCTL => handleArchPrctl(slot, blk),
        SYS_FUTEX => handleFutex(slot, blk),
        SYS_CLONE => ENOSYS,
        SYS_EXIT, SYS_EXIT_GROUP => {
            slot.active = false;
            blk.retval = 0;
            _ = libvanta.vanta_cap_notify(slot.pong_cap, 1);
            return;
        },
        SYS_UNAME => handleUname(slot, blk),
        SYS_SET_TID_ADDRESS, SYS_SET_ROBUST_LIST => 0,
        SYS_PRLIMIT64 => 0,
        else => blk2: {
            var buf: [64]u8 = [_]u8{0} ** 64;
            const s = std.fmt.bufPrint(&buf, "[PSERVER] unhandled Linux syscall {}", .{nr}) catch "";
            libvanta.vanta_debug_print(s);
            break :blk2 ENOSYS;
        },
    };

    blk.retval = retval;
    _ = libvanta.vanta_cap_notify(slot.pong_cap, 1);
}

// ── Setup handler ─────────────────────────────────────────────────────────────

fn handlePersonalitySetup(msg: *Message) void {
    var slot_idx: usize = MAX_SLOTS;
    var i: usize = 0;
    while (i < MAX_SLOTS) : (i += 1) {
        if (!slots[i].active) {
            slot_idx = i;
            break;
        }
    }
    if (slot_idx == MAX_SLOTS) {
        libvanta.vanta_debug_print("[PSERVER] no free slots for Linux thread");
        var reply = Message{ .msg_type = 0x32 };
        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
        return;
    }

    const slot = &slots[slot_idx];

    const pid_u64 = readU64LE(msg.payload[0..8]);
    const tid_u64 = readU64LE(msg.payload[8..16]);
    const pserver_shm_virt = readU64LE(msg.payload[16..24]);
    _ = tid_u64;

    _ = libvanta.vanta_shm_map(msg.caps[0], pserver_shm_virt);

    slot.pid = @truncate(pid_u64);
    slot.shm_ptr = @ptrFromInt(pserver_shm_virt);
    slot.ping_cap = msg.caps[1];
    slot.pong_cap = msg.caps[2];
    slot.thread_cap = msg.caps[3];
    slot.brk_current = 0x4000_0000;
    slot.fds_open = [_]bool{false} ** 64;
    slot.active = true;

    {
        var buf: [64]u8 = [_]u8{0} ** 64;
        const s = std.fmt.bufPrint(&buf, "[PSERVER] Linux thread setup: pid={}", .{slot.pid}) catch "";
        libvanta.vanta_debug_print(s);
    }

    var reply = Message{ .msg_type = 0x31 };
    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
}

// ── Registry registration ─────────────────────────────────────────────────────

fn registerWithRegistry() void {
    var msg: Message = .{};
    msg.msg_type = 0x10; // MSG_REGISTRY_REGISTER
    const name = "sys.personality";
    @memcpy(msg.payload[0..name.len], name);
    const handle_val: u64 = PORT_CAP_HANDLE;
    @memcpy(msg.payload[16..24], @as([*]const u8, @ptrFromInt(@intFromPtr(&handle_val)))[0..8]);
    var reply: Message = .{};
    _ = libvanta.vanta_cap_call(REGISTRY_CAP_HANDLE, @intFromPtr(&msg), @intFromPtr(&reply));
}

// ── Main ──────────────────────────────────────────────────────────────────────

pub export fn main() void {
    libvanta.vanta_debug_print("[PSERVER] Linux personality server starting");

    registerWithRegistry();

    var handle_buf: [MAX_SLOTS + 1]u64 = undefined;
    var slot_handle_map: [MAX_SLOTS]usize = undefined;

    while (true) {
        var n_handles: usize = 0;
        handle_buf[0] = PORT_CAP_HANDLE;
        n_handles = 1;

        var s: usize = 0;
        while (s < MAX_SLOTS) : (s += 1) {
            if (slots[s].active) {
                slot_handle_map[s] = n_handles;
                handle_buf[n_handles] = slots[s].ping_cap;
                n_handles += 1;
            }
        }

        const poll_res = libvanta.vanta_cap_poll(
            @intFromPtr(&handle_buf[0]),
            n_handles,
            -1,
        );
        if (poll_res.err != 0) continue;

        const ready_idx = poll_res.idx;
        if (ready_idx == 0) {
            var recv_msg: Message = .{};
            _ = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&recv_msg));
            if (recv_msg.msg_type == MSG_PERSONALITY_SETUP) {
                handlePersonalitySetup(&recv_msg);
            }
        } else {
            var si: usize = 0;
            while (si < MAX_SLOTS) : (si += 1) {
                if (slots[si].active and slot_handle_map[si] == ready_idx) {
                    handleLinuxSyscall(&slots[si]);
                    break;
                }
            }
        }
    }
}
