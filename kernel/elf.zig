// ============================================================================
// VantaOS — ELF64 Parser & Loader (Phase 4)
// ============================================================================

const std = @import("std");
const pmm = @import("mm/pmm.zig");
const vmm = @import("mm/vmm.zig");
const serial = @import("arch/x86_64/serial.zig");

pub const LoadSegment = struct {
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    flags: u32,
    offset: u64,
};

pub const ElfInfo = struct {
    entry: u64,
    segments: [8]LoadSegment,
    segment_count: usize,
};

pub const ElfError = error{
    InvalidMagic,
    InvalidClass,
    InvalidEndian,
    InvalidMachine,
    InvalidType,
    SegmentNotPageAligned,
    OverlappingSegments,
    OutOfMemory,
    SecurityViolation,
};

pub fn parse_elf64(data: []const u8) ElfError!ElfInfo {
    if (data.len < 64) return ElfError.InvalidMagic;
    if (!std.mem.eql(u8, data[0..4], "\x7fELF")) return ElfError.InvalidMagic;
    if (data[4] != 2) return ElfError.InvalidClass;
    if (data[5] != 1) return ElfError.InvalidEndian;

    const type_val = std.mem.readInt(u16, data[16..18], .little);
    if (type_val != 2 and type_val != 3) return ElfError.InvalidType;

    const machine = std.mem.readInt(u16, data[18..20], .little);
    if (machine != 0x3e) return ElfError.InvalidMachine;

    const entry = std.mem.readInt(u64, data[24..32], .little);
    const phoff = std.mem.readInt(u64, data[32..40], .little);
    const phnum = std.mem.readInt(u16, data[56..58], .little);

    var info = ElfInfo{
        .entry = entry,
        .segments = undefined,
        .segment_count = 0,
    };

    var ph_idx: usize = 0;
    while (ph_idx < phnum) : (ph_idx += 1) {
        const ph_offset = phoff + ph_idx * 56;
        if (ph_offset + 56 > data.len) return ElfError.InvalidMagic;
        
        const p_type = std.mem.readInt(u32, data[ph_offset..][0..4], .little);
        if (p_type == 1) {
            if (info.segment_count >= 8) return ElfError.OutOfMemory;
            
            const p_flags = std.mem.readInt(u32, data[ph_offset+4..][0..4], .little);
            const p_offset = std.mem.readInt(u64, data[ph_offset+8..][0..8], .little);
            const p_vaddr = std.mem.readInt(u64, data[ph_offset+16..][0..8], .little);
            const p_filesz = std.mem.readInt(u64, data[ph_offset+32..][0..8], .little);
            const p_memsz = std.mem.readInt(u64, data[ph_offset+40..][0..8], .little);

            if (p_vaddr & 0xFFF != 0) return ElfError.SegmentNotPageAligned;
            if ((p_flags & 2 != 0) and (p_flags & 1 != 0)) return ElfError.SecurityViolation;

            info.segments[info.segment_count] = .{
                .vaddr = p_vaddr,
                .filesz = p_filesz,
                .memsz = p_memsz,
                .flags = p_flags,
                .offset = p_offset,
            };
            info.segment_count += 1;
        }
    }

    var i: usize = 0;
    while (i < info.segment_count) : (i += 1) {
        var j: usize = i + 1;
        while (j < info.segment_count) : (j += 1) {
            const s1 = &info.segments[i];
            const s2 = &info.segments[j];
            const s1_end = s1.vaddr + s1.memsz;
            const s2_end = s2.vaddr + s2.memsz;
            if (s1.vaddr < s2_end and s2.vaddr < s1_end) {
                return ElfError.OverlappingSegments;
            }
        }
    }

    return info;
}

