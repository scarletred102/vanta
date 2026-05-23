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
const interrupts = @import("arch/x86_64/interrupts.zig");
const cap = @import("cap/handle.zig");
const pci = @import("drivers/pci.zig");
const port_mod = @import("ipc/port.zig");

// ── Limine Requests ─────────────────────────────────────────────

pub export var base_revision: limine.BaseRevision linksection(".limine_requests_start") = .{};
pub export var framebuffer_req: limine.FramebufferRequest linksection(".limine_requests") = .{};
pub export var memmap_req: limine.MemoryMapRequest linksection(".limine_requests") = .{};
pub export var hhdm_req: limine.HhdmRequest linksection(".limine_requests") = .{};
pub export var kaddr_req: limine.KernelAddressRequest linksection(".limine_requests") = .{};
pub export var rsdp_req: limine.RsdpRequest linksection(".limine_requests") = .{};
pub export var requests_end: [2]u64 linksection(".limine_requests_end") = limine.REQUESTS_END_MARKER;

// ── Entry ───────────────────────────────────────────────────────

export fn _start() callconv(.c) noreturn {
    asm volatile ("cli");
    cpu.enableSse();
    kmain();
    halt();
}

// ── Test capability-based IPC threads ───────────────────────────

var test_port: port_mod.Port = .{};
var root_handle: cap.Handle = 0;
var write_handle: cap.Handle = 0;

fn consumerThread() callconv(.c) noreturn {
    var msg: port_mod.Message = undefined;
    var msg_count: u64 = 0;
    
    while (true) {
        // Call the cap_recv system call directly using table.dispatch
        const res = @import("syscall/table.zig").dispatch(
            @intFromEnum(@import("syscall/table.zig").Syscall.cap_recv),
            root_handle,
            @intFromPtr(&msg),
            0, 0, 0, 0
        );
        
        if (res.err != .success) {
            serial.puts("[CONSUMER] recv failed! Error: ");
            serial.putDec(@intFromEnum(res.err));
            serial.puts("\n");
            sched.yield();
            continue;
        }
        
        msg_count += 1;
        const payload = msg.readPayload(0, 32);
        const actual_str = std.mem.sliceTo(payload, 0);
        serial.puts("[CONSUMER] Got Message #");
        serial.putDec(msg_count);
        serial.puts(": '");
        serial.puts(actual_str);
        serial.puts("'\n");
    }
}

fn producerThread() callconv(.c) noreturn {
    var msg: port_mod.Message = .{};
    var msg_count: u64 = 0;
    
    while (true) {
        msg_count += 1;
        
        // Format message
        var buf: [32]u8 = [_]u8{0} ** 32;
        const msg_str = std.fmt.bufPrint(&buf, "VantaOS Msg #{d}", .{msg_count}) catch unreachable;
        msg.writePayload(0, msg_str);
        
        serial.puts("[PRODUCER] Sending Message #");
        serial.putDec(msg_count);
        serial.puts("...\n");
        
        // Call the cap_send system call
        const res = @import("syscall/table.zig").dispatch(
            @intFromEnum(@import("syscall/table.zig").Syscall.cap_send),
            write_handle,
            @intFromPtr(&msg),
            0, 0, 0, 0
        );
        
        if (res.err != .success) {
            serial.puts("[PRODUCER] send failed! Error: ");
            serial.putDec(@intFromEnum(res.err));
            serial.puts("\n");
        }
        
        // Sleep or busy-loop a bit so it's readable
        var delay: u64 = 0;
        while (delay < 1_000_000) : (delay += 1) {
            asm volatile ("pause");
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

    // VMM
    if (hhdm_req.response) |h| {
        vmm.init(h);
    } else {
        serial.puts("[FATAL] no HHDM\n");
        halt();
    }

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

    // Kernel process
    proc.initKernelProc();
    serial.puts("[PROC]  kernel proc (pid 0) initialized\n");

    // SYSCALL MSRs
    syscall.init();
    // To run the Ring 3 automated syscall/sysret MSR verification test:
    // syscall.verifySyscallFromRing3();

    // APIC & Interrupt routing
    interrupts.init(rsdp_req.response);
    interrupts.routeIrq(1, 33, 0);
    serial.puts("[KBD]   PS/2 Keyboard routed (IRQ 1 -> Vector 33)\n");

    // PCI Bus Scanner
    pci.init();

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

    // Create a capability for our test port with full rights
    const port_addr = @intFromPtr(&test_port);
    const root_cap = cap.Capability{
        .obj_type = .ipc_port,
        .rights = cap.Rights.ALL,
        .object = port_addr,
        .parent = null,
        .generation = 0,
        .owner = 0, // kernel process
    };
    
    // Register the port capability in the kernel process's cap table
    root_handle = proc.kernel_proc.cap_table.insert(root_cap) orelse {
        serial.puts("[FATAL] Failed to insert root port cap\n");
        halt();
    };
    
    // Now let's derive a Write-Only capability mask (disable read, enable write and derive)
    var write_only_rights = cap.Rights.ALL;
    write_only_rights.read = false;
    
    // Derive it!
    write_handle = proc.kernel_proc.cap_table.derive(root_handle, write_only_rights) orelse {
        serial.puts("[FATAL] Failed to derive write cap\n");
        halt();
    };
    
    serial.puts("[TEST]  IPC Caps initialized: Root Handle=");
    serial.putDec(root_handle);
    serial.puts("  Write-Only Handle=");
    serial.putDec(write_handle);
    serial.puts("\n");

    // Spawn producer and consumer threads
    const ta = thread.create(producerThread) orelse {
        serial.puts("[FATAL] producer thread create failed\n");
        halt();
    };
    const tb = thread.create(consumerThread) orelse {
        serial.puts("[FATAL] consumer thread create failed\n");
        halt();
    };
    sched.enqueue(ta);
    sched.enqueue(tb);
    serial.puts("[SCHED] producer and consumer threads queued\n");

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
