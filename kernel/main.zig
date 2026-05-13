// ============================================================================
// VantaOS Kernel — Entry Point
// ============================================================================

const std = @import("std");
const limine = @import("limine.zig");
const serial = @import("arch/x86_64/serial.zig");
const gdt = @import("arch/x86_64/gdt.zig");
const idt = @import("arch/x86_64/idt.zig");
const pmm = @import("mm/pmm.zig");

// ── Limine Requests ─────────────────────────────────────────────
// These are placed in special linker sections so the bootloader can find them.

pub export var requests_start: [2]u64 linksection(".limine_requests_start") = limine.REQUESTS_START_MARKER;

pub export var base_revision: limine.BaseRevision linksection(".limine_requests") = .{};
pub export var framebuffer_req: limine.FramebufferRequest linksection(".limine_requests") = .{};
pub export var memmap_req: limine.MemoryMapRequest linksection(".limine_requests") = .{};
pub export var hhdm_req: limine.HhdmRequest linksection(".limine_requests") = .{};
pub export var kaddr_req: limine.KernelAddressRequest linksection(".limine_requests") = .{};

pub export var requests_end: [2]u64 linksection(".limine_requests_end") = limine.REQUESTS_END_MARKER;

// ── Entry Point ─────────────────────────────────────────────────
// Limine sets up long mode, paging, and a stack before jumping here.

export fn _start() callconv(.c) noreturn {
    kmain();
    halt();
}

// ── Kernel Main ─────────────────────────────────────────────────

fn kmain() void {
    // Stage 1: Serial output — our debug lifeline
    serial.init();
    serial.puts("\n");
    serial.puts("  ╔═══════════════════════════════════════╗\n");
    serial.puts("  ║   VantaOS v0.1.0 — First Light       ║\n");
    serial.puts("  ║   Capability-based microkernel        ║\n");
    serial.puts("  ╚═══════════════════════════════════════╝\n");
    serial.puts("\n");

    // Stage 2: Verify bootloader protocol
    if (!base_revision.isSupported()) {
        serial.puts("[FATAL] Limine base revision not supported!\n");
        serial.puts("        Need Limine bootloader v7+ with protocol revision 3\n");
        halt();
    }
    serial.puts("[BOOT]  Limine protocol v3 — OK\n");

    // Stage 3: Load our own GDT (Limine's is temporary)
    gdt.init();
    serial.puts("[GDT]   Global Descriptor Table loaded\n");

    // Stage 4: Load IDT (exception handlers)
    idt.init();

    // Stage 5: Initialize physical memory
    if (memmap_req.response) |memmap_resp| {
        pmm.init(memmap_resp);
        serial.puts("[PMM]   Physical memory manager initialized\n");

        // Print memory stats
        const stats = pmm.getStats();
        serial.puts("        Total: ");
        serial.putDec(stats.total_pages * 4);
        serial.puts(" KB (");
        serial.putDec(stats.total_pages * 4 / 1024);
        serial.puts(" MB)\n");
        serial.puts("        Free:  ");
        serial.putDec(stats.free_pages * 4);
        serial.puts(" KB (");
        serial.putDec(stats.free_pages * 4 / 1024);
        serial.puts(" MB)\n");

        // Test allocation
        if (pmm.allocPage()) |page| {
            serial.puts("        Alloc test: page at 0x");
            serial.putHex(page);
            serial.puts(" — OK\n");
            pmm.freePage(page);
        }
    } else {
        serial.puts("[FATAL] No memory map from bootloader!\n");
        halt();
    }

    // Stage 6: HHDM info
    if (hhdm_req.response) |hhdm| {
        serial.puts("[HHDM]  Higher-half direct map at 0x");
        serial.putHex(hhdm.offset);
        serial.puts("\n");
    }

    // Stage 7: Kernel address info
    if (kaddr_req.response) |kaddr| {
        serial.puts("[KERN]  Physical base: 0x");
        serial.putHex(kaddr.physical_base);
        serial.puts("\n");
        serial.puts("[KERN]  Virtual base:  0x");
        serial.putHex(kaddr.virtual_base);
        serial.puts("\n");
    }

    // Stage 8: Framebuffer
    if (framebuffer_req.response) |fb_resp| {
        if (fb_resp.framebuffer_count > 0) {
            const fb = fb_resp.framebuffers[0];
            serial.puts("[FB]    ");
            serial.putDec(fb.width);
            serial.puts("x");
            serial.putDec(fb.height);
            serial.puts("x");
            serial.putDec(fb.bpp);
            serial.puts("bpp\n");

            // Draw the Vanta Black boot screen
            drawBootScreen(fb);
        }
    }

    // ── Boot complete ────────────────────────────────────────────
    serial.puts("\n");
    serial.puts("══════════════════════════════════════════════\n");
    serial.puts("  VantaOS kernel initialized successfully.\n");
    serial.puts("  Next: VMM → Scheduler → First userspace\n");
    serial.puts("══════════════════════════════════════════════\n");
    serial.puts("\n");
}

// ── Boot Screen ─────────────────────────────────────────────────
// Draw a deep dark gradient — Vanta Black aesthetic.

fn drawBootScreen(fb: *volatile limine.Framebuffer) void {
    const width: usize = @intCast(fb.width);
    const height: usize = @intCast(fb.height);
    const pitch_bytes: usize = @intCast(fb.pitch);
    const addr = fb.address;

    for (0..height) |y| {
        const row: [*]volatile u32 = @ptrCast(@alignCast(addr + y * pitch_bytes));
        for (0..width) |x| {
            // Deep dark gradient: almost black with subtle variation
            const gy: u32 = @intCast(@min(y * 6 / height, 5));
            const gx: u32 = @intCast(@min(x * 3 / width, 2));
            const shade: u32 = gy + gx;
            row[x] = shade | (shade << 8) | (shade << 16);
        }
    }

    // Draw a small white rectangle as a "cursor" / proof of life
    const cx: usize = width / 2 - 20;
    const cy: usize = height / 2 - 2;
    for (cy..cy + 4) |y| {
        const row: [*]volatile u32 = @ptrCast(@alignCast(addr + y * pitch_bytes));
        for (cx..cx + 40) |x| {
            row[x] = 0x00CCCCCC; // Light gray
        }
    }

    serial.puts("[FB]    Boot screen drawn (vanta black gradient)\n");
}

// ── Halt ─────────────────────────────────────────────────────────

fn halt() noreturn {
    serial.puts("[HALT]  System halted.\n");
    asm volatile ("cli");
    while (true) {
        asm volatile ("hlt");
    }
}

// ── Panic Handler ───────────────────────────────────────────────
// Required for freestanding Zig. Called on @panic(), assert failures, etc.

pub fn panic(msg: []const u8, _: ?*std.builtin.StackTrace, _: ?usize) noreturn {
    serial.puts("\n");
    serial.puts("!!! KERNEL PANIC !!!\n");
    serial.puts(msg);
    serial.puts("\n");
    halt();
}
