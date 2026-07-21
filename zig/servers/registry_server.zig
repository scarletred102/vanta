// ============================================================================
// VantaOS Userspace — Registry Server
// ============================================================================

const std = @import("std");
const libvanta = @import("../libvanta/libvanta.zig");

// Pre-assigned startup capability handles
pub const PORT_CAP_HANDLE: u64 = 0x0001000000000001; // Slot 1, Gen 1 (our listener port)

// Message codes (from IPC_FORMAT.md)
pub const MSG_REGISTRY_REGISTER: u32 = 0x10;
pub const MSG_REGISTRY_LOOKUP: u32 = 0x11;
pub const MSG_REGISTRY_LIST: u32 = 0x12;
pub const MSG_ERROR: u32 = 0x0003;

// Error codes
pub const OK: i64 = 0;
pub const EPERM: i64 = -1;
pub const ENOENT: i64 = -2;
pub const EINVAL: i64 = -5;

// ── Message / CapEntry (matching ns_server pattern) ────────────────────────

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

// ── PageAllocator (matching ns_server pattern) ─────────────────────────────

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

// ── Registry Table ─────────────────────────────────────────────────────────

const Entry = struct {
    name: []const u8,
    endpoint_cap: u64,
};

var registry: std.ArrayList(Entry) = undefined;

// ── Helpers ────────────────────────────────────────────────────────────────

fn sendErrorReply(code: i64) void {
    var reply = Message{};
    reply.msg_type = MSG_ERROR;
    reply.flags.is_reply = true;
    std.mem.writeInt(i64, reply.payload[0..8], code, .little);
    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
}

fn findEntry(name: []const u8) ?usize {
    for (registry.items, 0..) |e, i| {
        if (std.mem.eql(u8, e.name, name)) return i;
    }
    return null;
}

// ── Main ───────────────────────────────────────────────────────────────────

pub export fn main() void {
    libvanta.vanta_debug_print("registry: Starting service registry server...");

    registry = std.ArrayList(Entry).empty;

    // Self-register as 'sys.registry' so other processes can discover us by name
    // via a bootstrapped handle.  We derive a send-only cap from our own port and
    // add ourselves to the table directly (no IPC round-trip needed).
    var self_derived: u64 = 0;
    if (libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 1, @intFromPtr(&self_derived)) == 0) {
        const self_name = gpa.dupe(u8, "sys.registry") catch unreachable;
        registry.append(gpa, .{ .name = self_name, .endpoint_cap = self_derived }) catch unreachable;
        libvanta.vanta_debug_print("registry: Registered 'sys.registry'");
    }

    libvanta.vanta_debug_print("registry: Entering IPC loop...");
    while (true) {
        var msg = Message{};
        const recv_err = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&msg));
        if (recv_err != 0) continue;

        switch (msg.msg_type) {

            // ── RegistryRegister ────────────────────────────────────────
            MSG_REGISTRY_REGISTER => {
                const name_raw = std.mem.sliceTo(msg.payload[0..63], 0);
                const endpoint_cap = msg.caps[0];

                if (name_raw.len == 0 or endpoint_cap == 0) {
                    sendErrorReply(EINVAL);
                    continue;
                }

                // Replace existing entry if name is already registered.
                if (findEntry(name_raw)) |idx| {
                    _ = libvanta.vanta_cap_revoke(registry.items[idx].endpoint_cap);
                    gpa.free(registry.items[idx].name);
                    registry.items[idx].endpoint_cap = endpoint_cap;
                    registry.items[idx].name = gpa.dupe(u8, name_raw) catch unreachable;
                } else {
                    const name_copy = gpa.dupe(u8, name_raw) catch unreachable;
                    registry.append(gpa, .{ .name = name_copy, .endpoint_cap = endpoint_cap }) catch unreachable;
                }

                var dbg_buf: [128]u8 = [_]u8{0} ** 128;
                const dbg_str = std.fmt.bufPrint(&dbg_buf, "registry: Registered '{s}'", .{name_raw}) catch unreachable;
                libvanta.vanta_debug_print(dbg_str);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_REGISTRY_REGISTER;
                    reply.flags.is_reply = true;
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },

            // ── RegistryLookup ──────────────────────────────────────────
            MSG_REGISTRY_LOOKUP => {
                const name_raw = std.mem.sliceTo(msg.payload[0..63], 0);

                if (name_raw.len == 0) {
                    sendErrorReply(EINVAL);
                    continue;
                }

                if (findEntry(name_raw)) |idx| {
                    // Derive a send-only cap (rights=1=EndpointSend) for the caller.
                    var derived_cap: u64 = 0;
                    const derive_err = libvanta.vanta_cap_derive(
                        registry.items[idx].endpoint_cap,
                        1,
                        @intFromPtr(&derived_cap),
                    );
                    if (derive_err != 0 or derived_cap == 0) {
                        sendErrorReply(EPERM);
                        continue;
                    }

                    var reply = Message{};
                    reply.msg_type = MSG_REGISTRY_LOOKUP;
                    reply.flags.is_reply = true;
                    reply.caps[0] = derived_cap;
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                } else {
                    sendErrorReply(ENOENT);
                }
            },

            // ── RegistryList ────────────────────────────────────────────
            MSG_REGISTRY_LIST => {
                // Build a newline-separated list in a dynamically allocated buffer,
                // then copy it into a SharedMemory cap for the caller.
                var total_len: usize = 0;
                for (registry.items) |e| {
                    total_len += e.name.len + 1; // name + '\n'
                }

                const n_pages: u64 = if (total_len == 0) 1 else (total_len + 4095) / 4096;
                const shm_res = libvanta.vanta_mem_create(n_pages);
                if (shm_res.err != 0) {
                    sendErrorReply(EPERM);
                    continue;
                }

                const shm_vaddr: u64 = 0x40000000;
                const map_err = libvanta.vanta_mem_map(shm_res.handle, shm_vaddr, n_pages);
                if (map_err != 0) {
                    sendErrorReply(EPERM);
                    continue;
                }

                const buf: [*]u8 = @ptrFromInt(shm_vaddr);
                var offset: usize = 0;
                for (registry.items) |e| {
                    @memcpy(buf[offset .. offset + e.name.len], e.name);
                    offset += e.name.len;
                    buf[offset] = '\n';
                    offset += 1;
                }

                _ = libvanta.vanta_mem_unmap(shm_vaddr);

                var reply = Message{};
                reply.msg_type = MSG_REGISTRY_LIST;
                reply.flags.is_reply = true;
                std.mem.writeInt(u64, reply.payload[0..8], @as(u64, total_len), .little);
                reply.caps[0] = shm_res.handle;
                _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
            },

            else => {
                sendErrorReply(EINVAL);
            },
        }
    }
}
