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
    
    // Dump CpuLocal offsets
    const cpu_local = @import("arch/x86_64/cpu_local.zig");
    serial.puts("[DEBUG] CpuLocal size: ");
    serial.putDec(@sizeOf(cpu_local.CpuLocal));
    serial.puts("  timer_ticks offset: ");
    serial.putDec(@offsetOf(cpu_local.CpuLocal, "timer_ticks"));
    serial.puts("  watchdog_last_ticks offset: ");
    serial.putDec(@offsetOf(cpu_local.CpuLocal, "watchdog_last_ticks"));
    serial.puts("  watchdog_miss_count offset: ");
    serial.putDec(@offsetOf(cpu_local.CpuLocal, "watchdog_miss_count"));
    serial.puts("\n");

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
        const vmm_mod = @import("mm/vmm.zig");
        @import("arch/x86_64/smp.zig").smp_init(vmm_mod.virt2phys_hhdm(resp.address));
    }
    interrupts.routeIrq(1, 33, 0);
    serial.puts("[KBD]   PS/2 Keyboard routed (IRQ 1 -> Vector 33)\n");
    interrupts.routeIrq(11, 34, 0);
    serial.puts("[AHCI]  AHCI IRQ routed (IRQ 11 -> Vector 34)\n");

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

    // Spawn Registry Thread
    const reg_th = thread.create(registryThread) orelse {
        serial.puts("[FATAL] Failed to create service registry thread\n");
        halt();
    };
    sched.enqueue(reg_th);

    // Run Phase 7 userspace filesystem acceptance test!
    runPhase7Test();

    serial.puts("══════════════════════════════════════════════\n");
    serial.puts("  VantaOS Phase 7 complete. Starting scheduler.\n");
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

pub fn panic(msg: []const u8, _: ?*std.builtin.StackTrace, return_address: ?usize) noreturn {
    serial.puts("\n!!! PANIC: ");
    serial.puts(msg);
    if (return_address) |ra| {
        serial.puts(" at RIP=0x");
        serial.putHex(ra);
    }
    serial.puts("\n");
    halt();
}

const ns_elf = @embedFile("bin/ns");
const tmpfs_elf = @embedFile("bin/tmpfs");
const vantafs_elf = @embedFile("bin/vantafs");
const ahci_elf = @embedFile("bin/ahci");
const fs_test_elf = @embedFile("bin/fs_test");
const virtio_net_elf = @embedFile("bin/virtio_net");

var ns_port: port_mod.Port = .{};
var tmpfs_port: port_mod.Port = .{};
var vantafs_port: port_mod.Port = .{};
var ahci_port: port_mod.Port = .{};
var registry_port: port_mod.Port = .{};
var virtio_net_port: port_mod.Port = .{};

const RegistryEntry = struct {
    name: [32]u8,
    name_len: usize,
    entry: cap.CapEntry,
};

var registry_table: [64]RegistryEntry = undefined;
var registry_count: usize = 0;

