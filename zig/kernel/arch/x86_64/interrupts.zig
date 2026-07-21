// ============================================================================
// VantaOS — Interrupt & APIC Controller Setup (x86_64)
// Phase 1: LAPIC mapping, PIT-calibrated Timer, MADT parse, I/O APIC route.
// ============================================================================

const std = @import("std");
const limine = @import("../../limine.zig");
const serial = @import("serial.zig");
const cpu = @import("cpu.zig");
const vmm = @import("../../mm/vmm.zig");
const sched = @import("../../sched/scheduler.zig");

// LAPIC Register Offsets
const LAPIC_ID: u32 = 0x20;
const LAPIC_VER: u32 = 0x30;
const LAPIC_TPR: u32 = 0x80;
const LAPIC_EOI: u32 = 0xB0;
const LAPIC_LDR: u32 = 0xD0;
const LAPIC_SVR: u32 = 0xF0;
const LAPIC_ESR: u32 = 0x280;
const LAPIC_LVT_TMR: u32 = 0x320;
const LAPIC_LVT_LINT0: u32 = 0x350;
const LAPIC_LVT_LINT1: u32 = 0x360;
const LAPIC_LVT_ERR: u32 = 0x370;
const LAPIC_TMRINIT: u32 = 0x380;
const LAPIC_TMRCURR: u32 = 0x390;
const LAPIC_TMRDIV: u32 = 0x3E0;

// Globals
pub var lapic_virt: u64 = 0;
pub var ioapic_virt: u64 = 0;
pub var lapic_ticks_in_10ms: u32 = 0;

// I/O APIC global system interrupt base (from the MADT I/O APIC record).
pub var ioapic_gsi_base: u32 = 0;

// MADT Interrupt Source Overrides. ISA IRQs are NOT always identity-mapped to
// I/O APIC pins — firmware (esp. VirtualBox) may remap them and specify
// non-default polarity/trigger. Real OSes honor these; ignoring them means the
// IRQ is programmed on the wrong pin/mode and never fires.
const MAX_ISO: usize = 24;
const IsoEntry = struct {
    source: u8 = 0,
    gsi: u32 = 0,
    flags: u16 = 0,
    active: bool = false,
};
pub var iso_table: [MAX_ISO]IsoEntry = [_]IsoEntry{.{}} ** MAX_ISO;

pub fn init(rsdp_response: ?*volatile limine.RsdpResponse) void {
    // 1. Mask the legacy 8259 PIC
    serial.puts("[APIC]  Masking legacy 8259 PIC...\n");
    cpu.outb(0x21, 0xFF);
    cpu.outb(0xA1, 0xFF);

    // 2. Discover LAPIC & I/O APIC physical addresses via ACPI MADT
    var lapic_phys: u64 = 0xFEE00000;
    var ioapic_phys: u64 = 0xFEC00000;

    if (rsdp_response) |resp| {
        // Limine gives a virtual (HHDM) address; convert to physical for parseMadt
        const rsdp_phys = vmm.virt2phys_hhdm(resp.address);
        serial.puts("[ACPI]  RSDP at virt=0x");
        serial.putHex(resp.address);
        serial.puts(" phys=0x");
        serial.putHex(rsdp_phys);
        serial.puts("\n");
        if (parseMadt(rsdp_phys)) |madt_info| {
            lapic_phys = madt_info.lapic_phys;
            ioapic_phys = madt_info.ioapic_phys;
        }
    } else {
        serial.puts("[ACPI]  No RSDP response. Using default fallback addresses.\n");
    }

    serial.puts("[APIC]  LAPIC Phys=0x");
    serial.putHex(lapic_phys);
    serial.puts("  IOAPIC Phys=0x");
    serial.putHex(ioapic_phys);
    serial.puts("\n");

    // 3. Map LAPIC Registers
    lapic_virt = 0xFFFF880000000000;
    if (!vmm.map(vmm.AddressSpace.current(), lapic_virt, lapic_phys, vmm.PTE_WRITE | vmm.PTE_CD | vmm.PTE_WT)) {
        serial.puts("[FATAL] Failed to map LAPIC registers\n");
        while (true) asm volatile ("cli; hlt");
    }

    // 4. Map I/O APIC Registers
    ioapic_virt = 0xFFFF880000001000;
    if (!vmm.map(vmm.AddressSpace.current(), ioapic_virt, ioapic_phys, vmm.PTE_WRITE | vmm.PTE_CD | vmm.PTE_WT)) {
        serial.puts("[FATAL] Failed to map I/O APIC registers\n");
        while (true) asm volatile ("cli; hlt");
    }

    // 5. Enable Local APIC
    // Set Spurious Interrupt Vector Register to 0x1FF (Vector 255 + APIC Software Enable bit 8)
    lapicWrite(LAPIC_SVR, lapicRead(LAPIC_SVR) | 0x100 | 0xFF);

    serial.puts("[APIC]  Local APIC enabled successfully\n");

    // 6. Calibrate LAPIC Timer using PIT
    calibrateTimer();
}

