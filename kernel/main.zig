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
const cap_stress = @import("cap_stress_test.zig");

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
    serial.puts("\n[BOOT]  VantaOS Phase 3 starting\n");

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
        @import("mm/slab.zig").init();
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
    syscall.verifySyscallFromRing3();

    // APIC & Interrupt routing
    interrupts.init(rsdp_req.response);
    if (rsdp_req.response) |resp| {
        @import("arch/x86_64/smp.zig").smp_init(resp.address);
    }
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

    // Run Phase 4 userspace end-to-end IPC integration test!
    runPhase4Test();

    serial.puts("══════════════════════════════════════════════\n");
    serial.puts("  VantaOS Phase 4 complete. Starting scheduler.\n");
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

const producer_elf = @embedFile("bin/producer");
const consumer_elf = @embedFile("bin/consumer");
var shared_port: port_mod.Port = .{};

pub fn runPhase4Test() void {
    serial.puts("[P4-TEST] Spawning userspace producer and consumer...\n");

    // 1. Create processes
    const prod_proc = proc.create("producer", 0) orelse {
        serial.puts("[FATAL] prod proc create failed\n");
        halt();
    };
    const cons_proc = proc.create("consumer", 0) orelse {
        serial.puts("[FATAL] cons proc create failed\n");
        halt();
    };

    // 2. Setup shared port in slot 1 of both processes
    const port_addr = @intFromPtr(&shared_port);
    _ = cap.cap_table_insert(&prod_proc.cap_table, port_addr, @intFromEnum(cap.CapType.Endpoint), 1); // Send=1
    _ = cap.cap_table_insert(&cons_proc.cap_table, port_addr, @intFromEnum(cap.CapType.Endpoint), 2); // Recv=2

    // 3. Parse ELFs
    const prod_elf_info = @import("elf.zig").parse_elf64(producer_elf) catch |err| {
        serial.puts("[FATAL] prod ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        halt();
    };
    const cons_elf_info = @import("elf.zig").parse_elf64(consumer_elf) catch |err| {
        serial.puts("[FATAL] cons ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        halt();
    };

    // 4. Load ELFs
    const prod_entry = @import("elf.zig").load_elf(prod_elf_info, producer_elf, prod_proc.space.pml4_phys) catch unreachable;
    const cons_entry = @import("elf.zig").load_elf(cons_elf_info, consumer_elf, cons_proc.space.pml4_phys) catch unreachable;

    // 5. Alloc stacks
    const prod_rsp = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = prod_proc.space.pml4_phys }, 4).?;
    _ = prod_rsp;
    const cons_rsp = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = cons_proc.space.pml4_phys }, 4).?;
    _ = cons_rsp;

    // 6. Push auxv/argc/envp/argv
    const prod_stack_top = @import("elf.zig").setupUserStack(prod_proc.space.pml4_phys, prod_entry, prod_elf_info);
    const cons_stack_top = @import("elf.zig").setupUserStack(cons_proc.space.pml4_phys, cons_entry, cons_elf_info);

    // 7. Create threads
    const ut_prod = thread.create_user(prod_entry, prod_stack_top, prod_proc.space.pml4_phys, prod_proc.pid) orelse {
        serial.puts("[FATAL] ut_prod create failed\n");
        halt();
    };
    const ut_cons = thread.create_user(cons_entry, cons_stack_top, cons_proc.space.pml4_phys, cons_proc.pid) orelse {
        serial.puts("[FATAL] ut_cons create failed\n");
        halt();
    };

    sched.enqueue(ut_prod);
    prod_proc.thread_count += 1;

    sched.enqueue(ut_cons);
    cons_proc.thread_count += 1;

    serial.puts("[P4-TEST] Spawning complete. Running scheduler...\n");
}
