// ============================================================================
// VantaOS Userspace — VantaFS On-Disk Filesystem Server
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

pub const MSG_BLOCK_READ: u32 = 0x0401;
pub const MSG_BLOCK_WRITE: u32 = 0x0402;
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

// ── VantaFS Structure Definitions ─────────────────────────────────

pub const Superblock = extern struct {
    magic: u64 = 0x56414E5441465300, // "VANTAFS\x00"
    block_size: u32 = 4096,
    inode_table_lba: u64 = 8,
    root_inode_index: u32 = 0,
    block_bitmap_lba: u64 = 2,
};

pub const Inode = extern struct {
    in_type: u32,             // 0 = free, 1 = file, 2 = dir
    size: u64,
    direct: [12]u32,
    indirect: u32,
    reserved: [60]u8 = [_]u8{0} ** 60,
};

pub const DirEntry = extern struct {
    ino: u64,
    is_dir: u8,
    name_len: u8,
    name: [62]u8,
};

const FileSession = struct {
    id: u64,
    ino: u64,
    offset: u64,
    flags: u32,
    active: bool,
};

var block_provider_cap: u64 = 0;
var sessions: [512]FileSession = [_]FileSession{.{ .id = 0, .ino = 0, .offset = 0, .flags = 0, .active = false }} ** 512;
var next_session_id: u64 = 1;

// ── Service Discovery ─────────────────────────────────────────────

fn vanta_registry_lookup(name: []const u8) ?u64 {
    var msg = Message{};
    msg.msg_type = 0x11; // RegistryLookup
    msg.flags.expects_reply = true;
    @memcpy(msg.payload[0..@min(name.len, 31)], name[0..@min(name.len, 31)]);
    
    var reply = Message{};
    const err = libvanta.vanta_cap_call(REGISTRY_CAP_HANDLE, @intFromPtr(&msg), @intFromPtr(&reply));
    if (err == 0 and reply.msg_type == 0x11 and reply.caps[0] != 0) {
        return reply.caps[0];
    }
    return null;
}

// ── Block Reading / Writing ───────────────────────────────────────

var mock_disk: [128][4096]u8 = undefined;
var mock_disk_initialized: bool = false;

// ── Block Reading / Writing ───────────────────────────────────────

fn readBlock(block_idx: u32, buf: []u8) !void {
    if (block_provider_cap == 0) {
        if (!mock_disk_initialized) {
            mock_disk_initialized = true;
            @memset(std.mem.asBytes(&mock_disk), 0);
            
            // Format mock disk with VantaFS Superblock and root directory
            var sb = Superblock{};
            @memcpy(mock_disk[0][0..@sizeOf(Superblock)], std.mem.asBytes(&sb));
            
            // Block Bitmap at sector 2 (block 0 offset 1024)
            // Marks first 6 blocks (Superblock, Bitmap, Inode table blocks 1..4, root data block 5)
            mock_disk[0][1024] = 0x3F;
            
            // Inode 0 is root dir (type = 2, size = 4096, points to block 5)
            var root_inode = Inode{
                .in_type = 2,
                .size = 4096,
                .direct = [_]u32{0} ** 12,
                .indirect = 0,
            };
            root_inode.direct[0] = 5;
            @memcpy(mock_disk[1][0..@sizeOf(Inode)], std.mem.asBytes(&root_inode));
            
            // Root dir entries '.' and '..'
            var dot = DirEntry{ .ino = 0, .is_dir = 1, .name_len = 1, .name = [_]u8{0} ** 62 };
            dot.name[0] = '.';
            @memcpy(mock_disk[5][0..@sizeOf(DirEntry)], std.mem.asBytes(&dot));
            
            var dotdot = DirEntry{ .ino = 0, .is_dir = 1, .name_len = 2, .name = [_]u8{0} ** 62 };
            dotdot.name[0] = '.';
            dotdot.name[1] = '.';
            @memcpy(mock_disk[5][@sizeOf(DirEntry)..@sizeOf(DirEntry)*2], std.mem.asBytes(&dotdot));
        }
        
        if (block_idx < 128) {
            @memcpy(buf[0..4096], mock_disk[block_idx][0..4096]);
            return;
        }
        return error.BlockReadFailed;
    }

    const mem_res = libvanta.vanta_mem_create(1);
    if (mem_res.err != 0) return error.MemCreateFailed;
    const shm_cap = mem_res.handle;
    
    const map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, 1);
    if (map_err != 0) {
        _ = libvanta.vanta_cap_revoke(shm_cap);
        return error.MemMapFailed;
    }
    
    var msg = Message{};
    msg.msg_type = MSG_BLOCK_READ;
    msg.flags.expects_reply = true;
    std.mem.writeInt(u64, msg.payload[0..8], @as(u64, block_idx) * 8, .little);
    std.mem.writeInt(u64, msg.payload[8..16], 8, .little);
    msg.buffer_cap = shm_cap; // moved
    
    var reply = Message{};
    const call_err = libvanta.vanta_cap_call(block_provider_cap, @intFromPtr(&msg), @intFromPtr(&reply));
    if (call_err != 0 or reply.msg_type == MSG_ERROR) {
        _ = libvanta.vanta_mem_unmap(SHM_VADDR);
        return error.BlockReadFailed;
    }
    
    const shm_ptr: [*]const u8 = @ptrFromInt(SHM_VADDR);
    @memcpy(buf[0..4096], shm_ptr[0..4096]);
    
    _ = libvanta.vanta_mem_unmap(SHM_VADDR);
}