// ── LAPIC MMIO Helpers ──────────────────────────────────────────

pub inline fn lapicWrite(reg: u32, val: u32) void {
    const ptr = @as(*volatile u32, @ptrFromInt(lapic_virt + reg));
    ptr.* = val;
}

pub inline fn lapicRead(reg: u32) u32 {
    const ptr = @as(*const volatile u32, @ptrFromInt(lapic_virt + reg));
    return ptr.*;
}

pub fn eoi() void {
    lapicWrite(LAPIC_EOI, 0);
}

// ── I/O APIC MMIO Helpers ───────────────────────────────────────

pub fn ioapicWrite(reg: u32, val: u32) void {
    const regsel = @as(*volatile u32, @ptrFromInt(ioapic_virt + 0x00));
    const iowin = @as(*volatile u32, @ptrFromInt(ioapic_virt + 0x10));
    regsel.* = reg;
    iowin.* = val;
}

pub fn ioapicRead(reg: u32) u32 {
    const regsel = @as(*volatile u32, @ptrFromInt(ioapic_virt + 0x00));
    const iowin = @as(*volatile u32, @ptrFromInt(ioapic_virt + 0x10));
    regsel.* = reg;
    return iowin.*;
}

/// Route a legacy ISA IRQ to an IDT vector via the I/O APIC, honoring any MADT
/// Interrupt Source Override (remapped GSI + polarity/trigger flags). This is
/// what real OSes do; hardcoding pin=IRQ, edge, active-high is only correct
/// when there is no override.
pub fn routeIrq(irq: u8, vector: u8, apic_id: u8) void {
    // Resolve the GSI and flags for this ISA IRQ.
    var gsi: u32 = irq;
    var flags: u16 = 0;
    for (iso_table) |e| {
        if (e.active and e.source == irq) {
            gsi = e.gsi;
            flags = e.flags;
            break;
        }
    }

    const pin = gsi - ioapic_gsi_base;
    const reg_low = 0x10 + pin * 2;
    const reg_high = reg_low + 1;

    // MPS INTI flags: bits[1:0] polarity (0/1 = active high, 3 = active low),
    // bits[3:2] trigger (0/1 = edge, 3 = level).
    var redir: u32 = vector; // fixed delivery, physical dest, unmasked
    if ((flags & 0x3) == 0x3) redir |= (1 << 13); // active low
    if (((flags >> 2) & 0x3) == 0x3) redir |= (1 << 15); // level triggered

    ioapicWrite(reg_high, @as(u32, apic_id) << 24);
    ioapicWrite(reg_low, redir);

    serial.puts("[APIC]  routed IRQ ");
    serial.putDec(irq);
    serial.puts(" -> GSI ");
    serial.putDec(gsi);
    serial.puts(" pin ");
    serial.putDec(pin);
    serial.puts(" vec ");
    serial.putDec(vector);
    serial.puts("\n");
}

// ── Timer Calibration ───────────────────────────────────────────