pub fn registryThread() callconv(.c) noreturn {
    serial.puts("[REGISTRY] Service registry thread online.\n");
    while (true) {
        if (registry_port.recvBlocking()) |*msg| {
            if (msg.msg_type == 0x10) { // RegistryRegister
                const name = std.mem.sliceTo(msg.payload[0..32], 0);
                if (name.len > 0 and msg.transferred_caps[0].type != 0) {
                    if (registry_count < 64) {
                        var reg_entry = &registry_table[registry_count];
                        @memset(&reg_entry.name, 0);
                        @memcpy(reg_entry.name[0..name.len], name);
                        reg_entry.name_len = name.len;
                        reg_entry.entry = msg.transferred_caps[0];
                        // Clear recipient table metadata so duplicate derivations link cleanly
                        reg_entry.entry.old_table = null;
                        reg_entry.entry.old_index = 0;
                        registry_count += 1;
                        
                        serial.puts("[REGISTRY] Registered service '");
                        serial.puts(name);
                        serial.puts("'\n");
                    }
                }
            } else if (msg.msg_type == 0x11) { // RegistryLookup
                const name = std.mem.sliceTo(msg.payload[0..32], 0);
                var found = false;
                var found_idx: usize = 0;
                for (0..registry_count) |i| {
                    const reg_name = registry_table[i].name[0..registry_table[i].name_len];
                    if (std.mem.eql(u8, reg_name, name)) {
                        found = true;
                        found_idx = i;
                        break;
                    }
                }
                
                if (found) {
                    var reply = port_mod.Message{};
                    reply.msg_type = 0x11;
                    reply.flags.is_reply = true;
                    var cap_entry = registry_table[found_idx].entry;
                    cap_entry.rights = cap.Rights.EndpointSend; // Send only!
                    reply.transferred_caps[0] = cap_entry;
                    
                    _ = registry_port.send(&reply);
                } else {
                    var reply = port_mod.Message{};
                    reply.msg_type = 0x0003; // MSG_ERROR
                    reply.flags.is_reply = true;
                    @memcpy(reply.payload[0..4], "FAIL");
                    _ = registry_port.send(&reply);
                }
            }
        }
    }
}

