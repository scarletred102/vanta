// ============================================================================
// VantaOS Userspace — Consumer Server
// ============================================================================

const std = @import("std");
const libvanta = @import("../libvanta/libvanta.zig");

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

// Exact replica of the kernel's Message layout to avoid alignment errors
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
    // Placeholders to align with kernel-only fields
    transferred_caps: [4]CapEntry = [_]CapEntry{.{}} ** 4,
    transferred_buffer_cap: CapEntry = .{},
};

pub export fn main() void {
    libvanta.vanta_debug_print("CONSUMER: Starting...");

    const shared_port_handle: u64 = 0x0001000000000001;

    libvanta.vanta_debug_print("CONSUMER: Waiting for messages...");

    var expected_seq: usize = 1;
    var ack_notif_handle: u64 = 0;

    while (expected_seq <= 1000) : (expected_seq += 1) {
        var msg: Message = .{};
        const recv_err = libvanta.vanta_cap_recv(shared_port_handle, @intFromPtr(&msg));
        if (recv_err != 0) {
            libvanta.vanta_debug_print("CONSUMER: Recv failed!");
            libvanta.vanta_exit(1);
        }

        // Verify sequence
        var buf: [32]u8 = [_]u8{0} ** 32;
        const expected_str = std.fmt.bufPrint(&buf, "MSG {d}", .{expected_seq}) catch unreachable;
        const actual_str = std.mem.sliceTo(&msg.payload, 0);

        if (!std.mem.eql(u8, actual_str, expected_str)) {
            libvanta.vanta_debug_print("CONSUMER: Out-of-order message received!");
            libvanta.vanta_exit(2);
        }

        if (expected_seq == 1000) {
            // Receive notification capability in slot 0
            ack_notif_handle = msg.caps[0];
        }
    }

    libvanta.vanta_debug_print("CONSUMER: Received all 1000 messages successfully! Sending ACK...");

    if (ack_notif_handle != 0) {
        const notify_err = libvanta.vanta_cap_notify(ack_notif_handle, 1);
        if (notify_err == 0) {
            libvanta.vanta_debug_print("CONSUMER: ACK notified successfully! Exiting.");
            libvanta.vanta_exit(0);
        } else {
            libvanta.vanta_debug_print("CONSUMER: ACK notification failed!");
            libvanta.vanta_exit(3);
        }
    } else {
        libvanta.vanta_debug_print("CONSUMER: No ACK notification capability received!");
        libvanta.vanta_exit(4);
    }
}