fn calibrateTimer() void {
    serial.puts("[PIT]   Calibrating LAPIC timer (10ms target)...\n");

    // Configure PIT Channel 0 to Mode 0 (Interrupt on Terminal Count / One-Shot)
    // Access Mode: lobyte/hibyte
    cpu.outb(0x43, 0x30);

    // Set PIT count to 11931 (which equals 10 milliseconds at 1.193182 MHz)
    cpu.outb(0x40, 0xAB); // Low byte
    cpu.outb(0x40, 0x2E); // High byte

    // Configure LAPIC timer: divide by 16
    lapicWrite(LAPIC_TMRDIV, 0x03);

    // Start LAPIC timer with initial count of 0xFFFFFFFF
    lapicWrite(LAPIC_TMRINIT, 0xFFFFFFFF);

    // Wait for PIT countdown to complete
    var last_val: u16 = 11931;
    while (true) {
        // Latch count
        cpu.outb(0x43, 0x00);
        const low = cpu.inb(0x40);
        const high = cpu.inb(0x40);
        const val = (@as(u16, high) << 8) | low;
        // Break if counter wrapped around or finished
        if (val > last_val or val == 0) break;
        last_val = val;
    }

    const current_lapic = lapicRead(LAPIC_TMRCURR);
    const ticks_in_10ms = 0xFFFFFFFF - current_lapic;

    serial.puts("[PIT]   Calibrated: ");
    serial.putDec(ticks_in_10ms);
    serial.puts(" LAPIC ticks per 10ms\n");

    lapic_ticks_in_10ms = ticks_in_10ms;

    // Configure LAPIC Timer for periodic interrupts on Vector 32 (Timer IRQ)
    // Periodic mode is bit 17
    lapicWrite(LAPIC_LVT_TMR, 0x20000 | 32);
    // Divide by 16
    lapicWrite(LAPIC_TMRDIV, 0x03);
    // Initial count to the calibrated value
    lapicWrite(LAPIC_TMRINIT, ticks_in_10ms);

    serial.puts("[SCHED] Periodic preemption timer enabled (100Hz)\n");
}

// ── PS/2 controller (i8042) helpers ─────────────────────────────
// Status register (port 0x64) bit 0 = output buffer full (data ready to read),
// bit 1 = input buffer full (must be 0 before writing a command/data).

/// Wait until the i8042 input buffer is empty so a write won't be dropped.
fn ps2WaitWrite() void {
    var spin: u32 = 0;
    while (spin < 100000) : (spin += 1) {
        if (cpu.inb(0x64) & 0x02 == 0) return;
    }
}

/// Wait until the i8042 output buffer has a byte to read. Returns false on timeout.
fn ps2WaitRead() bool {
    var spin: u32 = 0;
    while (spin < 100000) : (spin += 1) {
        if (cpu.inb(0x64) & 0x01 != 0) return true;
    }
    return false;
}

/// Drain every pending byte from the output buffer (so OBF=0). While the
/// output buffer is full the controller cannot deliver new bytes or raise
/// IRQs, so a stale byte here silently disables the keyboard.
fn ps2Flush() void {
    var spin: u32 = 0;
    while (cpu.inb(0x64) & 0x01 != 0 and spin < 1024) : (spin += 1) {
        _ = cpu.inb(0x60);
    }
}