pub fn runPhase7Test() void {
    serial.puts("[P7-TEST] Spawning userspace filesystem stacks...\n");

    // 1. Create processes
    const ns_proc = proc.create("ns", 0) orelse {
        serial.puts("[FATAL] ns proc create failed\n");
        halt();
    };
    const tmpfs_proc = proc.create("tmpfs", 0) orelse {
        serial.puts("[FATAL] tmpfs proc create failed\n");
        halt();
    };
    const vantafs_proc = proc.create("vantafs", 0) orelse {
        serial.puts("[FATAL] vantafs proc create failed\n");
        halt();
    };
    const ahci_proc = proc.create("ahci", 0) orelse {
        serial.puts("[FATAL] ahci proc create failed\n");
        halt();
    };
    const fs_test_proc = proc.create("fs_test", 0) orelse {
        serial.puts("[FATAL] fs_test proc create failed\n");
        halt();
    };
    const virtio_net_proc = proc.create("virtio_net", 0) orelse {
        serial.puts("[FATAL] virtio_net proc create failed\n");
        halt();
    };

    // 2. Setup capabilities
    // Setup NS
    _ = cap.cap_table_insert(&ns_proc.cap_table, @intFromPtr(&ns_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend | cap.Rights.EndpointRecv | cap.Rights.EndpointGrant);
    _ = cap.cap_table_insert(&ns_proc.cap_table, @intFromPtr(&registry_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend);

    // Setup tmpfs
    _ = cap.cap_table_insert(&tmpfs_proc.cap_table, @intFromPtr(&tmpfs_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend | cap.Rights.EndpointRecv | cap.Rights.EndpointGrant);
    _ = cap.cap_table_insert(&tmpfs_proc.cap_table, @intFromPtr(&registry_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend);

    // Setup VantaFS
    _ = cap.cap_table_insert(&vantafs_proc.cap_table, @intFromPtr(&vantafs_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend | cap.Rights.EndpointRecv | cap.Rights.EndpointGrant);
    _ = cap.cap_table_insert(&vantafs_proc.cap_table, @intFromPtr(&registry_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend);

    // Setup AHCI
    const bar5_phys = if (pci.ahci_bar5_phys != 0) pci.ahci_bar5_phys else b: {
        const p = pmm.allocPage().?;
        @memset(@as([*]u8, @ptrFromInt(vmm.phys2virt(p)))[0..4096], 0);
        break :b p;
    };
    _ = cap.cap_table_insert(&ahci_proc.cap_table, bar5_phys, @intFromEnum(cap.CapType.Memory), cap.Rights.MemoryRead | cap.Rights.MemoryWrite | cap.Rights.MemoryMap);
    _ = cap.cap_table_insert(&ahci_proc.cap_table, @intFromPtr(&ahci_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend | cap.Rights.EndpointRecv | cap.Rights.EndpointGrant);
    _ = cap.cap_table_insert(&ahci_proc.cap_table, @intFromPtr(&registry_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend);
    _ = cap.cap_table_insert(&ahci_proc.cap_table, 11, @intFromEnum(cap.CapType.DeviceIRQ), cap.Rights.DeviceIRQBind);

    // Setup fs_test
    _ = cap.cap_table_insert(&fs_test_proc.cap_table, @intFromPtr(&ns_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend);
    _ = cap.cap_table_insert(&fs_test_proc.cap_table, @intFromPtr(&registry_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend);

    // Setup virtio-net
    const virtio_net_bar0 = if (pci.virtio_net_bar0_phys != 0) pci.virtio_net_bar0_phys else b: {
        const p = pmm.allocPage().?;
        @memset(@as([*]u8, @ptrFromInt(vmm.phys2virt(p)))[0..4096], 0);
        break :b p;
    };
    _ = cap.cap_table_insert(&virtio_net_proc.cap_table, virtio_net_bar0, @intFromEnum(cap.CapType.Memory), cap.Rights.MemoryRead | cap.Rights.MemoryWrite | cap.Rights.MemoryMap);
    _ = cap.cap_table_insert(&virtio_net_proc.cap_table, @intFromPtr(&virtio_net_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend | cap.Rights.EndpointRecv | cap.Rights.EndpointGrant);
    _ = cap.cap_table_insert(&virtio_net_proc.cap_table, @intFromPtr(&registry_port), @intFromEnum(cap.CapType.Endpoint), cap.Rights.EndpointSend);
    _ = cap.cap_table_insert(&virtio_net_proc.cap_table, 11, @intFromEnum(cap.CapType.DeviceIRQ), cap.Rights.DeviceIRQBind);

    // 3. Parse ELFs
    const ns_elf_info = @import("elf.zig").parse_elf64(ns_elf) catch |err| {
        serial.puts("[FATAL] ns ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        halt();
    };
    const tmpfs_elf_info = @import("elf.zig").parse_elf64(tmpfs_elf) catch |err| {
        serial.puts("[FATAL] tmpfs ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        halt();
    };
    const vantafs_elf_info = @import("elf.zig").parse_elf64(vantafs_elf) catch |err| {
        serial.puts("[FATAL] vantafs ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        halt();
    };
    const ahci_elf_info = @import("elf.zig").parse_elf64(ahci_elf) catch |err| {
        serial.puts("[FATAL] ahci ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        halt();
    };
    const fs_test_elf_info = @import("elf.zig").parse_elf64(fs_test_elf) catch |err| {
        serial.puts("[FATAL] fs_test ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        halt();
    };
    const virtio_net_elf_info = @import("elf.zig").parse_elf64(virtio_net_elf) catch |err| {
        serial.puts("[FATAL] virtio_net ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        halt();
    };

    // 4. Load ELFs
    const ns_entry = @import("elf.zig").load_elf(ns_elf_info, ns_elf, ns_proc.space.pml4_phys) catch unreachable;
    const tmpfs_entry = @import("elf.zig").load_elf(tmpfs_elf_info, tmpfs_elf, tmpfs_proc.space.pml4_phys) catch unreachable;
    const vantafs_entry = @import("elf.zig").load_elf(vantafs_elf_info, vantafs_elf, vantafs_proc.space.pml4_phys) catch unreachable;
    const ahci_entry = @import("elf.zig").load_elf(ahci_elf_info, ahci_elf, ahci_proc.space.pml4_phys) catch unreachable;
    const fs_test_entry = @import("elf.zig").load_elf(fs_test_elf_info, fs_test_elf, fs_test_proc.space.pml4_phys) catch unreachable;
    const virtio_net_entry = @import("elf.zig").load_elf(virtio_net_elf_info, virtio_net_elf, virtio_net_proc.space.pml4_phys) catch unreachable;

    // 5. Alloc stacks
    _ = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = ns_proc.space.pml4_phys }, 16).?;
    _ = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = tmpfs_proc.space.pml4_phys }, 16).?;
    _ = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = vantafs_proc.space.pml4_phys }, 16).?;
    _ = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = ahci_proc.space.pml4_phys }, 16).?;
    _ = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = fs_test_proc.space.pml4_phys }, 16).?;
    _ = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = virtio_net_proc.space.pml4_phys }, 16).?;

    // 6. Setup stacks
    const ns_stack_top = @import("elf.zig").setupUserStack(ns_proc.space.pml4_phys, ns_entry, ns_elf_info);
    const tmpfs_stack_top = @import("elf.zig").setupUserStack(tmpfs_proc.space.pml4_phys, tmpfs_entry, tmpfs_elf_info);
    const vantafs_stack_top = @import("elf.zig").setupUserStack(vantafs_proc.space.pml4_phys, vantafs_entry, vantafs_elf_info);
    const ahci_stack_top = @import("elf.zig").setupUserStack(ahci_proc.space.pml4_phys, ahci_entry, ahci_elf_info);
    const fs_test_stack_top = @import("elf.zig").setupUserStack(fs_test_proc.space.pml4_phys, fs_test_entry, fs_test_elf_info);
    const virtio_net_stack_top = @import("elf.zig").setupUserStack(virtio_net_proc.space.pml4_phys, virtio_net_entry, virtio_net_elf_info);

    // 7. Create threads
    const ut_ns = thread.create_user(ns_entry, ns_stack_top, ns_proc.space.pml4_phys, ns_proc.pid) orelse {
        serial.puts("[FATAL] ut_ns create failed\n");
        halt();
    };
    const ut_tmpfs = thread.create_user(tmpfs_entry, tmpfs_stack_top, tmpfs_proc.space.pml4_phys, tmpfs_proc.pid) orelse {
        serial.puts("[FATAL] ut_tmpfs create failed\n");
        halt();
    };
    const ut_vantafs = thread.create_user(vantafs_entry, vantafs_stack_top, vantafs_proc.space.pml4_phys, vantafs_proc.pid) orelse {
        serial.puts("[FATAL] ut_vantafs create failed\n");
        halt();
    };
    const ut_ahci = thread.create_user(ahci_entry, ahci_stack_top, ahci_proc.space.pml4_phys, ahci_proc.pid) orelse {
        serial.puts("[FATAL] ut_ahci create failed\n");
        halt();
    };
    const ut_fs_test = thread.create_user(fs_test_entry, fs_test_stack_top, fs_test_proc.space.pml4_phys, fs_test_proc.pid) orelse {
        serial.puts("[FATAL] ut_fs_test create failed\n");
        halt();
    };
    const ut_virtio_net = thread.create_user(virtio_net_entry, virtio_net_stack_top, virtio_net_proc.space.pml4_phys, virtio_net_proc.pid) orelse {
        serial.puts("[FATAL] ut_virtio_net create failed\n");
        halt();
    };

    sched.enqueue(ut_ns);
    ns_proc.thread_count += 1;

    sched.enqueue(ut_tmpfs);
    tmpfs_proc.thread_count += 1;

    sched.enqueue(ut_vantafs);
    vantafs_proc.thread_count += 1;

    sched.enqueue(ut_ahci);
    ahci_proc.thread_count += 1;

    sched.enqueue(ut_fs_test);
    fs_test_proc.thread_count += 1;

    sched.enqueue(ut_virtio_net);
    virtio_net_proc.thread_count += 1;

    serial.puts("[P7-TEST] Spawning complete. Running scheduler...\n");
}