fn writeBlock(block_idx: u32, buf: []const u8) !void {
    if (block_provider_cap == 0) {
        if (!mock_disk_initialized) {
            var dummy: [4096]u8 = undefined;
            try readBlock(0, &dummy);
        }
        if (block_idx < 128) {
            @memcpy(mock_disk[block_idx][0..4096], buf[0..4096]);
            return;
        }
        return error.BlockWriteFailed;
    }

    const mem_res = libvanta.vanta_mem_create(1);
    if (mem_res.err != 0) return error.MemCreateFailed;
    const shm_cap = mem_res.handle;
    
    const map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, 1);
    if (map_err != 0) {
        _ = libvanta.vanta_cap_revoke(shm_cap);
        return error.MemMapFailed;
    }
    
    const shm_ptr: [*]u8 = @ptrFromInt(SHM_VADDR);
    @memcpy(shm_ptr[0..4096], buf[0..4096]);
    
    var msg = Message{};
    msg.msg_type = MSG_BLOCK_WRITE;
    msg.flags.expects_reply = true;
    std.mem.writeInt(u64, msg.payload[0..8], @as(u64, block_idx) * 8, .little);
    std.mem.writeInt(u64, msg.payload[8..16], 8, .little);
    msg.buffer_cap = shm_cap; // moved
    
    var reply = Message{};
    const call_err = libvanta.vanta_cap_call(block_provider_cap, @intFromPtr(&msg), @intFromPtr(&reply));
    if (call_err != 0 or reply.msg_type == MSG_ERROR) {
        _ = libvanta.vanta_mem_unmap(SHM_VADDR);
        return error.BlockWriteFailed;
    }
    
    _ = libvanta.vanta_mem_unmap(SHM_VADDR);
}

// ── Inode Table Reading / Writing ─────────────────────────────────

fn readInode(ino: u32, inode: *Inode) !void {
    const block_idx = 1 + (ino * 128) / 4096;
    const block_offset = (ino * 128) % 4096;
    
    var buf = [_]u8{0} ** 4096;
    try readBlock(block_idx, &buf);
    
    @memcpy(std.mem.asBytes(inode), buf[block_offset..block_offset + 128]);
}

fn writeInode(ino: u32, inode: *const Inode) !void {
    const block_idx = 1 + (ino * 128) / 4096;
    const block_offset = (ino * 128) % 4096;
    
    var buf = [_]u8{0} ** 4096;
    try readBlock(block_idx, &buf);
    
    @memcpy(buf[block_offset..block_offset + 128], std.mem.asBytes(inode));
    try writeBlock(block_idx, &buf);
}

// ── Bitmap & Inode Allocation ─────────────────────────────────────

fn allocBlock() !u32 {
    var buf = [_]u8{0} ** 4096;
    try readBlock(0, &buf);
    
    const bitmap = buf[1024..1536]; // 512 bytes = 4096 bits
    
    var block_idx: u32 = 0;
    while (block_idx < 4096) : (block_idx += 1) {
        const byte = block_idx / 8;
        const bit = @as(u32, 1) << @as(u5, @truncate(block_idx % 8));
        if ((bitmap[byte] & bit) == 0) {
            bitmap[byte] |= @truncate(bit);
            try writeBlock(0, &buf);
            return block_idx;
        }
    }
    return error.DiskFull;
}

