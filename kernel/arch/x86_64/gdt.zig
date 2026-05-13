// ============================================================================
// VantaOS — Global Descriptor Table (GDT)
// Sets up segment descriptors for x86_64 long mode.
// Limine provides a temporary GDT; we install our own.
// ============================================================================

const serial = @import("serial.zig");

// ── GDT Entry (8 bytes) ────────────────────────────────────────

const GdtEntry = packed struct(u64) {
    limit_low: u16,
    base_low: u16,
    base_mid: u8,
    access: u8,
    limit_high_flags: u8, // bits 0-3: limit[19:16], bits 4-7: flags
    base_high: u8,
};

fn makeEntry(base: u32, limit: u32, access: u8, flags: u8) GdtEntry {
    return .{
        .limit_low = @truncate(limit & 0xFFFF),
        .base_low = @truncate(base & 0xFFFF),
        .base_mid = @truncate((base >> 16) & 0xFF),
        .access = access,
        .limit_high_flags = @truncate(
            ((limit >> 16) & 0x0F) | ((flags << 4) & 0xF0),
        ),
        .base_high = @truncate((base >> 24) & 0xFF),
    };
}

// ── Null entry macro ────────────────────────────────────────────

fn nullEntry() GdtEntry {
    return .{
        .limit_low = 0,
        .base_low = 0,
        .base_mid = 0,
        .access = 0,
        .limit_high_flags = 0,
        .base_high = 0,
    };
}

// ── GDT Table ───────────────────────────────────────────────────
//
// Selector layout:
//   0x00 — Null descriptor
//   0x08 — Kernel code (ring 0, 64-bit, execute/read)
//   0x10 — Kernel data (ring 0, read/write)
//   0x18 — User code   (ring 3, 64-bit, execute/read)
//   0x20 — User data   (ring 3, read/write)
//
// In long mode, the base and limit fields of code/data segments are
// ignored by hardware (except for FS/GS). The access byte and the
// L (long mode) flag are what matter.

var gdt_entries = [_]GdtEntry{
    // 0x00: Null
    nullEntry(),

    // 0x08: Kernel Code — Present, Ring 0, Code, Readable, Long Mode
    //   Access: P=1 DPL=00 S=1 E=1 DC=0 RW=1 A=0 = 0b10011010 = 0x9A
    //   Flags: G=1 L=1 D=0 = 0b1010 = 0xA
    makeEntry(0, 0xFFFFF, 0x9A, 0xA),

    // 0x10: Kernel Data — Present, Ring 0, Data, Writable
    //   Access: P=1 DPL=00 S=1 E=0 DC=0 RW=1 A=0 = 0b10010010 = 0x92
    //   Flags: G=1 D/B=1 L=0 = 0b1100 = 0xC
    makeEntry(0, 0xFFFFF, 0x92, 0xC),

    // 0x18: User Code — Present, Ring 3, Code, Readable, Long Mode
    //   Access: P=1 DPL=11 S=1 E=1 DC=0 RW=1 A=0 = 0b11111010 = 0xFA
    //   Flags: G=1 L=1 D=0 = 0b1010 = 0xA
    makeEntry(0, 0xFFFFF, 0xFA, 0xA),

    // 0x20: User Data — Present, Ring 3, Data, Writable
    //   Access: P=1 DPL=11 S=1 E=0 DC=0 RW=1 A=0 = 0b11110010 = 0xF2
    //   Flags: G=1 D/B=1 L=0 = 0b1100 = 0xC
    makeEntry(0, 0xFFFFF, 0xF2, 0xC),
};

pub const KERNEL_CODE_SEL: u16 = 0x08;
pub const KERNEL_DATA_SEL: u16 = 0x10;
pub const USER_CODE_SEL: u16 = 0x18;
pub const USER_DATA_SEL: u16 = 0x20;

const SegmentSelectors = struct {
    cs: u16,
    ds: u16,
    es: u16,
    fs: u16,
    gs: u16,
    ss: u16,
};

fn readSelectors() SegmentSelectors {
    var selectors: SegmentSelectors = undefined;
    var cs: u16 = 0;
    var ds: u16 = 0;
    var es: u16 = 0;
    var fs: u16 = 0;
    var gs: u16 = 0;
    var ss: u16 = 0;
    asm volatile ("mov %%cs, %[out]" : [out] "=r" (cs));
    asm volatile ("mov %%ds, %[out]" : [out] "=r" (ds));
    asm volatile ("mov %%es, %[out]" : [out] "=r" (es));
    asm volatile ("mov %%fs, %[out]" : [out] "=r" (fs));
    asm volatile ("mov %%gs, %[out]" : [out] "=r" (gs));
    asm volatile ("mov %%ss, %[out]" : [out] "=r" (ss));
    selectors.cs = cs;
    selectors.ds = ds;
    selectors.es = es;
    selectors.fs = fs;
    selectors.gs = gs;
    selectors.ss = ss;
    return selectors;
}

pub fn logSelectors(tag: []const u8) void {
    const selectors = readSelectors();
    serial.puts(tag);
    serial.puts(" CS=0x");
    serial.putHex(selectors.cs);
    serial.puts(" DS=0x");
    serial.putHex(selectors.ds);
    serial.puts(" ES=0x");
    serial.putHex(selectors.es);
    serial.puts(" FS=0x");
    serial.putHex(selectors.fs);
    serial.puts(" GS=0x");
    serial.putHex(selectors.gs);
    serial.puts(" SS=0x");
    serial.putHex(selectors.ss);
    serial.puts("\n");
}

// ── GDTR Loading ────────────────────────────────────────────────
// The GDTR is a 10-byte structure: 2-byte limit + 8-byte base.
// We build it in a byte array to avoid packed struct alignment issues.

pub fn init() void {
    // Build GDTR in a local byte array (10 bytes: limit[2] + base[8])
    var gdtr: [10]u8 align(4) = undefined;
    const limit: u16 = @sizeOf(@TypeOf(gdt_entries)) - 1;
    const base: u64 = @intFromPtr(&gdt_entries);

    // Write limit (little-endian)
    gdtr[0] = @truncate(limit);
    gdtr[1] = @truncate(limit >> 8);

    // Write base (little-endian)
    inline for (0..8) |i| {
        gdtr[2 + i] = @truncate(base >> (i * 8));
    }

    // Load GDT register
    asm volatile ("lgdt %[gdtr]"
        :
        : [gdtr] "m" (gdtr),
        : .{ .memory = true }
    );

    // Reload segment registers with our new selectors
    reloadSegments();
}

/// Reload CS (via far return) and data segment registers.
fn reloadSegments() void {
    // Reload CS by pushing new CS + return address, then lretq
    asm volatile (
        \\push $0x08
        \\lea 1f(%%rip), %%rax
        \\push %%rax
        \\lretq
        \\1:
        \\mov $0x10, %%ax
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
