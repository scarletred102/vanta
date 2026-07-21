// ============================================================================
// VantaOS Tools — mkfs.vantafs Formatter Tool
// ============================================================================

const std = @import("std");

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
    reserved: [64]u8 = [_]u8{0} ** 64,
};

pub const DirEntry = extern struct {
    ino: u64,
    is_dir: u8,
    name_len: u8,
    name: [62]u8,
};

pub fn main() !void {
    const allocator = std.heap.page_allocator;
    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);

    if (args.len < 2) {
        std.debug.print("Usage: mkfs_vantafs <disk_image_path>\n", .{});
        std.process.exit(1);
    }

    const disk_path = args[1];
    std.debug.print("Formatting {s} as VantaFS...\n", .{disk_path});

    const file = try std.fs.cwd().createFile(disk_path, .{ .read = true });
    defer file.close();

    // 1. Write Superblock at sector 0
    var sb = Superblock{};
    var sb_bytes = [_]u8{0} ** 512;
    @memcpy(sb_bytes[0..@sizeOf(Superblock)], std.mem.asBytes(&sb));
    try file.writeAll(&sb_bytes);

    // Write dummy sector 1 to make superblock block-aligned
    var zero_sector = [_]u8{0} ** 512;
    try file.writeAll(&zero_sector);

    // 2. Write Block Bitmap at sector 2
    // Marks first 6 blocks (Superblock, Bitmap, Inode Table sectors 8-39) as used.
    // 6 blocks = 6 bits set to 1 -> 0x3F in first byte of bitmap
    var bitmap = [_]u8{0} ** 512;
    bitmap[0] = 0x3F;
    try file.writeAll(&bitmap);

    // Write dummy sectors 3 to 7
    var i: usize = 3;
    while (i < 8) : (i += 1) {
        try file.writeAll(&zero_sector);
    }

    // 3. Write Inode Table at sector 8 (32 sectors)
    // First inode (0) is the root directory
    var root_inode = Inode{
        .in_type = 2, // Directory
        .size = 4096, // 1 block
        .direct = [_]u32{0} ** 12,
        .indirect = 0,
    };
    root_inode.direct[0] = 5; // First data block starts at block index 5 (sector 40)

    var inode_table = [_]u8{0} ** (128 * 32); // 128 inodes * 128 bytes = 16384 bytes
    @memcpy(inode_table[0..@sizeOf(Inode)], std.mem.asBytes(&root_inode));
    try file.writeAll(&inode_table);

    // 4. Write Root Directory Data Block at sector 40 (8 sectors)
    var root_dir_block = [_]u8{0} ** 4096;
    
    // "." entry
    var dot = DirEntry{
        .ino = 0,
        .is_dir = 1,
        .name_len = 1,
        .name = [_]u8{0} ** 62,
    };
    dot.name[0] = '.';
    @memcpy(root_dir_block[0..@sizeOf(DirEntry)], std.mem.asBytes(&dot));

    // ".." entry
    var dotdot = DirEntry{
        .ino = 0,
        .is_dir = 1,
        .name_len = 2,
        .name = [_]u8{0} ** 62,
    };
    dotdot.name[0] = '.';
    dotdot.name[1] = '.';
    @memcpy(root_dir_block[@sizeOf(DirEntry) .. @sizeOf(DirEntry) * 2], std.mem.asBytes(&dotdot));

    try file.writeAll(&root_dir_block);

    std.debug.print("VantaFS formatted successfully!\n", .{});
}