/// Full PS/2 keyboard initialization (pluto/zigux/OSDev style).
/// Steps: disable ports → flush → configure → self-test → re-enable →
/// keyboard reset → enable IRQ1+translation → enable scanning.
/// Skipping any of these steps leaves the controller in an undefined state
/// (especially after UEFI hand-off) and IRQ 1 will never fire.
pub fn initPs2Keyboard() void {
    // 1. Disable both PS/2 ports so nothing fires during setup.
    ps2WaitWrite();
    cpu.outb(0x64, 0xAD); // disable port 1 (keyboard)
    ps2WaitWrite();
    cpu.outb(0x64, 0xA7); // disable port 2 (mouse, no-op if single-channel)

    // 2. Drain any stale byte left in the output buffer.
    ps2Flush();

    // 3. Read config, clear IRQ bits and translation so self-test runs clean.
    ps2WaitWrite();
    cpu.outb(0x64, 0x20);
    var cfg: u8 = 0;
    if (ps2WaitRead()) cfg = cpu.inb(0x60);
    cfg &= ~@as(u8, 0x43); // clear bits 0 (IRQ1), 1 (IRQ12), 6 (translation)
    cfg &= ~@as(u8, 0x10); // clear bit 4 (keyboard clock disable)
    ps2WaitWrite();
    cpu.outb(0x64, 0x60);
    ps2WaitWrite();
    cpu.outb(0x60, cfg);

    // 4. Controller self-test — sends 0xAA, expects 0x55.
    //    On some hardware the controller resets its config after this,
    //    so we re-apply settings in step 7.
    ps2WaitWrite();
    cpu.outb(0x64, 0xAA);
    if (ps2WaitRead()) {
        const r = cpu.inb(0x60);
        serial.puts("[PS2]   Controller self-test: 0x");
        serial.putHex(r);
        serial.puts(if (r == 0x55) " OK\n" else " FAIL (expected 0x55)\n");
    }

    // 5. Re-enable port 1 (keyboard).
    ps2WaitWrite();
    cpu.outb(0x64, 0xAE);
    ps2Flush();

    // 6. Full keyboard reset (0xFF → expect 0xFA ACK, then 0xAA self-test pass).
    //    This clears any leftover state from BIOS/UEFI.
    ps2WaitWrite();
    cpu.outb(0x60, 0xFF);
    if (ps2WaitRead()) {
        const ack = cpu.inb(0x60);
        serial.puts("[PS2]   KBD reset ACK: 0x");
        serial.putHex(ack);
        serial.puts("\n");
    }
    if (ps2WaitRead()) {
        const st = cpu.inb(0x60);
        serial.puts("[PS2]   KBD self-test: 0x");
        serial.putHex(st);
        serial.puts(if (st == 0xAA) " OK\n" else " FAIL\n");
    }
    ps2Flush();

    // 7. Re-read config, then set:
    //    bit 0 = IRQ1 enabled
    //    bit 6 = scancode translation (set2 → set1) — critical for our scancode map
    //    bit 4 cleared = keyboard clock enabled
    ps2WaitWrite();
    cpu.outb(0x64, 0x20);
    if (ps2WaitRead()) cfg = cpu.inb(0x60);
    cfg |= 0x41;            // set IRQ1 + translation
    cfg &= ~@as(u8, 0x10);  // ensure keyboard is enabled
    ps2WaitWrite();
    cpu.outb(0x64, 0x60);
    ps2WaitWrite();
    cpu.outb(0x60, cfg);
    serial.puts("[PS2]   Config byte written: 0x");
    serial.putHex(cfg);
    serial.puts("\n");

    // 8. Tell keyboard to start scanning, consume its ACK.
    ps2WaitWrite();
    cpu.outb(0x60, 0xF4);
    if (ps2WaitRead()) _ = cpu.inb(0x60);

    ps2Flush();
    serial.puts("[PS2]   Keyboard initialized (IRQ 1 + translation enabled)\n");
}

/// Initialize the PS/2 mouse port.
/// Keeps port 2 disabled (no mouse server in current build) — just flushes
/// any pending mouse bytes so they don't block keyboard scancode delivery.
pub fn initPs2Mouse() void {
    // Drain anything the mouse left in the buffer.
    ps2Flush();
    serial.puts("[PS2]   Mouse port flushed (mouse IRQ disabled)\n");
}

pub fn setupApTimer() void {
    // Configure LAPIC Timer for periodic interrupts on Vector 32 (Timer IRQ)
    lapicWrite(LAPIC_LVT_TMR, 0x20000 | 32);
    // Divide by 16
    lapicWrite(LAPIC_TMRDIV, 0x03);
    // Initial count to the calibrated value
    lapicWrite(LAPIC_TMRINIT, lapic_ticks_in_10ms);
}

// ── ACPI / MADT Parsing ──────────────────────────────────────────

const MadtInfo = struct {
    lapic_phys: u64,
    ioapic_phys: u64,
};

