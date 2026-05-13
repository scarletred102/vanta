// ============================================================================
// VantaOS — Global Descriptor Table (GDT) + TSS
//
// Selector layout (chosen for SYSCALL/SYSRET compatibility):
//   0x00 — Null
//   0x08 — Kernel code (64-bit, ring 0)
//   0x10 — Kernel data (ring 0)
//   0x18 — User code (32-bit, unused — required slot for SYSRET)
//   0x20 — User data (ring 3)
//   0x28 — User code (64-bit, ring 3)
//   0x30 — TSS descriptor (low 8 bytes)
//   0x38 — TSS descriptor (high 8 bytes, 64-bit base extension)
//
// STAR MSR mapping for SYSCALL:
//   Kernel CS = 0x08, Kernel SS = 0x10  (STAR[47:32] = 0x08)
//   User CS   = 0x28, User SS   = 0x20  (STAR[63:48] = 0x1B — selects 0x18,
//                                        then +8 = 0x20 (SS), +16 = 0x28 (CS))
// ============================================================================

const serial = @import("serial.zig");
const tss = @import("tss.zig");

// ── GDT Entry (8 bytes) ─────────────────────────────────────────

const GdtEntry = packed struct(u64) {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    limit_hi_flags: u8, // [0:3] limit[16:19], [4:7] flags (AVL, L, D/B, G)
    base_high: u8,
};

fn entry(base: u32, limit: u32, access: u8, flags: u8) GdtEntry {
    return .{
        .limit_low = @truncate(limit & 0xFFFF),
        .base_low = @truncate(base & 0xFFFF),
        .base_mid = @truncate((base >> 16) & 0xFF),
        .access = access,
        .limit_hi_flags = @truncate(((limit >> 16) & 0x0F) | ((flags << 4) & 0xF0)),
        .base_high = @truncate((base >> 24) & 0xFF),
    };
}

// 64-bit TSS descriptor (16 bytes = 2 GDT slots)
const TssDescriptor = extern struct {
    limit_low: u16,
    base_low: u16,
    base_mid1: u8,
    access: u8,         // 0x89 = present, ring 0, available 64-bit TSS
    limit_hi_flags: u8, // [0:3] limit[16:19], [4:7] flags
    base_mid2: u8,
    base_upper: u32,    // base[32:63]
    reserved: u32 = 0,
};

comptime {
    if (@sizeOf(TssDescriptor) != 16) @compileError("TSS descriptor must be 16 bytes");
}

// ── Public Selectors ────────────────────────────────────────────

pub const KERNEL_CODE_SEL: u16 = 0x08;
pub const KERNEL_DATA_SEL: u16 = 0x10;
pub const USER_CODE32_SEL: u16 = 0x18;
pub const USER_DATA_SEL: u16 = 0x20;
pub const USER_CODE_SEL: u16 = 0x28;
pub const TSS_SEL: u16 = 0x30;

// ── GDT Table ───────────────────────────────────────────────────
//
// Layout in memory (7 logical slots, 8 actual entries since TSS takes 2):
//   [0] null
//   [1] kernel code  (0x08)
//   [2] kernel data  (0x10)
//   [3] user code 32 (0x18)
//   [4] user data    (0x20)
//   [5] user code 64 (0x28)
//   [6] TSS low      (0x30)
//   [7] TSS high     (0x38)

const GdtArray = extern struct {
    null_entry: GdtEntry,
    kernel_code: GdtEntry,
    kernel_data: GdtEntry,
    user_code32: GdtEntry,
    user_data: GdtEntry,
    user_code: GdtEntry,
    tss_desc: TssDescriptor,
};

var gdt: GdtArray align(8) = .{
    .null_entry  = entry(0, 0, 0, 0),
    // Access flags:
    //   P=1 DPL=00 S=1 E=1 RW=1 = 0x9A (kernel code)
    //   P=1 DPL=00 S=1 E=0 RW=1 = 0x92 (kernel data)
    //   P=1 DPL=11 S=1 E=1 RW=1 = 0xFA (user code)
    //   P=1 DPL=11 S=1 E=0 RW=1 = 0xF2 (user data)
    // Flags nibble:
    //   G=1 L=1 D=0 = 0xA  (64-bit code)
    //   G=1 D/B=1   = 0xC  (data / 32-bit code)
    .kernel_code = entry(0, 0xFFFFF, 0x9A, 0xA),
    .kernel_data = entry(0, 0xFFFFF, 0x92, 0xC),
    .user_code32 = entry(0, 0xFFFFF, 0xFA, 0xC),
    .user_data   = entry(0, 0xFFFFF, 0xF2, 0xC),
    .user_code   = entry(0, 0xFFFFF, 0xFA, 0xA),
    .tss_desc = .{
        .limit_low = 0,
        .base_low = 0,
        .base_mid1 = 0,
        .access = 0x89, // Present, DPL=0, available 64-bit TSS (type=0x9)
        .limit_hi_flags = 0,
        .base_mid2 = 0,
        .base_upper = 0,
    },
};

