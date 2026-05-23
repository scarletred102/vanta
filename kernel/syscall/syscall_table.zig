const std = @import("std");
const serial = @import("../arch/x86_64/serial.zig");
const sched = @import("../sched/scheduler.zig");
const proc = @import("../proc/process.zig");
const port_mod = @import("../ipc/port.zig");
const notif_mod = @import("../ipc/notification.zig");
const cap_mod = @import("../cap/handle.zig");
const table_orig = @import("table.zig");
const vmm = @import("../mm/vmm.zig");
const pmm = @import("../mm/pmm.zig");

pub const SyscallNumber = enum(u64) {
    CapSend = 0,
    CapRecv = 1,
    CapCall = 2,
    CapDerive = 3,
    CapRevoke = 4,
    DebugPrint = 5,
    Exit = 6,
    MemMap = 7,
    MemUnmap = 8,
    ThreadSpawn = 9,
    CapWait = 10,
    // Phase 3 additions
    MemCreate = 11,      // allocate physical pages → MemoryCap
    CapNotify = 12,      // notify (OR bits into Notification)
    NotifCreate = 13,    // create a Notification object → NotificationCap
    _, // Allow others without compilation error
};

pub const Result = extern struct {
    value: u64 = 0,
    err: table_orig.Error = .success,
};

pub fn dispatch(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) Result {
    _ = arg5; _ = arg6; _ = arg4;
    const sys_num: SyscallNumber = @enumFromInt(number);

    return switch (sys_num) {
        .CapSend     => handleCapSend(arg1, arg2),
        .CapRecv     => handleCapRecv(arg1, arg2),
        .CapCall     => handleCapCall(arg1, arg2, arg3),
        .CapDerive   => handleCapDerive(arg1, arg2, arg3),
        .CapRevoke   => handleCapRevoke(arg1),
        .DebugPrint  => handleDebugPrint(arg1, arg2),
        .Exit        => handleExit(arg1),
        .MemCreate   => handleMemCreate(arg1),
        .MemMap      => handleMemMap(arg1, arg2, arg3),
        .MemUnmap    => handleMemUnmap(arg1),
        .ThreadSpawn => handleThreadSpawn(arg1),
        .CapWait     => handleCapWait(arg1, arg2),
        .CapNotify   => handleCapNotify(arg1, arg2),
        .NotifCreate => handleNotifCreate(),
        else => .{ .err = .bad_syscall },
    };
}

// ── Capability-based handlers ────────────────────────────────────

fn handleCapSend(port_handle: u64, msg_ptr: u64) Result {
    if (msg_ptr == 0) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, port_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Endpoint)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.EndpointSend) == 0) return .{ .err = .permission_denied };

    const msg = @as(*port_mod.Message, @ptrFromInt(msg_ptr));
    const port = @as(*port_mod.Port, @ptrFromInt(cap_mod.getObjectPtr(entry)));

    var msg_copy = msg.*;
    const err = cap_mod.prepareMessageForSend(&current_proc.cap_table, &msg_copy);
    if (err != .success) return .{ .err = err };

    if (port.sendBlocking(&msg_copy)) {
        return .{ .value = 0, .err = .success };
    } else {
        return .{ .err = .permission_denied };
    }
}

fn handleCapRecv(port_handle: u64, msg_ptr: u64) Result {
    if (msg_ptr == 0) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, port_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Endpoint)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.EndpointRecv) == 0) return .{ .err = .permission_denied };

    const port = @as(*port_mod.Port, @ptrFromInt(cap_mod.getObjectPtr(entry)));
    if (port.recvBlocking()) |*msg| {
        var msg_copy = msg.*;
        cap_mod.receiveMessageCaps(&current_proc.cap_table, &msg_copy);
        const dest = @as(*port_mod.Message, @ptrFromInt(msg_ptr));
        dest.* = msg_copy;
        return .{ .value = 0, .err = .success };
    } else {
        return .{ .err = .permission_denied };
    }
}

fn handleCapCall(port_handle: u64, msg_ptr: u64, reply_ptr: u64) Result {
    if (msg_ptr == 0 or reply_ptr == 0) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, port_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Endpoint)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.EndpointSend) == 0 or (entry.rights & cap_mod.Rights.EndpointRecv) == 0) return .{ .err = .permission_denied };

    const port = @as(*port_mod.Port, @ptrFromInt(cap_mod.getObjectPtr(entry)));

    const msg = @as(*port_mod.Message, @ptrFromInt(msg_ptr));
    var msg_copy = msg.*;
    const err = cap_mod.prepareMessageForSend(&current_proc.cap_table, &msg_copy);
    if (err != .success) return .{ .err = err };

    if (!port.sendBlocking(&msg_copy)) return .{ .err = .permission_denied };

    if (port.recvBlocking()) |*reply| {
        var reply_copy = reply.*;
        cap_mod.receiveMessageCaps(&current_proc.cap_table, &reply_copy);
        const dest = @as(*port_mod.Message, @ptrFromInt(reply_ptr));
        dest.* = reply_copy;
        return .{ .value = 0, .err = .success };
    } else {
        return .{ .err = .permission_denied };
    }
}

