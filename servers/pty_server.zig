// ============================================================================
// VantaOS Userspace — PTY Server (Phase 10 stub)
// ============================================================================

const std = @import("std");
const libvanta = @import("../libvanta/libvanta.zig");

// Pre-assigned startup capability handles
pub const PORT_CAP_HANDLE: u64 = 0x0001000000000001; // Slot 1, Gen 1
pub const REGISTRY_CAP_HANDLE: u64 = 0x0001000000000002; // Slot 2, Gen 1

// Message codes
pub const MSG_PTY_OPEN: u32 = 0x40;
pub const MSG_PTY_WRITE: u32 = 0x41;
pub const MSG_PTY_READ: u32 = 0x42;
pub const MSG_PTY_CLOSE: u32 = 0x43;
pub const MSG_REGISTRY_REGISTER: u32 = 0x10;
pub const MSG_ERROR: u32 = 0x0003;

// Error codes
pub const OK: i64 = 0;
pub const EINVAL: i64 = -5;

// FD type constants
pub const FD_MASTER: u32 = 0;
pub const FD_SLAVE: u32 = 1;

// ── Message / CapEntry ──────────────────────────────────────────────────────

pub const CapEntry = struct {
    type: u4 = 0,
    rights: u8 = 0,
    generation: u16 = 1,
    kernel_object_ptr: u48 = 0,
    next_derived_table: ?*anyopaque = null,
    next_derived_index: u16 = 0,
    parent_table: ?*anyopaque = null,
    parent_index: u16 = 0,
    parent_generation: u16 = 0,
    old_table: ?*anyopaque = null,
    old_index: u16 = 0,
};

pub const Message = struct {
    msg_type: u32 = 0,
    flags: packed struct(u32) {
        expects_reply: bool = false,
        is_reply: bool = false,
        has_buffer: bool = false,
        urgent: bool = false,
        _reserved: u28 = 0,
    } = .{},
    payload: [64]u8 = [_]u8{0} ** 64,
    caps: [4]u64 = [_]u64{0} ** 4,
    buffer_cap: u64 = 0,
    transferred_caps: [4]CapEntry = [_]CapEntry{.{}} ** 4,
    transferred_buffer_cap: CapEntry = .{},
};

// ── Ring buffer ─────────────────────────────────────────────────────────────

const RING_SIZE: usize = 4096;
const Ring = struct {
    buf: [RING_SIZE]u8 = [_]u8{0} ** RING_SIZE,
    head: usize = 0,
    tail: usize = 0,

    fn push(self: *Ring, data: []const u8) usize {
        var written: usize = 0;
        for (data) |b| {
            const next = (self.tail + 1) % RING_SIZE;
            if (next == self.head) break; // full
            self.buf[self.tail] = b;
            self.tail = next;
            written += 1;
        }
        return written;
    }

    fn pop(self: *Ring, out: []u8) usize {
        var read: usize = 0;
        while (read < out.len and self.head != self.tail) {
            out[read] = self.buf[self.head];
            self.head = (self.head + 1) % RING_SIZE;
            read += 1;
        }
        return read;
    }
};

// master writes here → slave reads
var master_to_slave = Ring{};
// slave writes here → master reads
var slave_to_master = Ring{};

// ── Helpers ─────────────────────────────────────────────────────────────────

fn sendErrorReply(code: i64) void {
    var reply = Message{};
    reply.msg_type = MSG_ERROR;
    reply.flags.is_reply = true;
    std.mem.writeInt(i64, reply.payload[0..8], code, .little);
    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
}

fn registerWithRegistry() void {
    var msg = Message{};
    msg.msg_type = MSG_REGISTRY_REGISTER;

    const name = "sys.pty";
    @memcpy(msg.payload[0..name.len], name);
    msg.payload[name.len] = 0;
    // Derive a send cap so we keep PORT_CAP_HANDLE for our own service loop
    var send_cap: u64 = 0;
    _ = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 7, @intFromPtr(&send_cap));
    msg.caps[0] = send_cap;

    _ = libvanta.vanta_cap_send(REGISTRY_CAP_HANDLE, @intFromPtr(&msg));
    libvanta.vanta_debug_print("[PTY] Registered as 'sys.pty'\n");
}

// ── Main ─────────────────────────────────────────────────────────────────────

pub export fn main() void {
    libvanta.vanta_debug_print("[PTY] server starting");

    registerWithRegistry();

    libvanta.vanta_debug_print("[PTY] Entering IPC loop...");
    while (true) {
        var msg = Message{};
        const recv_err = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&msg));
        if (recv_err != 0) continue;

        switch (msg.msg_type) {

            // ── PTY Open ────────────────────────────────────────────────
            MSG_PTY_OPEN => {
                // Reply with same cap handle for both sides; fd_type in payload
                // distinguishes master (0) from slave (1) in subsequent calls.
                var reply = Message{};
                reply.msg_type = MSG_PTY_OPEN;
                reply.flags.is_reply = true;
                // master cap in caps[0], slave cap in caps[1]
                reply.caps[0] = PORT_CAP_HANDLE;
                reply.caps[1] = PORT_CAP_HANDLE;
                // Also encode handles in payload for convenience
                std.mem.writeInt(u64, reply.payload[0..8], PORT_CAP_HANDLE, .little);
                std.mem.writeInt(u64, reply.payload[8..16], PORT_CAP_HANDLE, .little);
                _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
            },

            // ── PTY Write ───────────────────────────────────────────────
            MSG_PTY_WRITE => {
                const fd_type = std.mem.readInt(u32, msg.payload[0..4], .little);
                const data_len = std.mem.readInt(u32, msg.payload[4..8], .little);
                const clamped_len = @min(data_len, 64);
                const data = msg.payload[8 .. 8 + clamped_len];

                const written = if (fd_type == FD_MASTER)
                    master_to_slave.push(data)
                else
                    slave_to_master.push(data);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_PTY_WRITE;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u32, reply.payload[0..4], @intCast(written), .little);
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },

            // ── PTY Read ────────────────────────────────────────────────
            MSG_PTY_READ => {
                const fd_type = std.mem.readInt(u32, msg.payload[0..4], .little);

                var out_buf: [64]u8 = [_]u8{0} ** 64;
                const n = if (fd_type == FD_MASTER)
                    slave_to_master.pop(&out_buf)
                else
                    master_to_slave.pop(&out_buf);

                var reply = Message{};
                reply.msg_type = MSG_PTY_READ;
                reply.flags.is_reply = true;
                std.mem.writeInt(u32, reply.payload[0..4], @intCast(n), .little);
                @memcpy(reply.payload[4 .. 4 + n], out_buf[0..n]);
                _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
            },

            // ── PTY Close ───────────────────────────────────────────────
            MSG_PTY_CLOSE => {
                // No-op for Phase 10 stub; acknowledge if expected.
                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_PTY_CLOSE;
                    reply.flags.is_reply = true;
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },

            else => {
                sendErrorReply(EINVAL);
            },
        }
    }
}
