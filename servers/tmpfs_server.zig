// ============================================================================
// VantaOS Userspace — tmpfs In-Memory Filesystem Server
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

pub const DirEntry = extern struct {
    ino: u64,
    is_dir: u8,
    name_len: u8,
    name: [62]u8,
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

// ── tmpfs In-Memory Data Structures ──────────────────────────────

const TmpNode = struct {
    ino: u64,
    name: []const u8,
    is_dir: bool,
    data: std.ArrayList(u8),
    children: std.ArrayList(*TmpNode),
};

var next_ino: u64 = 1;
var root_node: *TmpNode = undefined;

// Stateful open file sessions
const FileSession = struct {
    id: u64,
    node: *TmpNode,
    offset: u64,
    flags: u32,
    active: bool,
};

var sessions: [512]FileSession = [_]FileSession{.{ .id = 0, .node = undefined, .offset = 0, .flags = 0, .active = false }} ** 512;
var next_session_id: u64 = 1;

// Path lookup helper
fn findNode(path: []const u8) ?*TmpNode {
    if (path.len == 0 or std.mem.eql(u8, path, "/")) {
        return root_node;
    }

    var current = root_node;
    var it = std.mem.tokenizeAny(u8, path, "/");
    
    while (it.next()) |part| {
        if (!current.is_dir) return null;
        var found = false;
        for (current.children.items) |child| {
            if (std.mem.eql(u8, child.name, part)) {
                current = child;
                found = true;
                break;
            }
        }
        if (!found) return null;
    }
    return current;
}

fn createNode(parent_path: []const u8, name: []const u8, is_dir: bool) ?*TmpNode {
    const parent = findNode(parent_path) orelse return null;
    if (!parent.is_dir) return null;

    const node = gpa.create(TmpNode) catch return null;
    node.* = .{
        .ino = next_ino,
        .name = gpa.dupe(u8, name) catch return null,
        .is_dir = is_dir,
        .data = std.ArrayList(u8).empty,
        .children = std.ArrayList(*TmpNode).empty,
    };
    next_ino += 1;

    parent.children.append(gpa, node) catch return null;
    return node;
}

pub export fn main() void {
    libvanta.vanta_debug_print("tmpfs: Starting in-memory filesystem server...");

    // Initialize root directory
    root_node = gpa.create(TmpNode) catch unreachable;
    root_node.* = .{
        .ino = next_ino,
        .name = "/",
        .is_dir = true,
        .data = std.ArrayList(u8).empty,
        .children = std.ArrayList(*TmpNode).empty,
    };
    next_ino += 1;

    // Registry Registration
    libvanta.vanta_debug_print("tmpfs: Registering with service registry...");
    var derived_port: u64 = 0;
    const derive_err = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 3, @intFromPtr(&derived_port));
    if (derive_err == 0) {
        var reg_msg = Message{};
        reg_msg.msg_type = 0x10; // RegistryRegister
        @memcpy(reg_msg.payload[0..8], "fs.tmpfs");
        reg_msg.caps[0] = derived_port;
        _ = libvanta.vanta_cap_send(REGISTRY_CAP_HANDLE, @intFromPtr(&reg_msg));
    }

    libvanta.vanta_debug_print("tmpfs: Entering IPC service loop...");
    while (true) {
        var msg = Message{};
        const recv_err = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&msg));
        if (recv_err != 0) continue;

        switch (msg.msg_type) {
            MSG_FS_OPEN => {
                const flags = std.mem.readInt(u32, msg.payload[0..4], .little);
                const path = std.mem.sliceTo(msg.payload[4..64], 0);

                var node = findNode(path);
                if (node == null and (flags & 8) != 0) { // O_CREAT
                    // Extract parent path and name
                    var parent_path: []const u8 = "";
                    var name: []const u8 = path;
                    if (std.mem.lastIndexOfScalar(u8, path, '/')) |idx| {
                        parent_path = path[0..idx];
                        name = path[idx + 1 ..];
                    }
                    node = createNode(parent_path, name, false);
                }

                if (node == null) {
                    sendErrorReply(&msg);
                    continue;
                }

                // Allocate a new session
                var session_idx: ?usize = null;
                for (0..512) |idx| {
                    if (!sessions[idx].active) {
                        session_idx = idx;
                        break;
                    }
                }

                if (session_idx) |idx| {
                    const sid = next_session_id;
                    next_session_id += 1;

                    sessions[idx] = .{
                        .id = sid,
                        .node = node.?,
                        .offset = 0,
                        .flags = flags,
                        .active = true,
                    };

                    var reply = Message{};
                    reply.msg_type = MSG_FS_OPEN;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], sid, .little);
                    
                    // Derive child FdCap and return it
                    var fd_cap: u64 = 0;
                    _ = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 3, @intFromPtr(&fd_cap));
                    reply.caps[0] = fd_cap;

                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                } else {
                    sendErrorReply(&msg);
                }
            },
            MSG_FS_READ => {
                const sid = std.mem.readInt(u64, msg.payload[0..8], .little);
                const offset = std.mem.readInt(u64, msg.payload[8..16], .little);
                const len = std.mem.readInt(u64, msg.payload[16..24], .little);
                const shm_cap = msg.buffer_cap;

                var session: ?*FileSession = null;
                for (0..512) |idx| {
                    if (sessions[idx].active and sessions[idx].id == sid) {
                        session = &sessions[idx];
                        break;
                    }
                }

                if (session == null or shm_cap == 0) {
                    sendErrorReply(&msg);
                    if (shm_cap != 0) _ = libvanta.vanta_cap_revoke(shm_cap);
                    continue;
                }

                const node = session.?.node;
                const pages_to_map = (len + 4095) / 4096;
                const shm_map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, pages_to_map);
                if (shm_map_err != 0) {
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                // Copy data to shared memory
                const bytes_to_read = if (offset >= node.data.items.len) 0 else @min(len, node.data.items.len - offset);
                if (bytes_to_read > 0) {
                    const shm_ptr: [*]u8 = @ptrFromInt(SHM_VADDR);
                    @memcpy(shm_ptr[0..bytes_to_read], node.data.items[offset .. offset + bytes_to_read]);
                }

                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_FS_READ;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], bytes_to_read, .little);
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },
            MSG_FS_WRITE => {
                const sid = std.mem.readInt(u64, msg.payload[0..8], .little);
                const offset = std.mem.readInt(u64, msg.payload[8..16], .little);
                const len = std.mem.readInt(u64, msg.payload[16..24], .little);
                const shm_cap = msg.buffer_cap;

                var session: ?*FileSession = null;
                for (0..512) |idx| {
                    if (sessions[idx].active and sessions[idx].id == sid) {
                        session = &sessions[idx];
                        break;
                    }
                }

                if (session == null or shm_cap == 0) {
                    sendErrorReply(&msg);
                    if (shm_cap != 0) _ = libvanta.vanta_cap_revoke(shm_cap);
                    continue;
                }

                const node = session.?.node;
                const pages_to_map = (len + 4095) / 4096;
                const shm_map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, pages_to_map);
                if (shm_map_err != 0) {
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                // Copy data from shared memory
                if (offset + len > node.data.items.len) {
                    node.data.resize(gpa, offset + len) catch unreachable;
                }

                if (len > 0) {
                    const shm_ptr: [*]const u8 = @ptrFromInt(SHM_VADDR);
                    @memcpy(node.data.items[offset .. offset + len], shm_ptr[0..len]);
                }

                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_FS_WRITE;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], len, .little);
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },
            MSG_FS_CLOSE => {
                const sid = std.mem.readInt(u64, msg.payload[0..8], .little);
                for (0..512) |idx| {
                    if (sessions[idx].active and sessions[idx].id == sid) {
                        sessions[idx].active = false;
                        break;
                    }
                }

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_FS_CLOSE;
                    reply.flags.is_reply = true;
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },
            MSG_FS_STAT => {
                const path = std.mem.sliceTo(msg.payload[0..64], 0);
                if (findNode(path)) |node| {
                    var reply = Message{};
                    reply.msg_type = MSG_FS_STAT;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], node.data.items.len, .little);
                    reply.payload[8] = if (node.is_dir) 1 else 0;
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                } else {
                    sendErrorReply(&msg);
                }
            },
            MSG_FS_READDIR => {
                const sid = std.mem.readInt(u64, msg.payload[0..8], .little);
                const offset = std.mem.readInt(u64, msg.payload[8..16], .little);
                const shm_cap = msg.buffer_cap;

                var session: ?*FileSession = null;
                for (0..512) |idx| {
                    if (sessions[idx].active and sessions[idx].id == sid) {
                        session = &sessions[idx];
                        break;
                    }
                }

                if (session == null or shm_cap == 0) {
                    sendErrorReply(&msg);
                    if (shm_cap != 0) _ = libvanta.vanta_cap_revoke(shm_cap);
                    continue;
                }

                const node = session.?.node;
                if (!node.is_dir) {
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                const shm_map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, 1);
                if (shm_map_err != 0) {
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                const shm_ptr = @as([*]DirEntry, @ptrFromInt(SHM_VADDR));
                var entry_count: u64 = 0;
                
                var child_idx = offset;
                while (child_idx < node.children.items.len and entry_count < 50) : (child_idx += 1) {
                    const child = node.children.items[child_idx];
                    var entry = DirEntry{
                        .ino = child.ino,
                        .is_dir = if (child.is_dir) 1 else 0,
                        .name_len = @truncate(child.name.len),
                        .name = [_]u8{0} ** 62,
                    };
                    @memcpy(entry.name[0..@min(child.name.len, 61)], child.name[0..@min(child.name.len, 61)]);
                    
                    shm_ptr[entry_count] = entry;
                    entry_count += 1;
                }

                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_FS_READDIR;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], entry_count, .little);
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },
            MSG_FS_MKDIR => {
                const path = std.mem.sliceTo(msg.payload[0..64], 0);
                var parent_path: []const u8 = "";
                var name: []const u8 = path;
                if (std.mem.lastIndexOfScalar(u8, path, '/')) |idx| {
                    parent_path = path[0..idx];
                    name = path[idx + 1 ..];
                }
                
                const node = createNode(parent_path, name, true);
                if (node != null) {
                    if (msg.flags.expects_reply) {
                        var reply = Message{};
                        reply.msg_type = MSG_FS_MKDIR;
                        reply.flags.is_reply = true;
                        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                    }
                } else {
                    sendErrorReply(&msg);
                }
            },
            MSG_FS_UNLINK => {
                const path = std.mem.sliceTo(msg.payload[0..64], 0);
                var parent_path: []const u8 = "";
                var name: []const u8 = path;
                if (std.mem.lastIndexOfScalar(u8, path, '/')) |idx| {
                    parent_path = path[0..idx];
                    name = path[idx + 1 ..];
                }

                const parent = findNode(parent_path);
                if (parent != null) {
                    var remove_idx: ?usize = null;
                    for (parent.?.children.items, 0..) |child, idx| {
                        if (std.mem.eql(u8, child.name, name)) {
                            remove_idx = idx;
                            break;
                        }
                    }

                    if (remove_idx) |idx| {
                        _ = parent.?.children.orderedRemove(idx);
                        if (msg.flags.expects_reply) {
                            var reply = Message{};
                            reply.msg_type = MSG_FS_UNLINK;
                            reply.flags.is_reply = true;
                            _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                        }
                        continue;
                    }
                }
                sendErrorReply(&msg);
            },
            MSG_FS_RENAME => {
                const src = std.mem.sliceTo(msg.payload[0..32], 0);
                const dst = std.mem.sliceTo(msg.payload[32..64], 0);
                
                const src_node = findNode(src);
                if (src_node != null) {
                    var parent_path: []const u8 = "";
                    var name: []const u8 = dst;
                    if (std.mem.lastIndexOfScalar(u8, dst, '/')) |idx| {
                        parent_path = dst[0..idx];
                        name = dst[idx + 1 ..];
                    }

                    const new_parent = findNode(parent_path);
                    if (new_parent != null) {
                        // Unlink from old parent
                        var old_parent_path: []const u8 = "";
                        var old_name: []const u8 = src;
                        if (std.mem.lastIndexOfScalar(u8, src, '/')) |idx| {
                            old_parent_path = src[0..idx];
                            old_name = src[idx + 1 ..];
                        }
                        const old_parent = findNode(old_parent_path);
                        if (old_parent != null) {
                            for (old_parent.?.children.items, 0..) |child, idx| {
                                if (child == src_node.?) {
                                    _ = old_parent.?.children.orderedRemove(idx);
                                    break;
                                }
                            }
                        }

                        // Rename and append to new parent
                        src_node.?.name = gpa.dupe(u8, name) catch unreachable;
                        new_parent.?.children.append(gpa, src_node.?) catch unreachable;

                        if (msg.flags.expects_reply) {
                            var reply = Message{};
                            reply.msg_type = MSG_FS_RENAME;
                            reply.flags.is_reply = true;
                            _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                        }
                        continue;
                    }
                }
                sendErrorReply(&msg);
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