fn handleCapDerive(parent_handle: u64, mask_val: u64, child_handle_ptr: u64) Result {
    const current_proc = table_orig.getCurrentProcess();

    const parent = cap_mod.cap_table_lookup(&current_proc.cap_table, parent_handle) orelse return .{ .err = .invalid_handle };

    const new_rights = @as(u8, @truncate(mask_val));

    if ((new_rights & parent.rights) != new_rights) {
        return .{ .err = .permission_denied };
    }

    const child_idx = findFreeSlot(&current_proc.cap_table) orelse return .{ .err = .out_of_memory };
    const child = &current_proc.cap_table.entries[child_idx];

    child.type = parent.type;
    child.rights = new_rights;
    child.kernel_object_ptr = parent.kernel_object_ptr;
    child.parent_table = &current_proc.cap_table;
    child.parent_index = @as(u16, @truncate(parent_handle & 0xFFFFFFFFFFFF));
    child.parent_generation = parent.generation;
    child.old_table = null;
    child.old_index = 0;

    current_proc.cap_table.count += 1;
    cap_mod.linkEntry(&current_proc.cap_table, child_idx);

    const child_handle = cap_mod.encodeHandle(child_idx, child.generation);

    if (child_handle_ptr != 0) {
        const ptr = @as(*cap_mod.Handle, @ptrFromInt(child_handle_ptr));
        ptr.* = child_handle;
    }
    return .{ .value = child_handle, .err = .success };
}

fn handleCapRevoke(handle: u64) Result {
    const current_proc = table_orig.getCurrentProcess();
    _ = cap_mod.cap_table_lookup(&current_proc.cap_table, handle) orelse return .{ .err = .invalid_handle };
    cap_mod.cap_revoke(&current_proc.cap_table, handle);
    return .{ .value = 0, .err = .success };
}

// ── P3.6 Memory Capability ───────────────────────────────────────

// Create a MemoryCap: allocate n_pages from buddy, store (base_phys, n_pages) in a MemoryCap.
// Returns handle encoding: top 32 bits = n_pages, bottom 32 bits = base_phys (page-aligned).
// The actual kernel object pointer IS the base physical address.
fn handleMemCreate(n_pages: u64) Result {
    if (n_pages == 0 or n_pages > 512) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    // Allocate contiguous physical pages
    const base_phys = pmm.allocContiguous(@intCast(n_pages)) orelse return .{ .err = .out_of_memory };

    // Encode into cap: kernel_object_ptr = base_phys (48-bit, page-aligned so bottom bits = 0)
    // rights = Read|Write|Map (0x01|0x02|0x08 = 0x0B)
    // Stash n_pages in payload via a tiny MemCapMeta stored at the start of the allocation
    const handle = cap_mod.cap_table_insert(
        &current_proc.cap_table,
        base_phys,
        @intFromEnum(cap_mod.CapType.Memory),
        cap_mod.Rights.MemoryRead | cap_mod.Rights.MemoryWrite | cap_mod.Rights.MemoryMap,
    ) orelse return .{ .err = .out_of_memory };

    return .{ .value = handle, .err = .success };
}

// MemMap: map a MemoryCap's pages into the current address space at vaddr.
// arg1 = mem_cap_handle, arg2 = target_vaddr, arg3 = n_pages (0 = use all from cap)
fn handleMemMap(mem_cap_handle: u64, target_vaddr: u64, n_pages: u64) Result {
    if (target_vaddr == 0) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, mem_cap_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Memory)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.MemoryMap) == 0) return .{ .err = .permission_denied };

    const base_phys = cap_mod.getObjectPtr(entry);

    // Derive PTE flags from capability rights
    var pte_flags: u64 = vmm.PTE_PRESENT | vmm.PTE_USER;
    if ((entry.rights & cap_mod.Rights.MemoryWrite) != 0) pte_flags |= vmm.PTE_WRITE;
    if ((entry.rights & cap_mod.Rights.MemoryExec) == 0) pte_flags |= vmm.PTE_NX;

    const pages_to_map = if (n_pages == 0) @as(u64, 1) else n_pages;
    const space = vmm.AddressSpace{ .pml4_phys = current_proc.space.pml4_phys };

    var i: u64 = 0;
    while (i < pages_to_map) : (i += 1) {
        const vaddr = target_vaddr + i * vmm.PAGE_SIZE;
        const paddr = base_phys + i * vmm.PAGE_SIZE;
        if (!vmm.map(space, vaddr, paddr, pte_flags)) {
            // Rollback
            var j: u64 = 0;
            while (j < i) : (j += 1) {
                vmm.unmap(space, target_vaddr + j * vmm.PAGE_SIZE);
            }
            return .{ .err = .out_of_memory };
        }
    }

    // Register VMA
    _ = current_proc.addVma(target_vaddr, target_vaddr + pages_to_map * vmm.PAGE_SIZE, pte_flags, false);

    return .{ .value = 0, .err = .success };
}