// ── Diagnostics ─────────────────────────────────────────────────

const SegmentSelectors = struct { cs: u16, ds: u16, es: u16, fs: u16, gs: u16, ss: u16 };

fn readSelectors() SegmentSelectors {
    var cs: u16 = 0; var ds: u16 = 0; var es: u16 = 0;
    var fs: u16 = 0; var gs: u16 = 0; var ss: u16 = 0;
    asm volatile ("mov %%cs, %[out]" : [out] "=r" (cs));
    asm volatile ("mov %%ds, %[out]" : [out] "=r" (ds));
    asm volatile ("mov %%es, %[out]" : [out] "=r" (es));
    asm volatile ("mov %%fs, %[out]" : [out] "=r" (fs));
    asm volatile ("mov %%gs, %[out]" : [out] "=r" (gs));
    asm volatile ("mov %%ss, %[out]" : [out] "=r" (ss));
    return .{ .cs = cs, .ds = ds, .es = es, .fs = fs, .gs = gs, .ss = ss };
}

pub fn logSelectors(tag: []const u8) void {
    const s = readSelectors();
    serial.puts(tag);
    serial.puts(" CS=0x");  serial.putHex(s.cs);
    serial.puts(" DS=0x");  serial.putHex(s.ds);
    serial.puts(" ES=0x");  serial.putHex(s.es);
    serial.puts(" FS=0x");  serial.putHex(s.fs);
    serial.puts(" GS=0x");  serial.putHex(s.gs);
    serial.puts(" SS=0x");  serial.putHex(s.ss);
    serial.puts("\n");
}

// ── Initialization ──────────────────────────────────────────────

pub fn init() void {
    // 1. Init TSS state (stacks + iopb)
    tss.init();

    // 2. Patch the TSS descriptor with the TSS's runtime address
    const tss_base: u64 = tss.address();
    const tss_limit: u32 = tss.size() - 1; // 0x67 (103)
    gdt.tss_desc.limit_low = @truncate(tss_limit & 0xFFFF);
    gdt.tss_desc.base_low = @truncate(tss_base & 0xFFFF);
    gdt.tss_desc.base_mid1 = @truncate((tss_base >> 16) & 0xFF);
    gdt.tss_desc.limit_hi_flags = @truncate(((tss_limit >> 16) & 0x0F) | 0x00); // G=0
    gdt.tss_desc.base_mid2 = @truncate((tss_base >> 24) & 0xFF);
    gdt.tss_desc.base_upper = @truncate((tss_base >> 32) & 0xFFFFFFFF);

    // 3. Build GDTR (10 bytes: limit + base)
    var gdtr: [10]u8 align(4) = undefined;
    const limit: u16 = @sizeOf(GdtArray) - 1;
    const base: u64 = @intFromPtr(&gdt);
    gdtr[0] = @truncate(limit);
    gdtr[1] = @truncate(limit >> 8);
    inline for (0..8) |i| {
        gdtr[2 + i] = @truncate(base >> (i * 8));
    }

    // 4. Load GDTR
    asm volatile ("lgdt %[gdtr]"
        :
        : [gdtr] "m" (gdtr),
        : .{ .memory = true }
    );

    // 5. Reload segment registers
    reloadSegments();

    // 6. Load TSS into the Task Register
    asm volatile ("ltr %[sel]"
        :
        : [sel] "r" (@as(u16, TSS_SEL)),
        : .{ .memory = true }
    );
}

/// Reload CS via far return and load all data segments with kernel data.
fn reloadSegments() void {
    asm volatile (
        \\push $0x08             // new CS = kernel code selector
        \\lea 1f(%%rip), %%rax
        \\push %%rax
        \\lretq
        \\1:
        \\mov $0x10, %%ax        // new DS/ES/FS/GS/SS = kernel data selector
        \\mov %%ax, %%ds
        \\mov %%ax, %%es
        \\mov %%ax, %%fs
        \\mov %%ax, %%gs
        \\mov %%ax, %%ss
        :
        :
        : .{ .rax = true, .memory = true }
    );
}
