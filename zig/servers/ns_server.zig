// ============================================================================
// VantaOS Userspace — Namespace Server
// ============================================================================

const std = @import("std");
const libvanta = @import("../libvanta/libvanta.zig");

// Hardcoded startup capability handles
pub const PORT_CAP_HANDLE: u64 = 0x0001000000000001; // Slot 1, Gen 1 (Server Listener Port)
pub const REGISTRY_CAP_HANDLE: u64 = 0x0001000000000002; // Slot 2, Gen 1 (Registry Port)

// Message codes (matching VFS_PROTOCOL.md)
pub const MSG_FS_OPEN: u32 = 0x0100;
pub const MSG_FS_READ: u32 = 0x0101;
pub const MSG_FS_WRITE: u32 = 0x0102;
pub const MSG_FS_CLOSE: u32 = 0x0103;
pub const MSG_FS_STAT: u32 = 0x0104;
pub const MSG_FS_READDIR: u32 = 0x0105;
pub const MSG_FS_MKDIR: u32 = 0x0106;
pub const MSG_FS_UNLINK: u32 = 0x0107;
pub const MSG_FS_RENAME: u32 = 0x0108;

pub const MSG_FS_MOUNT: u32 = 0x0109;
pub const MSG_FS_UNMOUNT: u32 = 0x010A;
pub const MSG_ERROR: u32 = 0x0003;

pub const SHM_VADDR: u64 = 0x30000000;

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

// ── Custom Freestanding Allocator ───────────────────────────────

const PageAllocator = struct {
    pub fn allocator(self: *PageAllocator) std.mem.Allocator {
        return .{
            .ptr = self,
            .vtable = &.{
                .alloc = alloc,
                .resize = resize,
                .remap = remap,
                .free = free,
            },
        };
    }

    fn alloc(ctx: *anyopaque, len: usize, ptr_align: std.mem.Alignment, ret_addr: usize) ?[*]u8 {
        _ = ptr_align; _ = ret_addr; _ = ctx;
        const n_pages = (len + 4095) / 4096;
        if (libvanta.vanta_alloc_pages(n_pages)) |vaddr| {
            return @ptrFromInt(vaddr);
        }
        return null;
    }

    fn resize(ctx: *anyopaque, buf: []u8, buf_align: std.mem.Alignment, new_len: usize, ret_addr: usize) bool {
        _ = ctx; _ = buf; _ = buf_align; _ = new_len; _ = ret_addr;
        return false;
    }

    fn remap(ctx: *anyopaque, buf: []u8, buf_align: std.mem.Alignment, new_len: usize, ret_addr: usize) ?[*]u8 {
        _ = ctx; _ = buf; _ = buf_align; _ = new_len; _ = ret_addr;
        return null;
    }

    fn free(ctx: *anyopaque, buf: []u8, buf_align: std.mem.Alignment, ret_addr: usize) void {
        _ = ctx; _ = buf; _ = buf_align; _ = ret_addr;
    }
};
var page_allocator_state = PageAllocator{};
pub const gpa = page_allocator_state.allocator();

// ── Namespace Data Structures ──────────────────────────────

const Mount = struct {
    path: []const u8,
    fs_cap: u64,
};

var mounts: std.ArrayList(Mount) = undefined;

fn findMount(path: []const u8) ?struct { mount: Mount, rel_path: []const u8 } {
    var best_mount: ?Mount = null;
    var best_len: usize = 0;
    
    for (mounts.items) |m| {
        if (std.mem.startsWith(u8, path, m.path)) {
            if (path.len == m.path.len or m.path.len == 1 or path[m.path.len] == '/') {
                if (m.path.len >= best_len) {
                    best_len = m.path.len;
                    best_mount = m;
                }
            }
        }
    }
    
    if (best_mount) |m| {
        const rel = path[best_len..];
        return .{ .mount = m, .rel_path = rel };
    }
    return null;
}