fn freeBlock(block_idx: u32) !void {
    var buf = [_]u8{0} ** 4096;
    try readBlock(0, &buf);
    
    const bitmap = buf[1024..1536];
    const byte = block_idx / 8;
    const bit = @as(u32, 1) << @as(u5, @truncate(block_idx % 8));
    bitmap[byte] &= ~@as(u8, @truncate(bit));
    try writeBlock(0, &buf);
}

fn allocInode(in_type: u32) !u32 {
    var inode = Inode{
        .in_type = 0,
        .size = 0,
        .direct = [_]u32{0} ** 12,
        .indirect = 0,
    };
    
    var ino: u32 = 1;
    while (ino < 128) : (ino += 1) {
        try readInode(ino, &inode);
        if (inode.in_type == 0) {
            inode.in_type = in_type;
            inode.size = 0;
            inode.direct = [_]u32{0} ** 12;
            inode.indirect = 0;
            try writeInode(ino, &inode);
            return ino;
        }
    }
    return error.NoFreeInodes;
}

fn freeInode(ino: u32) !void {
    var inode = Inode{
        .in_type = 0,
        .size = 0,
        .direct = [_]u32{0} ** 12,
        .indirect = 0,
    };
    try writeInode(ino, &inode);
}

// ── Inode Block Mapping ───────────────────────────────────────────

fn getOrAllocBlock(inode: *Inode, ino: u32, block_offset: u32) !u32 {
    if (block_offset < 12) {
        if (inode.direct[block_offset] == 0) {
            const b = try allocBlock();
            inode.direct[block_offset] = b;
            try writeInode(ino, inode);
            return b;
        }
        return inode.direct[block_offset];
    } else {
        const ind_block = block_offset - 12;
        if (inode.indirect == 0) {
            const b = try allocBlock();
            inode.indirect = b;
            var zeros = [_]u8{0} ** 4096;
            try writeBlock(b, &zeros);
            try writeInode(ino, inode);
        }
        
        var ind_buf = [_]u8{0} ** 4096;
        try readBlock(inode.indirect, &ind_buf);
        
        const ptrs = @as([*]u32, @ptrCast(@alignCast(&ind_buf)));
        if (ptrs[ind_block] == 0) {
            const b = try allocBlock();
            ptrs[ind_block] = b;
            try writeBlock(inode.indirect, &ind_buf);
            return b;
        }
        return ptrs[ind_block];
    }
}

// ── Directory Operations ──────────────────────────────────────────

fn findDirEntry(dir_ino: u32, name: []const u8) !?struct { ino: u32, is_dir: bool } {
    var inode = Inode{ .in_type = 0, .size = 0, .direct = [_]u32{0} ** 12, .indirect = 0 };
    try readInode(dir_ino, &inode);
    
    const num_blocks = (inode.size + 4095) / 4096;
    var block_idx: u32 = 0;
    while (block_idx < num_blocks) : (block_idx += 1) {
        const b = try getOrAllocBlock(&inode, dir_ino, block_idx);
        var buf = [_]u8{0} ** 4096;
        try readBlock(b, &buf);
        
        var offset: usize = 0;
        while (offset + 72 <= 4096) : (offset += 72) {
            const entry = @as(*const DirEntry, @ptrCast(@alignCast(&buf[offset])));
            if (entry.name_len > 0) {
                const entry_name = entry.name[0..entry.name_len];
                if (std.mem.eql(u8, entry_name, name)) {
                    return .{ .ino = @truncate(entry.ino), .is_dir = entry.is_dir == 1 };
                }
            }
        }
    }
    return null;
}

