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

/// Route specific hardware IRQ to a target IDT Vector
pub fn routeIrq(irq: u8, vector: u8, apic_id: u8) void {
    const reg_low = 0x10 + @as(u32, irq) * 2;
    const reg_high = reg_low + 1;

    // Route to BSP: high 32 bits holds destination APIC ID in bits 24-31
    ioapicWrite(reg_high, @as(u32, apic_id) << 24);

    // Low 32 bits: vector, fixed delivery, edge-triggered, active-high, unmasked
    ioapicWrite(reg_low, @as(u32, vector));
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

/// Initialize the PS/2 auxiliary (mouse) port and enable IRQ 12.
/// Call before scheduling userspace so the input_server can bind IRQ 12.
pub fn initPs2Mouse() void {
    // Flush any stale data in the PS/2 data port.
    _ = cpu.inb(0x60);

    // Enable the auxiliary device (PS/2 mouse port).
    cpu.outb(0x64, 0xA8);

    // Read current controller config byte, OR in bit 1 (enable IRQ 12).
    cpu.outb(0x64, 0x20);
    var cfg = cpu.inb(0x60);
    cfg |= 0x02;
    cpu.outb(0x64, 0x60);
    cpu.outb(0x60, cfg);

    // Send "Enable Data Reporting" command to the mouse.
    cpu.outb(0x64, 0xD4);
    cpu.outb(0x60, 0xF4);
    // Discard the ACK byte.
    _ = cpu.inb(0x60);

    serial.puts("[PS2]   Mouse auxiliary port enabled (IRQ 12)\n");
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
            // I/O APIC Record
            const ioapic_addr = @as(*align(1) const u32, @ptrFromInt(table_virt + offset + 4)).*;
            ioapic_phys = ioapic_addr;
            serial.puts("[ACPI]  MADT reports I/O APIC at 0x");
            serial.putHex(ioapic_phys);
            serial.puts("\n");
        }

        offset += entry_len;
    }

    return .{
        .lapic_phys = lapic_phys,
        .ioapic_phys = ioapic_phys,
    };
}