pub export fn main() void {
    libvanta.vanta_debug_print("ns: Starting namespace routing server...");

    mounts = std.ArrayList(Mount).empty;

    // Register with registry
    libvanta.vanta_debug_print("ns: Registering with service registry...");
    var derived_port: u64 = 0;
    const derive_err = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 3, @intFromPtr(&derived_port));
    if (derive_err == 0) {
        var reg_msg = Message{};
        reg_msg.msg_type = 0x10; // RegistryRegister
        @memcpy(reg_msg.payload[0..13], "sys.namespace");
        reg_msg.caps[0] = derived_port;
        _ = libvanta.vanta_cap_send(REGISTRY_CAP_HANDLE, @intFromPtr(&reg_msg));
    }

    libvanta.vanta_debug_print("ns: Entering IPC routing loop...");
    while (true) {
        var msg = Message{};
        const recv_err = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&msg));
        if (recv_err != 0) continue;

        switch (msg.msg_type) {
            MSG_FS_MOUNT => {
                const path = std.mem.sliceTo(msg.payload[0..64], 0);
                const fs_cap = msg.caps[0];
                
                if (fs_cap == 0 or path.len == 0) {
                    sendErrorReply(&msg);
                    if (fs_cap != 0) _ = libvanta.vanta_cap_revoke(fs_cap);
                    continue;
                }
                
                mounts.append(gpa, .{
                    .path = gpa.dupe(u8, path) catch unreachable,
                    .fs_cap = fs_cap,
                }) catch unreachable;
                
                var dbg_buf: [128]u8 = [_]u8{0} ** 128;
                const dbg_str = std.fmt.bufPrint(&dbg_buf, "ns: Mounted FS at '{s}' (cap=0x{x})", .{path, fs_cap}) catch unreachable;
                libvanta.vanta_debug_print(dbg_str);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_FS_MOUNT;
                    reply.flags.is_reply = true;
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },
            MSG_FS_UNMOUNT => {
                const path = std.mem.sliceTo(msg.payload[0..64], 0);
                var found_idx: ?usize = null;
                for (mounts.items, 0..) |m, idx| {
                    if (std.mem.eql(u8, m.path, path)) {
                        found_idx = idx;
                        break;
                    }
                }
                
                if (found_idx) |idx| {
                    const m = mounts.orderedRemove(idx);
                    _ = libvanta.vanta_cap_revoke(m.fs_cap);
                    if (msg.flags.expects_reply) {
                        var reply = Message{};
                        reply.msg_type = MSG_FS_UNMOUNT;
                        reply.flags.is_reply = true;
                        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                    }
                } else {
                    sendErrorReply(&msg);
                }
            },
            MSG_FS_OPEN => {
                const flags = std.mem.readInt(u32, msg.payload[0..4], .little);
                const path = std.mem.sliceTo(msg.payload[4..64], 0);
                
                if (findMount(path)) |res| {
                    var rel_buf: [128]u8 = [_]u8{0} ** 128;
                    const rel = if (res.rel_path.len == 0 or res.rel_path[0] != '/')
                        std.fmt.bufPrint(&rel_buf, "/{s}", .{res.rel_path}) catch "/"
                    else
                        res.rel_path;
                        
                    // Forward MSG_FS_OPEN with relative path
                    var open_msg = Message{};
                    open_msg.msg_type = MSG_FS_OPEN;
                    open_msg.flags.expects_reply = true;
                    std.mem.writeInt(u32, open_msg.payload[0..4], flags, .little);
                    @memcpy(open_msg.payload[4..4 + rel.len], rel);
                    
                    var reply = Message{};
                    const call_err = libvanta.vanta_cap_call(res.mount.fs_cap, @intFromPtr(&open_msg), @intFromPtr(&reply));
                    if (call_err == 0 and reply.msg_type == MSG_FS_OPEN) {
                        // Forward the response back to client
                        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                    } else {
                        sendErrorReply(&msg);
                    }
                } else {
                    sendErrorReply(&msg);
                }
            },
            MSG_FS_STAT => {
                const path = std.mem.sliceTo(msg.payload[0..64], 0);
                
                if (findMount(path)) |res| {
                    var rel_buf: [128]u8 = [_]u8{0} ** 128;
                    const rel = if (res.rel_path.len == 0 or res.rel_path[0] != '/')
                        std.fmt.bufPrint(&rel_buf, "/{s}", .{res.rel_path}) catch "/"
                    else
                        res.rel_path;
                        
                    var stat_msg = Message{};
                    stat_msg.msg_type = MSG_FS_STAT;
                    stat_msg.flags.expects_reply = true;
                    @memcpy(stat_msg.payload[0..rel.len], rel);
                    
                    var reply = Message{};
                    const call_err = libvanta.vanta_cap_call(res.mount.fs_cap, @intFromPtr(&stat_msg), @intFromPtr(&reply));
                    if (call_err == 0 and reply.msg_type == MSG_FS_STAT) {
                        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                    } else {
                        sendErrorReply(&msg);
                    }
                } else {
                    sendErrorReply(&msg);
                }
            },
            MSG_FS_MKDIR => {
                const path = std.mem.sliceTo(msg.payload[0..64], 0);
                
                if (findMount(path)) |res| {
                    var rel_buf: [128]u8 = [_]u8{0} ** 128;
                    const rel = if (res.rel_path.len == 0 or res.rel_path[0] != '/')
                        std.fmt.bufPrint(&rel_buf, "/{s}", .{res.rel_path}) catch "/"
                    else
                        res.rel_path;
                        
                    var mkdir_msg = Message{};
                    mkdir_msg.msg_type = MSG_FS_MKDIR;
                    mkdir_msg.flags.expects_reply = true;
                    @memcpy(mkdir_msg.payload[0..rel.len], rel);
                    
                    var reply = Message{};
                    const call_err = libvanta.vanta_cap_call(res.mount.fs_cap, @intFromPtr(&mkdir_msg), @intFromPtr(&reply));
                    if (call_err == 0 and reply.msg_type == MSG_FS_MKDIR) {
                        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                    } else {
                        sendErrorReply(&msg);
                    }
                } else {
                    sendErrorReply(&msg);
                }
            },
            MSG_FS_UNLINK => {
                const path = std.mem.sliceTo(msg.payload[0..64], 0);
                
                if (findMount(path)) |res| {
                    var rel_buf: [128]u8 = [_]u8{0} ** 128;
                    const rel = if (res.rel_path.len == 0 or res.rel_path[0] != '/')
                        std.fmt.bufPrint(&rel_buf, "/{s}", .{res.rel_path}) catch "/"
                    else
                        res.rel_path;
                        
                    var unlink_msg = Message{};
                    unlink_msg.msg_type = MSG_FS_UNLINK;
                    unlink_msg.flags.expects_reply = true;
                    @memcpy(unlink_msg.payload[0..rel.len], rel);
                    
                    var reply = Message{};
                    const call_err = libvanta.vanta_cap_call(res.mount.fs_cap, @intFromPtr(&unlink_msg), @intFromPtr(&reply));
                    if (call_err == 0 and reply.msg_type == MSG_FS_UNLINK) {
                        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                    } else {
                        sendErrorReply(&msg);
                    }
                } else {
                    sendErrorReply(&msg);
                }
            },
            MSG_FS_RENAME => {
                const src = std.mem.sliceTo(msg.payload[0..32], 0);
                const dst = std.mem.sliceTo(msg.payload[32..64], 0);
                
                const src_res = findMount(src);
                const dst_res = findMount(dst);
                
                if (src_res != null and dst_res != null and src_res.?.mount.fs_cap == dst_res.?.mount.fs_cap) {
                    const m = src_res.?.mount;
                    
                    var rel_src_buf: [32]u8 = [_]u8{0} ** 32;
                    const rel_src = if (src_res.?.rel_path.len == 0 or src_res.?.rel_path[0] != '/')
                        std.fmt.bufPrint(&rel_src_buf, "/{s}", .{src_res.?.rel_path}) catch "/"
                    else
                        src_res.?.rel_path;
                        
                    var rel_dst_buf: [32]u8 = [_]u8{0} ** 32;
                    const rel_dst = if (dst_res.?.rel_path.len == 0 or dst_res.?.rel_path[0] != '/')
                        std.fmt.bufPrint(&rel_dst_buf, "/{s}", .{dst_res.?.rel_path}) catch "/"
                    else
                        dst_res.?.rel_path;
                        
                    var rename_msg = Message{};
                    rename_msg.msg_type = MSG_FS_RENAME;
                    rename_msg.flags.expects_reply = true;
                    const src_copy_len: usize = @min(rel_src.len, 31);
                    const dst_copy_len: usize = @min(rel_dst.len, 31);
                    @memcpy(rename_msg.payload[0..src_copy_len], rel_src[0..src_copy_len]);
                    @memcpy(rename_msg.payload[32..32 + dst_copy_len], rel_dst[0..dst_copy_len]);
                    
                    var reply = Message{};
                    const call_err = libvanta.vanta_cap_call(m.fs_cap, @intFromPtr(&rename_msg), @intFromPtr(&reply));
                    if (call_err == 0 and reply.msg_type == MSG_FS_RENAME) {
                        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                    } else {
                        sendErrorReply(&msg);
                    }
                } else {
                    sendErrorReply(&msg);
                }
            },
            else => {
                sendErrorReply(&msg);
            }
        }
    }
}

fn sendErrorReply(msg: *const Message) void {
    if (msg.flags.expects_reply) {
        var reply = Message{};
        reply.msg_type = MSG_ERROR;
        reply.flags.is_reply = true;
        @memcpy(reply.payload[0..4], "FAIL");
        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
    }
}
