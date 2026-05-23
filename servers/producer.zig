// ============================================================================
// VantaOS Userspace — Producer Server
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
    libvanta.vanta_debug_print("PRODUCER: Starting...");

    const shared_port_handle: u64 = 0x0001000000000001;

    const notif = libvanta.vanta_notif_create();
    if (notif.err != 0) {
        libvanta.vanta_debug_print("PRODUCER: Failed to create notif");
        libvanta.vanta_exit(1);
    }

    var derived_handle: u64 = 0;
    const derive_err = libvanta.vanta_cap_derive(notif.handle, 1, @intFromPtr(&derived_handle));
    if (derive_err != 0) {
        libvanta.vanta_debug_print("PRODUCER: Failed to derive notif handle");
        libvanta.vanta_exit(4);
    }

    libvanta.vanta_debug_print("PRODUCER: Sending messages...");

    var i: usize = 1;
    while (i <= 1000) : (i += 1) {
        var msg: Message = .{};
        msg.msg_type = 0x100;
        
        var buf: [32]u8 = [_]u8{0} ** 32;
        const msg_str = std.fmt.bufPrint(&buf, "MSG {d}", .{i}) catch unreachable;
        @memcpy(msg.payload[0..msg_str.len], msg_str);

        if (i == 1000) {
            msg.caps[0] = derived_handle;
        }

        const send_err = libvanta.vanta_cap_send(shared_port_handle, @intFromPtr(&msg));
        if (send_err != 0) {
            libvanta.vanta_debug_print("PRODUCER: Send failed!");
            libvanta.vanta_exit(2);
        }
    }

    libvanta.vanta_debug_print("PRODUCER: All 1000 sent. Waiting for ACK...");
    
    const wait_res = libvanta.vanta_cap_wait(notif.handle, 1);
    if (wait_res.err == 0 and wait_res.matched == 1) {
        libvanta.vanta_debug_print("PRODUCER: ACK received successfully! Exiting.");
        libvanta.vanta_exit(0);
    } else {
        libvanta.vanta_debug_print("PRODUCER: ACK wait failed.");
        var err_buf: [64]u8 = [_]u8{0} ** 64;
        const err_str = std.fmt.bufPrint(&err_buf, "PRODUCER: err={d} matched={d}", .{wait_res.err, wait_res.matched}) catch unreachable;
        libvanta.vanta_debug_print(err_str);
        libvanta.vanta_exit(3);
    }
}