fn addDirEntry(dir_ino: u32, name: []const u8, child_ino: u32, is_dir: bool) !void {
    var inode = Inode{ .in_type = 0, .size = 0, .direct = [_]u32{0} ** 12, .indirect = 0 };
    try readInode(dir_ino, &inode);
    
    const num_blocks = (inode.size + 4095) / 4096;
    var block_idx: u32 = 0;
    while (block_idx < num_blocks) : (block_idx += 1) {
        const b = try getOrAllocBlock(&inode, dir_ino, block_idx);
        var buf = [_]u8{0} ** 4096;
        try readBlock(b, &buf);
        
        var offset: usize = 0;
        while (offset + 72 <= 4096) : (offset += 72) {
            const entry = @as(*DirEntry, @ptrCast(@alignCast(&buf[offset])));
            if (entry.name_len == 0) {
                entry.ino = child_ino;
                entry.is_dir = if (is_dir) 1 else 0;
                entry.name_len = @truncate(name.len);
                @memset(&entry.name, 0);
                @memcpy(entry.name[0..name.len], name);
                try writeBlock(b, &buf);
                return;
            }
        }
    }
    
    // Expand directory block
    const new_block_idx = @as(u32, @truncate(num_blocks));
    const b = try getOrAllocBlock(&inode, dir_ino, new_block_idx);
    var buf = [_]u8{0} ** 4096;
    
    const entry = @as(*DirEntry, @ptrCast(@alignCast(&buf[0])));
    entry.ino = child_ino;
    entry.is_dir = if (is_dir) 1 else 0;
    entry.name_len = @truncate(name.len);
    @memset(&entry.name, 0);
    @memcpy(entry.name[0..name.len], name);
    
    try writeBlock(b, &buf);
    
    inode.size += 4096;
    try writeInode(dir_ino, &inode);
}

fn removeDirEntry(dir_ino: u32, name: []const u8) !void {
    var inode = Inode{ .in_type = 0, .size = 0, .direct = [_]u32{0} ** 12, .indirect = 0 };
    try readInode(dir_ino, &inode);
    
    const num_blocks = (inode.size + 4095) / 4096;
    var block_idx: u32 = 0;
    while (block_idx < num_blocks) : (block_idx += 1) {
        const b = try getOrAllocBlock(&inode, dir_ino, block_idx);
        var buf = [_]u8{0} ** 4096;
        try readBlock(b, &buf);
        
        var offset: usize = 0;
        while (offset + 72 <= 4096) : (offset += 72) {
            const entry = @as(*DirEntry, @ptrCast(@alignCast(&buf[offset])));
            if (entry.name_len > 0) {
                const entry_name = entry.name[0..entry.name_len];
                if (std.mem.eql(u8, entry_name, name)) {
                    entry.name_len = 0;
                    entry.ino = 0;
                    try writeBlock(b, &buf);
                    return;
                }
            }
        }
    }
}

// ── Path Resolution ───────────────────────────────────────────────

fn resolvePathToInode(path: []const u8) !?u32 {
    if (path.len == 0 or std.mem.eql(u8, path, "/")) {
        return 0; // Root directory is inode 0
    }
    
    var current_ino: u32 = 0;
    var it = std.mem.tokenizeAny(u8, path, "/");
    while (it.next()) |part| {
        if (try findDirEntry(current_ino, part)) |res| {
            current_ino = res.ino;
        } else {
            return null;
        }
    }
    return current_ino;
}

fn createFileAtReference(path: []const u8, is_dir: bool) !u32 {
    var parent_path: []const u8 = "";
    var name: []const u8 = path;
    if (std.mem.lastIndexOfScalar(u8, path, '/')) |idx| {
        parent_path = path[0..idx];
        name = path[idx + 1 ..];
    }
    
    const parent_ino = (try resolvePathToInode(parent_path)) orelse return error.ParentNotFound;
    
    const new_ino = try allocInode(if (is_dir) 2 else 1);
    try addDirEntry(parent_ino, name, new_ino, is_dir);
    
    if (is_dir) {
        var dir_inode = Inode{ .in_type = 2, .size = 4096, .direct = [_]u32{0} ** 12, .indirect = 0 };
        const b = try getOrAllocBlock(&dir_inode, new_ino, 0);
        var buf = [_]u8{0} ** 4096;
        
        var dot = DirEntry{ .ino = new_ino, .is_dir = 1, .name_len = 1, .name = [_]u8{0} ** 62 };
        dot.name[0] = '.';
        @memcpy(buf[0..72], std.mem.asBytes(&dot));
        
        var dotdot = DirEntry{ .ino = parent_ino, .is_dir = 1, .name_len = 2, .name = [_]u8{0} ** 62 };
        dotdot.name[0] = '.';
        dotdot.name[1] = '.';
        @memcpy(buf[72..144], std.mem.asBytes(&dotdot));
        
        try writeBlock(b, &buf);
        try writeInode(new_ino, &dir_inode);
    }
    
    return new_ino;
}

// ── Main IPC Loop ─────────────────────────────────────────────────