fn parseMadt(rsdp_phys: u64) ?MadtInfo {
    const rsdp_virt = vmm.phys2virt(rsdp_phys);
    const sig = @as([*]const u8, @ptrFromInt(rsdp_virt));

    if (!std.mem.eql(u8, sig[0..8], "RSD PTR ")) {
        serial.puts("[ACPI]  Invalid RSDP signature\n");
        return null;
    }

    const revision = sig[15];
    var xsdt_phys: u64 = 0;
    var rsdt_phys: u64 = 0;

    if (revision >= 2) {
        // ACPI 2.0+
        const xsdt_ptr = @as(*align(1) const u64, @ptrFromInt(rsdp_virt + 24));
        xsdt_phys = xsdt_ptr.*;
    } else {
        // ACPI 1.0
        const rsdt_ptr = @as(*align(1) const u32, @ptrFromInt(rsdp_virt + 16));
        rsdt_phys = rsdt_ptr.*;
    }

    var table_phys: u64 = 0;
    var is_xsdt = false;

    if (xsdt_phys != 0) {
        table_phys = xsdt_phys;
        is_xsdt = true;
    } else if (rsdt_phys != 0) {
        table_phys = rsdt_phys;
        is_xsdt = false;
    } else {
        return null;
    }

    const table_virt = vmm.phys2virt(table_phys);
    const header_sig = @as([*]const u8, @ptrFromInt(table_virt));
    const header_len = @as(*align(1) const u32, @ptrFromInt(table_virt + 4)).*;

    if (is_xsdt) {
        if (!std.mem.eql(u8, header_sig[0..4], "XSDT")) return null;
        const entry_count = (header_len - 36) / 8;
        const entries = @as([*]align(1) const u64, @ptrFromInt(table_virt + 36));
        var i: usize = 0;
        while (i < entry_count) : (i += 1) {
            if (checkMadt(entries[i])) |info| return info;
        }
    } else {
        if (!std.mem.eql(u8, header_sig[0..4], "RSDT")) return null;
        const entry_count = (header_len - 36) / 4;
        const entries = @as([*]align(1) const u32, @ptrFromInt(table_virt + 36));
        var i: usize = 0;
        while (i < entry_count) : (i += 1) {
            if (checkMadt(entries[i])) |info| return info;
        }
    }

    return null;
}

fn checkMadt(table_phys: u64) ?MadtInfo {
    const table_virt = vmm.phys2virt(table_phys);
    const sig = @as([*]const u8, @ptrFromInt(table_virt));

    if (!std.mem.eql(u8, sig[0..4], "APIC")) {
        return null;
    }

    serial.puts("[ACPI]  MADT found!\n");
    const len = @as(*align(1) const u32, @ptrFromInt(table_virt + 4)).*;

    const lapic_phys: u64 = @as(*align(1) const u32, @ptrFromInt(table_virt + 36)).*;
    var ioapic_phys: u64 = 0xFEC00000;

    var offset: usize = 44;
    while (offset < len) {
        const entry_type = @as(*const u8, @ptrFromInt(table_virt + offset)).*;
        const entry_len = @as(*const u8, @ptrFromInt(table_virt + offset + 1)).*;
        if (entry_len == 0) break;

        if (entry_type == 1) {
            // I/O APIC Record: addr at +4, GSI base at +8.
            const ioapic_addr = @as(*align(1) const u32, @ptrFromInt(table_virt + offset + 4)).*;
            ioapic_phys = ioapic_addr;
            ioapic_gsi_base = @as(*align(1) const u32, @ptrFromInt(table_virt + offset + 8)).*;
            serial.puts("[ACPI]  I/O APIC at 0x");
            serial.putHex(ioapic_phys);
            serial.puts(" gsi_base=");
            serial.putDec(ioapic_gsi_base);
            serial.puts("\n");
        } else if (entry_type == 2) {
            // Interrupt Source Override: bus(+2), source IRQ(+3), GSI(+4), flags(+8).
            const source = @as(*const u8, @ptrFromInt(table_virt + offset + 3)).*;
            const gsi = @as(*align(1) const u32, @ptrFromInt(table_virt + offset + 4)).*;
            const flags = @as(*align(1) const u16, @ptrFromInt(table_virt + offset + 8)).*;
            for (&iso_table) |*e| {
                if (!e.active) {
                    e.* = .{ .source = source, .gsi = gsi, .flags = flags, .active = true };
                    break;
                }
            }
            serial.puts("[ACPI]  ISO: IRQ ");
            serial.putDec(source);
            serial.puts(" -> GSI ");
            serial.putDec(gsi);
            serial.puts(" flags=0x");
            serial.putHex(flags);
            serial.puts("\n");
        }

        offset += entry_len;
    }

    return .{
        .lapic_phys = lapic_phys,
        .ioapic_phys = ioapic_phys,
    };
}