pub fn load_elf(elf: ElfInfo, data: []const u8, page_table: u64) ElfError!u64 {
    const space = vmm.AddressSpace{ .pml4_phys = page_table };
    
    var seg_idx: usize = 0;
    while (seg_idx < elf.segment_count) : (seg_idx += 1) {
        const seg = &elf.segments[seg_idx];
        const num_pages = (seg.memsz + 4095) / 4096;

        var page_idx: usize = 0;
        while (page_idx < num_pages) : (page_idx += 1) {
            const page_vaddr = seg.vaddr + page_idx * 4096;
            const paddr = pmm.allocPage() orelse {
                return ElfError.OutOfMemory;
            };

            const page_virt = vmm.phys2virt(paddr);
            @memset(@as([*]u8, @ptrFromInt(page_virt))[0..4096], 0);

            const page_offset = page_idx * 4096;
            if (page_offset < seg.filesz) {
                const copy_len = @min(4096, seg.filesz - page_offset);
                @memcpy(
                    @as([*]u8, @ptrFromInt(page_virt))[0..copy_len],
                    data[seg.offset + page_offset .. seg.offset + page_offset + copy_len]
                );
            }

            var pte_flags: u64 = vmm.PTE_USER;
            if (seg.flags & 2 != 0) pte_flags |= vmm.PTE_WRITE;
            if (seg.flags & 1 == 0) pte_flags |= vmm.PTE_NX;

            if (!vmm.map(space, page_vaddr, paddr, pte_flags)) {
                pmm.freePage(paddr);
                return ElfError.OutOfMemory;
            }
        }
    }

    return elf.entry;
}

pub fn writeUserStackU64(new_pml4: u64, vaddr: u64, val: u64) void {
    const phys = vmm.translate(vmm.AddressSpace{ .pml4_phys = new_pml4 }, vaddr).?;
    const virt = vmm.phys2virt(phys);
    @as(*u64, @ptrFromInt(virt)).* = val;
}

pub fn writeUserStackBytes(new_pml4: u64, vaddr: u64, bytes: []const u8) void {
    var offset: usize = 0;
    while (offset < bytes.len) : (offset += 1) {
        const phys = vmm.translate(vmm.AddressSpace{ .pml4_phys = new_pml4 }, vaddr + offset).?;
        const virt = vmm.phys2virt(phys);
        @as(*u8, @ptrFromInt(virt)).* = bytes[offset];
    }
}

pub fn setupUserStack(new_pml4: u64, user_entry: u64, elf_info: ElfInfo) u64 {
    const rand_addr = 0x7FFF00000000 - 16;
    var rand_bytes: [16]u8 = undefined;
    var r: u64 = 0x123456789ABCDEF0;
    for (0..16) |idx| {
        r = r *% 6364136223846793005 +% 1442695040888963407;
        rand_bytes[idx] = @truncate(r);
    }
    writeUserStackBytes(new_pml4, rand_addr, &rand_bytes);

    const aux_start = 0x7FFF00000000 - 112;
    writeUserStackU64(new_pml4, aux_start + 0, 25); // AT_RANDOM
    writeUserStackU64(new_pml4, aux_start + 8, rand_addr);
    writeUserStackU64(new_pml4, aux_start + 16, 9); // AT_ENTRY
    writeUserStackU64(new_pml4, aux_start + 24, user_entry);
    writeUserStackU64(new_pml4, aux_start + 32, 6); // AT_PAGESZ
    writeUserStackU64(new_pml4, aux_start + 40, 4096);
    const phdr_addr = if (elf_info.segment_count > 0) elf_info.segments[0].vaddr + 64 else 0;
    writeUserStackU64(new_pml4, aux_start + 48, 3); // AT_PHDR
    writeUserStackU64(new_pml4, aux_start + 56, phdr_addr);
    writeUserStackU64(new_pml4, aux_start + 64, 5); // AT_PHNUM
    writeUserStackU64(new_pml4, aux_start + 72, elf_info.segment_count);
    writeUserStackU64(new_pml4, aux_start + 80, 0); // AT_NULL
    writeUserStackU64(new_pml4, aux_start + 88, 0);

    const envp_start = aux_start - 8; // 0x7FFF00000000 - 120
    writeUserStackU64(new_pml4, envp_start, 0);

    const argv_start = envp_start - 8; // 0x7FFF00000000 - 128
    writeUserStackU64(new_pml4, argv_start, 0);

    const argc_start = argv_start - 8; // 0x7FFF00000000 - 136
    writeUserStackU64(new_pml4, argc_start, 0);

    return argc_start;
}
