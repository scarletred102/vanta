// ============================================================================
// VantaOS Kernel — Entry Point (Phase 1)
// ============================================================================

const std = @import("std");
const limine = @import("limine.zig");
const cpu = @import("arch/x86_64/cpu.zig");
const serial = @import("arch/x86_64/serial.zig");
const gdt = @import("arch/x86_64/gdt.zig");
const idt = @import("arch/x86_64/idt.zig");
const syscall = @import("arch/x86_64/syscall.zig");
const ctx = @import("arch/x86_64/context.zig");
const pmm = @import("mm/pmm.zig");
const vmm = @import("mm/vmm.zig");
const sched = @import("sched/scheduler.zig");
const thread = @import("sched/thread.zig");
const proc = @import("proc/process.zig");

// ── Limine Requests ─────────────────────────────────────────────

pub export var base_revision: limine.BaseRevision linksection(".limine_requests_start") = .{};
pub export var framebuffer_req: limine.FramebufferRequest linksection(".limine_requests") = .{};
pub export var memmap_req: limine.MemoryMapRequest linksection(".limine_requests") = .{};
pub export var hhdm_req: limine.HhdmRequest linksection(".limine_requests") = .{};
pub export var kaddr_req: limine.KernelAddressRequest linksection(".limine_requests") = .{};
pub export var requests_end: [2]u64 linksection(".limine_requests_end") = limine.REQUESTS_END_MARKER;

// ── Entry ───────────────────────────────────────────────────────

export fn _start() callconv(.c) noreturn {
    asm volatile ("cli");
    cpu.enableSse();
    kmain();
    halt();
}

// ── Test kernel threads ─────────────────────────────────────────

var counter_a: u64 = 0;
var counter_b: u64 = 0;

fn threadA() callconv(.c) noreturn {
    while (true) {
        counter_a += 1;
        if (counter_a % 1_000_000 == 0) {
            serial.puts("[T-A]   tick ");
            serial.putDec(counter_a / 1_000_000);
            serial.puts("\n");
            sched.yield();
        }
    }
}

fn threadB() callconv(.c) noreturn {
    while (true) {
        counter_b += 1;
        if (counter_b % 1_000_000 == 0) {
            serial.puts("[T-B]   tick ");
            serial.putDec(counter_b / 1_000_000);
            serial.puts("\n");
            sched.yield();
        }
    }
}

// ── kmain ───────────────────────────────────────────────────────

fn kmain() void {
    earlyFbMark(0x00220022);

    serial.init();
    serial.puts("\n[BOOT]  VantaOS Phase 1 starting\n");

    if (!base_revision.isSupported()) {
        serial.puts("[WARN]  base revision not confirmed\n");
    } else {
        serial.puts("[BOOT]  Limine protocol — OK\n");
    }

    // GDT + TSS
    gdt.init();
    serial.puts("[GDT]   own GDT + TSS loaded\n");

    // IDT
    idt.init();

    earlyFbMark(0x00000044);

    // PMM
    if (memmap_req.response) |mm| {
        pmm.init(mm);
        const s = pmm.getStats();
        serial.puts("[PMM]   total=");
        serial.putDec(s.total_pages * 4 / 1024);
        serial.puts("MB free=");
        serial.putDec(s.free_pages * 4 / 1024);
        serial.puts("MB\n");
    } else {
        serial.puts("[FATAL] no memory map\n");
        halt();
    }

    // VMM
    if (hhdm_req.response) |h| {
        vmm.init(h);
    } else {
        serial.puts("[FATAL] no HHDM\n");
        halt();
    }

    // Kernel process
    proc.initKernelProc();
    serial.puts("[PROC]  kernel proc (pid 0) initialized\n");

    // SYSCALL MSRs
    syscall.init();

    earlyFbMark(0x00440000);

    // Framebuffer info
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
            drawBootScreen(fb);
        }
    }

    // Scheduler
    sched.init();

    // Spawn two test kernel threads
    const ta = thread.create(threadA) orelse {
        serial.puts("[FATAL] thread A create failed\n");
        halt();
    };
    const tb = thread.create(threadB) orelse {
        serial.puts("[FATAL] thread B create failed\n");
        halt();
    };
    sched.enqueue(ta);
    sched.enqueue(tb);
    serial.puts("[SCHED] 2 test threads queued\n");

    serial.puts("══════════════════════════════════════════════\n");
    serial.puts("  VantaOS Phase 1 ready. Starting scheduler.\n");
    serial.puts("══════════════════════════════════════════════\n");

    // Hand off to scheduler — does not return
    sched.start();
}

// ── Helpers ─────────────────────────────────────────────────────

fn earlyFbMark(color: u32) void {
    const resp = framebuffer_req.response orelse return;
    if (resp.framebuffer_count == 0) return;
    const fb = resp.framebuffers[0];
    const pitch: usize = @intCast(fb.pitch);
    const base: usize = @intFromPtr(fb.address);
    var y: usize = 0;
    while (y < 64) : (y += 1) {
        var x: usize = 0;
        while (x < 64) : (x += 1) {
            const pixel: *volatile u32 = @ptrFromInt(base + y * pitch + x * 4);
            pixel.* = color;
        }
    }
}

fn drawBootScreen(fb: *volatile limine.Framebuffer) void {
    const width: usize = @intCast(fb.width);
    const height: usize = @intCast(fb.height);
    const pitch_bytes: usize = @intCast(fb.pitch);
    const addr = fb.address;
    for (0..height) |y| {
        const row: [*]volatile u32 = @ptrCast(@alignCast(addr + y * pitch_bytes));
        for (0..width) |x| {
            const gy: u32 = @intCast(@min(y * 6 / height, 5));
            const gx: u32 = @intCast(@min(x * 3 / width, 2));
            const shade: u32 = gy + gx;
            row[x] = shade | (shade << 8) | (shade << 16);
        }
    }
}

fn halt() noreturn {
    serial.puts("[HALT]\n");
    asm volatile ("cli");
    while (true) asm volatile ("hlt");
}

pub fn panic(msg: []const u8, _: ?*std.builtin.StackTrace, _: ?usize) noreturn {
    serial.puts("\n!!! PANIC: ");
    serial.puts(msg);
    serial.puts("\n");
    halt();
}