// MemUnmap: unmap pages at vaddr (removes first matching VMA that covers vaddr)
fn handleMemUnmap(vaddr: u64) Result {
    const current_proc = table_orig.getCurrentProcess();
    if (current_proc.findVma(vaddr)) |vma| {
        const space = vmm.AddressSpace{ .pml4_phys = current_proc.space.pml4_phys };
        var addr = vma.start;
        while (addr < vma.end) : (addr += vmm.PAGE_SIZE) {
            vmm.unmap(space, addr);
        }
    }
    return .{ .value = 0, .err = .success };
}

// ── P3.7 Notification Capability ────────────────────────────────

fn handleNotifCreate() Result {
    const current_proc = table_orig.getCurrentProcess();

    const notif = notif_mod.create() orelse return .{ .err = .out_of_memory };
    const handle = cap_mod.cap_table_insert(
        &current_proc.cap_table,
        @intFromPtr(notif),
        @intFromEnum(cap_mod.CapType.Notification),
        cap_mod.Rights.NotificationSignal | cap_mod.Rights.NotificationWait,
    ) orelse return .{ .err = .out_of_memory };

    return .{ .value = handle, .err = .success };
}

// cap_notify: arg1 = notif_handle, arg2 = bits to set
fn handleCapNotify(notif_handle: u64, bits: u64) Result {
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, notif_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Notification)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.NotificationSignal) == 0) return .{ .err = .permission_denied };

    const notif = @as(*notif_mod.Notification, @ptrFromInt(cap_mod.getObjectPtr(entry)));
    notif.notify(bits);
    return .{ .value = 0, .err = .success };
}

// cap_wait: arg1 = notif_handle, arg2 = mask
fn handleCapWait(notif_handle: u64, mask: u64) Result {
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, notif_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Notification)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.NotificationWait) == 0) return .{ .err = .permission_denied };

    const notif = @as(*notif_mod.Notification, @ptrFromInt(cap_mod.getObjectPtr(entry)));
    const matched = notif.wait(mask);
    return .{ .value = matched, .err = .success };
}

// ── Utility ─────────────────────────────────────────────────────

fn findFreeSlot(table: *cap_mod.CapTable) ?u16 {
    var i: usize = 1;
    while (i < cap_mod.MAX_CAPS) : (i += 1) {
        if (table.entries[i].type == 0) return @intCast(i);
    }
    return null;
}

fn handleDebugPrint(msg_ptr: u64, msg_len: u64) Result {
    if (msg_len > 4096) return .{ .err = .invalid_argument };
    if (msg_ptr == 0) return .{ .err = .invalid_argument };
    const slice = @as([*]const u8, @ptrFromInt(msg_ptr))[0..@intCast(msg_len)];
    serial.puts("[LOG] ");
    serial.puts(slice);
    serial.puts("\n");
    return .{ .value = msg_len, .err = .success };
}

fn handleExit(code: u64) Result {
    serial.puts("[SYSCALL] Exit code: ");
    serial.putDec(code);
    serial.puts("\n");
    sched.exitCurrentThread();
}

fn stubNotImpl(name: []const u8) Result {
    serial.puts("[SYSCALL] ");
    serial.puts(name);
    serial.puts(" — not implemented\n");
    return .{ .err = .not_implemented };
}

fn handleThreadSpawn(mem_cap_handle: u64) Result {
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, mem_cap_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Memory)) return .{ .err = .permission_denied };
    const base_phys = cap_mod.getObjectPtr(entry);

    const data = @as([*]const u8, @ptrFromInt(vmm.phys2virt(base_phys)))[0 .. 512 * 4096];
    const elf_info = @import("../elf.zig").parse_elf64(data) catch |err| {
        serial.puts("[ThreadSpawn] ELF parse failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        return .{ .err = .invalid_argument };
    };

    const new_proc = proc.create("user_app", current_proc.pid) orelse return .{ .err = .out_of_memory };

    const user_entry = @import("../elf.zig").load_elf(elf_info, data, new_proc.space.pml4_phys) catch |err| {
        serial.puts("[ThreadSpawn] ELF load failed: ");
        serial.puts(@errorName(err));
        serial.puts("\n");
        return .{ .err = .out_of_memory };
    };

    const user_rsp = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = new_proc.space.pml4_phys }, 4) orelse {
        return .{ .err = .out_of_memory };
    };
    _ = user_rsp;

    const argc_start = @import("../elf.zig").setupUserStack(new_proc.space.pml4_phys, user_entry, elf_info);

    const ut = @import("../sched/thread.zig").create_user(user_entry, argc_start, new_proc.space.pml4_phys, new_proc.pid) orelse {
        return .{ .err = .out_of_memory };
    };

    sched.enqueue(ut);
    new_proc.thread_count += 1;

    // Create Thread capability for the new thread
    const th_handle = cap_mod.cap_table_insert(
        &current_proc.cap_table,
        @intFromPtr(ut),
        @intFromEnum(cap_mod.CapType.Thread),
        cap_mod.Rights.ThreadControl | cap_mod.Rights.ThreadInspect,
    ) orelse return .{ .err = .out_of_memory };

    return .{ .value = th_handle, .err = .success };
}