pub export fn main() void {
    libvanta.vanta_debug_print("vantafs: Starting VantaFS userspace driver server...");

    // Setup Block Provider
    libvanta.vanta_debug_print("vantafs: Looking up block provider 'block.ahci.0p0'...");
    var block_cap: u64 = 0;
    var timeout: usize = 0;
    while (timeout < 1000) : (timeout += 1) {
        if (vanta_registry_lookup("block.ahci.0p0")) |cap| {
            block_cap = cap;
            break;
        }
        // Yield/delay
        var delay: u64 = 0;
        while (delay < 1_000_000) : (delay += 1) {
            asm volatile ("pause");
        }
    }

    if (block_cap == 0) {
        libvanta.vanta_debug_print("vantafs: Dry-run / no block provider found. Fallback to mock block IO.");
    } else {
        block_provider_cap = block_cap;
        libvanta.vanta_debug_print("vantafs: Bound to partition block provider successfully!");
    }

    // Registry Registration
    libvanta.vanta_debug_print("vantafs: Registering with service registry...");
    var derived_port: u64 = 0;
    const derive_err = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 3, @intFromPtr(&derived_port));
    if (derive_err == 0) {
        var reg_msg = Message{};
        reg_msg.msg_type = 0x10; // RegistryRegister
        @memcpy(reg_msg.payload[0..10], "fs.vantafs");
        reg_msg.caps[0] = derived_port;
        _ = libvanta.vanta_cap_send(REGISTRY_CAP_HANDLE, @intFromPtr(&reg_msg));
    }

    libvanta.vanta_debug_print("vantafs: Entering IPC service loop...");
    while (true) {
        var msg = Message{};
        const recv_err = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&msg));
        if (recv_err != 0) continue;

        switch (msg.msg_type) {
            MSG_FS_OPEN => {
                const flags = std.mem.readInt(u32, msg.payload[0..4], .little);
                const path = std.mem.sliceTo(msg.payload[4..64], 0);

                var ino = resolvePathToInode(path) catch null;
                if (ino == null and (flags & 8) != 0) { // O_CREAT
                    ino = createFileAtReference(path, false) catch null;
                }

                if (ino == null) {
                    sendErrorReply(&msg);
                    continue;
                }

                // Allocate a session
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
                        .ino = ino.?,
                        .offset = 0,
                        .flags = flags,
                        .active = true,
                    };

                    var reply = Message{};
                    reply.msg_type = MSG_FS_OPEN;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], sid, .little);

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

                const pages_to_map = (len + 4095) / 4096;
                const shm_map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, pages_to_map);
                if (shm_map_err != 0) {
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                const shm_ptr: [*]u8 = @ptrFromInt(SHM_VADDR);
                
                var inode = Inode{ .in_type = 0, .size = 0, .direct = [_]u32{0} ** 12, .indirect = 0 };
                readInode(@truncate(session.?.ino), &inode) catch {
                    _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                };

                const bytes_to_read = if (offset >= inode.size) 0 else @min(len, inode.size - offset);
                
                var bytes_read: u64 = 0;
                while (bytes_read < bytes_to_read) {
                    const curr_offset = offset + bytes_read;
                    const block_idx = @as(u32, @truncate(curr_offset / 4096));
                    const block_offset = curr_offset % 4096;
                    const block_bytes = @min(4096 - block_offset, bytes_to_read - bytes_read);
                    
                    const b = getOrAllocBlock(&inode, @truncate(session.?.ino), block_idx) catch break;
                    var block_buf = [_]u8{0} ** 4096;
                    readBlock(b, &block_buf) catch break;
                    
                    @memcpy(shm_ptr[bytes_read .. bytes_read + block_bytes], block_buf[block_offset .. block_offset + block_bytes]);
                    bytes_read += block_bytes;
                }

                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_FS_READ;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], bytes_read, .little);
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

                const pages_to_map = (len + 4095) / 4096;
                const shm_map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, pages_to_map);
                if (shm_map_err != 0) {
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                const shm_ptr: [*]const u8 = @ptrFromInt(SHM_VADDR);
                
                var inode = Inode{ .in_type = 0, .size = 0, .direct = [_]u32{0} ** 12, .indirect = 0 };
                readInode(@truncate(session.?.ino), &inode) catch {
                    _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                };

                var bytes_written: u64 = 0;
                while (bytes_written < len) {
                    const curr_offset = offset + bytes_written;
                    const block_idx = @as(u32, @truncate(curr_offset / 4096));
                    const block_offset = curr_offset % 4096;
                    const block_bytes = @min(4096 - block_offset, len - bytes_written);
                    
                    const b = getOrAllocBlock(&inode, @truncate(session.?.ino), block_idx) catch break;
                    var block_buf = [_]u8{0} ** 4096;
                    if (block_offset > 0 or block_bytes < 4096) {
                        readBlock(b, &block_buf) catch {};
                    }
                    
                    @memcpy(block_buf[block_offset .. block_offset + block_bytes], shm_ptr[bytes_written .. bytes_written + block_bytes]);
                    writeBlock(b, &block_buf) catch break;
                    
                    bytes_written += block_bytes;
                }

                if (offset + bytes_written > inode.size) {
                    inode.size = offset + bytes_written;
                    writeInode(@truncate(session.?.ino), &inode) catch {};
                }

                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_FS_WRITE;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], bytes_written, .little);
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
                if (resolvePathToInode(path) catch null) |ino| {
                    var inode = Inode{ .in_type = 0, .size = 0, .direct = [_]u32{0} ** 12, .indirect = 0 };
                    readInode(ino, &inode) catch {};
                    
                    var reply = Message{};
                    reply.msg_type = MSG_FS_STAT;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], inode.size, .little);
                    reply.payload[8] = if (inode.in_type == 2) 1 else 0;
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

                const shm_map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, 1);
                if (shm_map_err != 0) {
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                var inode = Inode{ .in_type = 0, .size = 0, .direct = [_]u32{0} ** 12, .indirect = 0 };
                readInode(@truncate(session.?.ino), &inode) catch {
                    _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                };

                const shm_ptr = @as([*]DirEntry, @ptrFromInt(SHM_VADDR));
                var entry_count: u64 = 0;

                const num_blocks = (inode.size + 4095) / 4096;
                var current_offset: u64 = 0;
                var block_idx: u32 = 0;
                
                outer: while (block_idx < num_blocks) : (block_idx += 1) {
                    const b = getOrAllocBlock(&inode, @truncate(session.?.ino), block_idx) catch break;
                    var buf = [_]u8{0} ** 4096;
                    readBlock(b, &buf) catch break;
                    
                    var offset_in_block: usize = 0;
                    while (offset_in_block + 72 <= 4096) : (offset_in_block += 72) {
                        const entry = @as(*const DirEntry, @ptrCast(@alignCast(&buf[offset_in_block])));
                        if (entry.name_len > 0) {
                            if (current_offset >= offset) {
                                var res_entry = DirEntry{
                                    .ino = entry.ino,
                                    .is_dir = entry.is_dir,
                                    .name_len = entry.name_len,
                                    .name = [_]u8{0} ** 62,
                                };
                                @memcpy(res_entry.name[0..@min(entry.name_len, 61)], entry.name[0..@min(entry.name_len, 61)]);
                                shm_ptr[entry_count] = res_entry;
                                entry_count += 1;
                                if (entry_count >= 50) break :outer;
                            }
                            current_offset += 1;
                        }
                    }
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
                const ino = createFileAtReference(path, true) catch null;
                if (ino != null) {
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

                if (resolvePathToInode(parent_path) catch null) |parent_ino| {
                    if (findDirEntry(parent_ino, name) catch null) |child| {
                        removeDirEntry(parent_ino, name) catch {};
                        freeInode(child.ino) catch {};
                        
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

                if (resolvePathToInode(src) catch null) |src_ino| {
                    var parent_path: []const u8 = "";
                    var name: []const u8 = dst;
                    if (std.mem.lastIndexOfScalar(u8, dst, '/')) |idx| {
                        parent_path = dst[0..idx];
                        name = dst[idx + 1 ..];
                    }

                    if (resolvePathToInode(parent_path) catch null) |new_parent_ino| {
                        // Unlink from old parent
                        var old_parent_path: []const u8 = "";
                        var old_name: []const u8 = src;
                        if (std.mem.lastIndexOfScalar(u8, src, '/')) |idx| {
                            old_parent_path = src[0..idx];
                            old_name = src[idx + 1 ..];
                        }
                        if (resolvePathToInode(old_parent_path) catch null) |old_parent_ino| {
                            removeDirEntry(old_parent_ino, old_name) catch {};
                        }

                        // Add to new parent
                        var inode = Inode{ .in_type = 0, .size = 0, .direct = [_]u32{0} ** 12, .indirect = 0 };
                        readInode(src_ino, &inode) catch {};
                        
                        addDirEntry(new_parent_ino, name, src_ino, inode.in_type == 2) catch {};

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
