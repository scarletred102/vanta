const std = @import("std");
const serial = @import("../arch/x86_64/serial.zig");
const sched = @import("../sched/scheduler.zig");
const proc = @import("../proc/process.zig");
const port_mod = @import("../ipc/port.zig");
const notif_mod = @import("../ipc/notification.zig");
const shm_mod = @import("../ipc/shm.zig");
const cap_mod = @import("../cap/handle.zig");
const table_orig = @import("table.zig");
const vmm = @import("../mm/vmm.zig");
const pmm = @import("../mm/pmm.zig");
const personality = @import("../ipc/personality.zig");
const thread_mod = @import("../sched/thread.zig");
const elf_mod = @import("../elf.zig");

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
    MemPhys = 14,        // get physical base address of Memory capability
    IrqBind = 15,        // bind DeviceIRQ capability to Notification
    ShmCreate = 16,      // allocate shared memory pages → ShmCap
    ShmMap = 17,         // map ShmCap pages into current address space
    CapSendNb = 18,      // non-blocking cap_send; returns EBUSY if queue full
    CapPoll = 19,        // wait on multiple caps; returns index of first ready
    PersonalitySpawn = 20,  // spawn Linux ELF under personality server
    ProcessMmap = 21,       // map anonymous pages into a process by pid
    ProcessMunmap = 22,     // unmap pages from a process by pid
    ThreadSetFsBase = 23,   // set FS base (IA32_FS_BASE) for a thread cap
    IrqReadByte = 24,       // read one byte from per-IRQ ring buffer (DeviceIRQ cap required)
    _, // Allow others without compilation error
};

pub const Result = extern struct {
    value: u64 = 0,
    err: table_orig.Error = .success,
};

// ── Userspace pointer validation ─────────────────────────────────
// x86_64 canonical user VA space: 0x0000_0000_0000_0000 – 0x0000_7FFF_FFFF_FFFF
const USER_VA_MAX: u64 = 0x0000_8000_0000_0000;

inline fn validateUserPtr(ptr: u64, size: u64) bool {
    if (ptr == 0) return false;
    if (ptr >= USER_VA_MAX) return false;
    if (size > 0 and ptr > USER_VA_MAX - size) return false;
    return true;
}

// Returns true if the caller holds any Thread cap whose owning process matches target_pid.
fn callerHoldsThreadCapForPid(cap_table: *cap_mod.CapTable, target_pid: u32) bool {
    var i: usize = 1;
    while (i < cap_mod.MAX_CAPS) : (i += 1) {
        const entry = &cap_table.entries[i];
        if (entry.type != @intFromEnum(cap_mod.CapType.Thread)) continue;
        const t = @as(*thread_mod.Thread, @ptrFromInt(cap_mod.getObjectPtr(entry)));
        if (t.proc_id == target_pid) return true;
    }
    return false;
}

pub fn dispatch(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) Result {
    // ── Linux personality fast path ─────────────────────────────────
    // If the calling thread is a Linux thread, route ALL syscalls through SHM.
    {
        const cpu = @import("../arch/x86_64/cpu_local.zig").get_cpu_local();
        if (cpu.current_thread) |ct| {
            if (ct.personality_shm_phys != 0) {
                const blk = @as(*volatile personality.SyscallShmBlock,
                    @ptrFromInt(@import("../mm/vmm.zig").phys2virt(ct.personality_shm_phys)));
                blk.nr   = number;
                blk.arg0 = arg1;
                blk.arg1 = arg2;
                blk.arg2 = arg3;
                blk.arg3 = arg4;
                blk.arg4 = arg5;
                blk.arg5 = arg6;
                // Signal personality server, then block until it replies.
                ct.personality_ping.?.notify(1);
                _ = ct.personality_pong.?.wait(1);
                const rv = blk.retval;
                if (rv < 0) {
                    return .{ .value = 0, .err = .permission_denied };
                }
                return .{ .value = @bitCast(rv), .err = .success };
            }
        }
    }
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
        .MemPhys     => handleMemPhys(arg1),
        .IrqBind     => handleIrqBind(arg1, arg2),
        .ShmCreate   => handleShmCreate(arg1),
        .ShmMap      => handleShmMap(arg1, arg2),
        .CapSendNb   => handleCapSendNb(arg1, arg2),
        .CapPoll     => handleCapPoll(arg1, arg2, arg3),
        .PersonalitySpawn => handlePersonalitySpawn(arg1, arg2),
        .ProcessMmap     => handleProcessMmap(arg1, arg2, arg3, arg4),
        .ProcessMunmap   => handleProcessMunmap(arg1, arg2, arg3),
        .ThreadSetFsBase => handleThreadSetFsBase(arg1, arg2),
        .IrqReadByte     => handleIrqReadByte(arg1),
        else => .{ .err = .bad_syscall },
    };
}

// ── Capability-based handlers ────────────────────────────────────

fn handleCapSend(port_handle: u64, msg_ptr: u64) Result {
    if (!validateUserPtr(msg_ptr, @sizeOf(port_mod.Message))) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, port_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Endpoint)) {
        if (entry.type == @intFromEnum(cap_mod.CapType.Thread)) {
            if ((entry.rights & cap_mod.Rights.ThreadControl) == 0) return .{ .err = .permission_denied };
            const msg = @as(*port_mod.Message, @ptrFromInt(msg_ptr));
            if (msg.msg_type == 0x01) { // ThreadStart
                const target_thread = @as(*thread_mod.Thread, @ptrFromInt(cap_mod.getObjectPtr(entry)));
                const entry_rip = std.mem.readInt(u64, msg.payload[0..8], .little);
                const stack_top = std.mem.readInt(u64, msg.payload[8..16], .little);
                
                target_thread.user_entry = entry_rip;
                target_thread.user_stack = stack_top;
                
                sched.enqueue(target_thread);
                serial.puts("[ThreadSpawn] Thread started successfully\n");
                return .{ .value = 0, .err = .success };
            }
        }
        return .{ .err = .permission_denied };
    }
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
    if (!validateUserPtr(msg_ptr, @sizeOf(port_mod.Message))) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, port_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Endpoint)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.EndpointRecv) == 0) return .{ .err = .permission_denied };

    const port = @as(*port_mod.Port, @ptrFromInt(cap_mod.getObjectPtr(entry)));
    if (port.recvBlockingFiltered(false)) |*msg| {
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
    if (!validateUserPtr(msg_ptr, @sizeOf(port_mod.Message)) or
        !validateUserPtr(reply_ptr, @sizeOf(port_mod.Message))) {
        serial.puts("[CAPCALL] validate fail\n");
        return .{ .err = .invalid_argument };
    }
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, port_handle) orelse {
        serial.puts("[CAPCALL] lookup fail\n");
        return .{ .err = .invalid_handle };
    };
    if (entry.type != @intFromEnum(cap_mod.CapType.Endpoint)) {
        serial.puts("[CAPCALL] not endpoint\n");
        return .{ .err = .permission_denied };
    }
    if ((entry.rights & cap_mod.Rights.EndpointSend) == 0 or (entry.rights & cap_mod.Rights.EndpointRecv) == 0) {
        serial.puts("[CAPCALL] no rights h=");
        serial.putHex(port_handle);
        serial.puts(" r=");
        serial.putHex(entry.rights);
        serial.puts(" pid=");
        serial.putHex(current_proc.pid);
        serial.putc('\n');
        return .{ .err = .permission_denied };
    }

    const port = @as(*port_mod.Port, @ptrFromInt(cap_mod.getObjectPtr(entry)));

    const msg = @as(*port_mod.Message, @ptrFromInt(msg_ptr));
    var msg_copy = msg.*;
    const err = cap_mod.prepareMessageForSend(&current_proc.cap_table, &msg_copy);
    if (err != .success) {
        serial.puts("[CAPCALL] prepSend fail\n");
        return .{ .err = err };
    }

    // Acquire the per-port RPC lock so that only one thread is in the
    // send→recv cycle at a time.  This prevents reply-stealing on SMP
    // when multiple processes share a port (e.g. the registry).
    port.acquireRpcLock();

    if (!port.sendBlocking(&msg_copy)) {
        port.releaseRpcLock();
        serial.puts("[CAPCALL] sendBlocking fail\n");
        return .{ .err = .permission_denied };
    }

    if (msg_copy.msg_type == 0x11) { // NS_LOOKUP trace
        serial.puts("[CAPCALL] lookup sent, waiting for reply pid=");
        serial.putHex(current_proc.pid);
        serial.putc('\n');
    }

    if (port.recvBlockingFiltered(true)) |*reply| {
        port.releaseRpcLock();
        var reply_copy = reply.*;
        cap_mod.receiveMessageCaps(&current_proc.cap_table, &reply_copy);
        const dest = @as(*port_mod.Message, @ptrFromInt(reply_ptr));
        dest.* = reply_copy;
        return .{ .value = 0, .err = .success };
    } else {
        port.releaseRpcLock();
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
        if (!validateUserPtr(child_handle_ptr, @sizeOf(cap_mod.Handle))) return .{ .err = .invalid_argument };
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
    if (target_vaddr == 0 or target_vaddr >= USER_VA_MAX) return .{ .err = .invalid_argument };
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
        if (!vmm.mapNoFlush(space, vaddr, paddr, pte_flags)) {
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
    if (!validateUserPtr(msg_ptr, msg_len)) return .{ .err = .invalid_argument };
    const slice = @as([*]const u8, @ptrFromInt(msg_ptr))[0..@intCast(msg_len)];
    const cur_proc = table_orig.getCurrentProcess();
    serial.puts("[LOG:");
    serial.puts(std.mem.sliceTo(&cur_proc.name, 0));
    serial.puts("] ");
    serial.puts(slice);
    if (msg_len == 0 or slice[msg_len - 1] != '\n') serial.puts("\n");
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
        if (err == error.InvalidMagic) {
            // Treat as spawning a thread in the current process!
            const ut = thread_mod.create_user(0, 0, current_proc.space.pml4_phys, current_proc.pid) orelse {
                return .{ .err = .out_of_memory };
            };
            ut.state = .ready;
            current_proc.thread_count += 1;

            const th_handle = cap_mod.cap_table_insert(
                &current_proc.cap_table,
                @intFromPtr(ut),
                @intFromEnum(cap_mod.CapType.Thread),
                cap_mod.Rights.ThreadControl | cap_mod.Rights.ThreadInspect,
            ) orelse return .{ .err = .out_of_memory };

            serial.puts("[ThreadSpawn] Spawning thread in current process\n");
            return .{ .value = th_handle, .err = .success };
        }
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

    const stack_top = new_proc.user_stack_top;
    new_proc.user_stack_top -= (4 + 1) * vmm.PAGE_SIZE; // 4 stack pages + 1 guard
    _ = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = new_proc.space.pml4_phys }, 4, stack_top) orelse {
        return .{ .err = .out_of_memory };
    };

    const argc_start = @import("../elf.zig").setupUserStack(new_proc.space.pml4_phys, user_entry, elf_info, stack_top);

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

fn handleMemPhys(mem_cap_handle: u64) Result {
    const current_proc = table_orig.getCurrentProcess();
    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, mem_cap_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Memory)) return .{ .err = .permission_denied };
    const base_phys = cap_mod.getObjectPtr(entry);
    return .{ .value = base_phys, .err = .success };
}

// ── Phase 9: ShmCreate / ShmMap ─────────────────────────────────

// shm_create(n_pages) → shm_cap_handle
fn handleShmCreate(n_pages: u64) Result {
    if (n_pages == 0 or n_pages > 512) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const shm = shm_mod.create(@intCast(n_pages)) orelse return .{ .err = .out_of_memory };

    const handle = cap_mod.cap_table_insert(
        &current_proc.cap_table,
        @intFromPtr(shm),
        @intFromEnum(cap_mod.CapType.SharedMemory),
        cap_mod.Rights.ShmRead | cap_mod.Rights.ShmWrite,
    ) orelse return .{ .err = .out_of_memory };

    return .{ .value = handle, .err = .success };
}

// shm_map(shm_cap_handle, vaddr) → 0 on success
fn handleShmMap(shm_cap_handle: u64, target_vaddr: u64) Result {
    if (target_vaddr == 0 or target_vaddr >= USER_VA_MAX) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, shm_cap_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.SharedMemory)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.ShmRead) == 0) return .{ .err = .permission_denied };

    const shm = @as(*shm_mod.ShmObject, @ptrFromInt(cap_mod.getObjectPtr(entry)));

    var pte_flags: u64 = vmm.PTE_PRESENT | vmm.PTE_USER;
    if ((entry.rights & cap_mod.Rights.ShmWrite) != 0) pte_flags |= vmm.PTE_WRITE;

    const space = vmm.AddressSpace{ .pml4_phys = current_proc.space.pml4_phys };
    var i: u64 = 0;
    while (i < shm.n_pages) : (i += 1) {
        const vaddr = target_vaddr + i * vmm.PAGE_SIZE;
        const paddr = shm.pages_phys + i * vmm.PAGE_SIZE;
        if (!vmm.map(space, vaddr, paddr, pte_flags)) {
            var j: u64 = 0;
            while (j < i) : (j += 1) vmm.unmap(space, target_vaddr + j * vmm.PAGE_SIZE);
            return .{ .err = .out_of_memory };
        }
    }
    _ = current_proc.addVma(target_vaddr, target_vaddr + shm.n_pages * vmm.PAGE_SIZE, pte_flags, false);
    return .{ .value = 0, .err = .success };
}

// ── Phase 9: Non-blocking IPC & cap_poll ────────────────────────

// cap_send_nb(port_handle, msg_ptr) → 0 or EBUSY
fn handleCapSendNb(port_handle: u64, msg_ptr: u64) Result {
    if (!validateUserPtr(msg_ptr, @sizeOf(port_mod.Message))) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, port_handle) orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Endpoint)) return .{ .err = .permission_denied };
    if ((entry.rights & cap_mod.Rights.EndpointSend) == 0) return .{ .err = .permission_denied };

    const msg = @as(*port_mod.Message, @ptrFromInt(msg_ptr));
    const port = @as(*port_mod.Port, @ptrFromInt(cap_mod.getObjectPtr(entry)));

    if (port.isFull() or port.state == .closed) return .{ .err = .would_block };

    var msg_copy = msg.*;
    const err = cap_mod.prepareMessageForSend(&current_proc.cap_table, &msg_copy);
    if (err != .success) return .{ .err = err };

    if (port.send(&msg_copy)) {
        return .{ .value = 0, .err = .success };
    }

    // send failed after caps were consumed — restore them to the sender's table
    var ri: usize = 0;
    while (ri < port_mod.MAX_CAP_TRANSFERS) : (ri += 1) {
        const tc = &msg_copy.transferred_caps[ri];
        if (tc.type == 0) continue;
        const restore_idx = tc.old_index;
        const re = &current_proc.cap_table.entries[restore_idx];
        re.* = tc.*;
        re.old_table = null;
        re.old_index = 0;
        cap_mod.linkEntry(&current_proc.cap_table, restore_idx);
        current_proc.cap_table.count += 1;
    }
    if (msg_copy.transferred_buffer_cap.type != 0) {
        const tc = &msg_copy.transferred_buffer_cap;
        const re = &current_proc.cap_table.entries[tc.old_index];
        re.* = tc.*;
        re.old_table = null;
        re.old_index = 0;
        cap_mod.linkEntry(&current_proc.cap_table, tc.old_index);
        current_proc.cap_table.count += 1;
    }
    return .{ .err = .would_block };
}

// cap_poll(handles_ptr, n_handles, timeout_ms) → ready_index (u64) or ETIMEOUT
// Scans all handles; if none ready, blocks on first Notification or Port.
fn pollScan(current_proc: anytype, handles: []const u64) ?u64 {
    for (handles, 0..) |handle, i| {
        const e = cap_mod.cap_table_lookup(&current_proc.cap_table, handle) orelse continue;
        switch (@as(cap_mod.CapType, @enumFromInt(e.type))) {
            .Endpoint => {
                const port = @as(*port_mod.Port, @ptrFromInt(cap_mod.getObjectPtr(e)));
                if (port.hasPending()) return @intCast(i);
            },
            .Notification => {
                const notif = @as(*notif_mod.Notification, @ptrFromInt(cap_mod.getObjectPtr(e)));
                if (@atomicLoad(u64, &notif.bitmask, .acquire) != 0) return @intCast(i);
            },
            else => {},
        }
    }
    return null;
}

// Multi-object wait. A thread can only be parked on one wait queue at a time
// (single Thread.next link), so instead of blocking on the first handle —
// which silently ignores activity on the others — we re-scan every handle on
// each iteration and yield the CPU between scans. This guarantees a message
// arriving on any polled port (e.g. a focus-cap registration) is noticed even
// while a notification (e.g. keyboard IRQ) has not yet fired.
fn handleCapPoll(handles_ptr: u64, n_handles: u64, timeout_ms_raw: u64) Result {
    if (n_handles == 0 or n_handles > 64) return .{ .err = .invalid_argument };
    if (!validateUserPtr(handles_ptr, n_handles * @sizeOf(u64))) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();
    const handles = @as([*]const u64, @ptrFromInt(handles_ptr))[0..@intCast(n_handles)];

    const timeout_ms: i64 = @bitCast(timeout_ms_raw);

    const cpu0 = @import("../arch/x86_64/cpu_local.zig").get_cpu_local();
    const start_ticks = @atomicLoad(u64, &cpu0.timer_ticks, .monotonic);

    while (true) {
        if (pollScan(current_proc, handles)) |idx| return .{ .value = idx, .err = .success };
        if (timeout_ms == 0) return .{ .err = .timeout };
        if (timeout_ms > 0) {
            const cpu = @import("../arch/x86_64/cpu_local.zig").get_cpu_local();
            const now = @atomicLoad(u64, &cpu.timer_ticks, .monotonic);
            // LAPIC timer runs at 100Hz → 10ms per tick.
            const elapsed_ms = (now -% start_ticks) *% 10;
            if (elapsed_ms >= @as(u64, @intCast(timeout_ms))) return .{ .err = .timeout };
        }
        // Nothing ready yet — give up the CPU and re-scan.
        sched.yield();
    }
}

// ── Phase 10: Linux Personality Syscalls ────────────────────────

fn getCurrentThread() ?*thread_mod.Thread {
    const cpu = @import("../arch/x86_64/cpu_local.zig").get_cpu_local();
    return cpu.current_thread;
}

// PersonalitySpawn(elf_mem_cap, personality_ep_cap) → linux_thread_cap
// Loads a Linux ELF, wires up personality SHM/notifications, notifies personality server.
fn handlePersonalitySpawn(elf_mem_cap_handle: u64, personality_ep_cap_handle: u64) Result {
    const current_proc = table_orig.getCurrentProcess();

    // 1. Validate + get ELF memory cap
    const elf_entry = cap_mod.cap_table_lookup(&current_proc.cap_table, elf_mem_cap_handle)
        orelse return .{ .err = .invalid_handle };
    if (elf_entry.type != @intFromEnum(cap_mod.CapType.Memory)) return .{ .err = .permission_denied };
    const elf_phys = cap_mod.getObjectPtr(elf_entry);
    const elf_data = @as([*]const u8, @ptrFromInt(vmm.phys2virt(elf_phys)))[0 .. 512 * 4096];

    // 2. Validate + get personality endpoint cap
    const ep_entry = cap_mod.cap_table_lookup(&current_proc.cap_table, personality_ep_cap_handle)
        orelse return .{ .err = .invalid_handle };
    if (ep_entry.type != @intFromEnum(cap_mod.CapType.Endpoint)) return .{ .err = .permission_denied };
    if ((ep_entry.rights & cap_mod.Rights.EndpointSend) == 0) return .{ .err = .permission_denied };

    // 3. Parse + load ELF into new process
    const elf_info = elf_mod.parse_elf64(elf_data) catch return .{ .err = .invalid_argument };
    const linux_proc = proc.create("linux_app", current_proc.pid) orelse return .{ .err = .out_of_memory };
    const user_entry = elf_mod.load_elf(elf_info, elf_data, linux_proc.space.pml4_phys)
        catch return .{ .err = .out_of_memory };

    // 4. Set up user stack (per-process bump allocator)
    const linux_stack_top = linux_proc.user_stack_top;
    linux_proc.user_stack_top -= (4 + 1) * vmm.PAGE_SIZE;
    _ = vmm.alloc_user_stack_in_space(vmm.AddressSpace{ .pml4_phys = linux_proc.space.pml4_phys }, 4, linux_stack_top)
        orelse return .{ .err = .out_of_memory };
    const argc_start = elf_mod.setupUserStack(linux_proc.space.pml4_phys, user_entry, elf_info, linux_stack_top);

    // 5. Create SHM page for syscall fast path
    const shm = shm_mod.create(1) orelse return .{ .err = .out_of_memory };
    // Map SHM into Linux process at LINUX_PERSONALITY_SHM_VIRT
    const shm_pte: u64 = vmm.PTE_PRESENT | vmm.PTE_USER | vmm.PTE_WRITE;
    const linux_space = vmm.AddressSpace{ .pml4_phys = linux_proc.space.pml4_phys };
    if (!vmm.map(linux_space, personality.LINUX_PERSONALITY_SHM_VIRT, shm.pages_phys, shm_pte))
        return .{ .err = .out_of_memory };

    // 6. Create ping + pong notifications
    const ping_notif = notif_mod.create() orelse return .{ .err = .out_of_memory };
    const pong_notif = notif_mod.create() orelse return .{ .err = .out_of_memory };

    // 7. Create the Linux thread (not yet enqueued)
    const linux_thread = @import("../sched/thread.zig").create_user(
        user_entry, argc_start, linux_proc.space.pml4_phys, linux_proc.pid,
    ) orelse return .{ .err = .out_of_memory };
    linux_thread.personality_shm_phys = shm.pages_phys;
    linux_thread.personality_ping = ping_notif;
    linux_thread.personality_pong = pong_notif;
    linux_proc.thread_count += 1;

    // 8. Build SHM cap for personality server
    const shm_cap_handle = cap_mod.cap_table_insert(
        &current_proc.cap_table, @intFromPtr(shm),
        @intFromEnum(cap_mod.CapType.SharedMemory),
        cap_mod.Rights.ShmRead | cap_mod.Rights.ShmWrite,
    ) orelse return .{ .err = .out_of_memory };

    // Build send-only ping cap
    const ping_send_handle = cap_mod.cap_table_insert(
        &current_proc.cap_table, @intFromPtr(ping_notif),
        @intFromEnum(cap_mod.CapType.Notification),
        cap_mod.Rights.NotificationSignal,
    ) orelse return .{ .err = .out_of_memory };

    // Build wait-only pong cap
    const pong_wait_handle = cap_mod.cap_table_insert(
        &current_proc.cap_table, @intFromPtr(pong_notif),
        @intFromEnum(cap_mod.CapType.Notification),
        cap_mod.Rights.NotificationWait,
    ) orelse return .{ .err = .out_of_memory };

    // Build thread cap
    const th_cap_handle = cap_mod.cap_table_insert(
        &current_proc.cap_table, @intFromPtr(linux_thread),
        @intFromEnum(cap_mod.CapType.Thread),
        cap_mod.Rights.ThreadControl | cap_mod.Rights.ThreadInspect,
    ) orelse return .{ .err = .out_of_memory };

    // 9. Send PersonalitySetup message to personality server (blocking call)
    const ep_port = @as(*port_mod.Port, @ptrFromInt(cap_mod.getObjectPtr(ep_entry)));
    var setup_msg = port_mod.Message{};
    setup_msg.msg_type = personality.MSG_PERSONALITY_SETUP;
    // payload: [0..7]=linux_pid, [8..15]=linux_thread_id, [16..23]=pserver_shm_virt (slot 0)
    const linux_pid_u64: u64 = linux_proc.pid;
    const linux_tid_u64: u64 = linux_thread.id;
    @memcpy(setup_msg.payload[0..8], @as([*]const u8, @ptrFromInt(@intFromPtr(&linux_pid_u64)))[0..8]);
    @memcpy(setup_msg.payload[8..16], @as([*]const u8, @ptrFromInt(@intFromPtr(&linux_tid_u64)))[0..8]);
    const pserver_shm_virt: u64 = personality.PSERVER_SHM_VIRT;
    @memcpy(setup_msg.payload[16..24], @as([*]const u8, @ptrFromInt(@intFromPtr(&pserver_shm_virt)))[0..8]);
    setup_msg.caps[0] = shm_cap_handle;
    setup_msg.caps[1] = ping_send_handle;
    setup_msg.caps[2] = pong_wait_handle;
    setup_msg.caps[3] = th_cap_handle;

    const setup_err = cap_mod.prepareMessageForSend(&current_proc.cap_table, &setup_msg);
    if (setup_err != .success) return .{ .err = setup_err };
    if (!ep_port.sendBlocking(&setup_msg)) {
        // Restore caps consumed by prepareMessageForSend
        var ri: usize = 0;
        while (ri < port_mod.MAX_CAP_TRANSFERS) : (ri += 1) {
            const tc = &setup_msg.transferred_caps[ri];
            if (tc.type == 0) continue;
            const re = &current_proc.cap_table.entries[tc.old_index];
            re.* = tc.*;
            re.old_table = null;
            re.old_index = 0;
            cap_mod.linkEntry(&current_proc.cap_table, tc.old_index);
            current_proc.cap_table.count += 1;
        }
        return .{ .err = .permission_denied };
    }

    // Wait for personality server to acknowledge (recv reply)
    if (ep_port.recvBlockingFiltered(true)) |*reply| {
        var rc = reply.*;
        cap_mod.receiveMessageCaps(&current_proc.cap_table, &rc);
    } else {
        return .{ .err = .permission_denied };
    }

    // 10. Now enqueue the Linux thread
    sched.enqueue(linux_thread);

    // Return thread cap handle from caller's table (re-insert since we passed it in msg)
    const ret_handle = cap_mod.cap_table_insert(
        &current_proc.cap_table, @intFromPtr(linux_thread),
        @intFromEnum(cap_mod.CapType.Thread),
        cap_mod.Rights.ThreadControl | cap_mod.Rights.ThreadInspect,
    ) orelse return .{ .err = .out_of_memory };

    return .{ .value = ret_handle, .err = .success };
}

// ProcessMmap(pid, hint_vaddr, n_pages, prot_flags) → mapped_vaddr
fn handleProcessMmap(pid_raw: u64, hint_vaddr: u64, n_pages: u64, prot_flags: u64) Result {
    if (n_pages == 0 or n_pages > 512) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();
    const target_pid: u32 = @truncate(pid_raw);
    const target_proc = proc.byPid(target_pid) orelse return .{ .err = .not_found };
    if (!callerHoldsThreadCapForPid(&current_proc.cap_table, target_pid)) return .{ .err = .permission_denied };

    const map_vaddr = if (hint_vaddr != 0) hint_vaddr else blk: {
        const v = target_proc.next_mmap_virt;
        target_proc.next_mmap_virt += n_pages * vmm.PAGE_SIZE;
        break :blk v;
    };

    var pte_flags: u64 = vmm.PTE_PRESENT | vmm.PTE_USER;
    if (prot_flags & 2 != 0) pte_flags |= vmm.PTE_WRITE; // PROT_WRITE
    if (prot_flags & 1 == 0) pte_flags |= vmm.PTE_NX;    // !PROT_EXEC

    const space = vmm.AddressSpace{ .pml4_phys = target_proc.space.pml4_phys };
    var i: u64 = 0;
    while (i < n_pages) : (i += 1) {
        const paddr = pmm.allocPage() orelse {
            // Rollback
            var j: u64 = 0;
            while (j < i) : (j += 1) vmm.unmap(space, map_vaddr + j * vmm.PAGE_SIZE);
            return .{ .err = .out_of_memory };
        };
        @memset(@as([*]u8, @ptrFromInt(vmm.phys2virt(paddr)))[0..vmm.PAGE_SIZE], 0);
        if (!vmm.map(space, map_vaddr + i * vmm.PAGE_SIZE, paddr, pte_flags)) {
            pmm.freePage(paddr);
            var j: u64 = 0;
            while (j < i) : (j += 1) vmm.unmap(space, map_vaddr + j * vmm.PAGE_SIZE);
            return .{ .err = .out_of_memory };
        }
    }
    _ = target_proc.addVma(map_vaddr, map_vaddr + n_pages * vmm.PAGE_SIZE, pte_flags, false);
    return .{ .value = map_vaddr, .err = .success };
}

// ProcessMunmap(pid, vaddr, n_pages)
fn handleProcessMunmap(pid_raw: u64, vaddr: u64, n_pages: u64) Result {
    const current_proc = table_orig.getCurrentProcess();
    const target_pid: u32 = @truncate(pid_raw);
    const target_proc = proc.byPid(target_pid) orelse return .{ .err = .not_found };
    if (!callerHoldsThreadCapForPid(&current_proc.cap_table, target_pid)) return .{ .err = .permission_denied };
    const space = vmm.AddressSpace{ .pml4_phys = target_proc.space.pml4_phys };
    var i: u64 = 0;
    while (i < n_pages) : (i += 1) {
        vmm.unmap(space, vaddr + i * vmm.PAGE_SIZE);
    }
    return .{ .value = 0, .err = .success };
}

// ThreadSetFsBase(thread_cap_handle, fs_base_addr)
fn handleThreadSetFsBase(thread_cap_handle: u64, fs_base_addr: u64) Result {
    const current_proc = table_orig.getCurrentProcess();
    const entry = cap_mod.cap_table_lookup(&current_proc.cap_table, thread_cap_handle)
        orelse return .{ .err = .invalid_handle };
    if (entry.type != @intFromEnum(cap_mod.CapType.Thread)) return .{ .err = .permission_denied };
    const t = @as(*thread_mod.Thread, @ptrFromInt(cap_mod.getObjectPtr(entry)));
    t.fs_base = fs_base_addr;
    return .{ .value = 0, .err = .success };
}

/// Read one byte from the per-IRQ kernel ring buffer.
/// Returns EBUSY (resource_busy) if the buffer is empty.
fn handleIrqReadByte(irq_cap_handle: u64) Result {
    const current_proc = table_orig.getCurrentProcess();
    const irq_entry = cap_mod.cap_table_lookup(&current_proc.cap_table, irq_cap_handle)
        orelse return .{ .err = .invalid_handle };
    if (irq_entry.type != @intFromEnum(cap_mod.CapType.DeviceIRQ)) return .{ .err = .permission_denied };
    const irq_num = cap_mod.getObjectPtr(irq_entry);
    if (irq_num >= 16) return .{ .err = .invalid_argument };
    const byte = @import("../arch/x86_64/idt.zig").irq_data_buffers[irq_num].pop()
        orelse return .{ .err = .would_block };
    return .{ .value = byte, .err = .success };
}

fn handleIrqBind(irq_cap_handle: u64, notif_cap_handle: u64) Result {
    const current_proc = table_orig.getCurrentProcess();

    // 1. Lookup DeviceIRQ capability
    const irq_entry = cap_mod.cap_table_lookup(&current_proc.cap_table, irq_cap_handle) orelse return .{ .err = .invalid_handle };
    if (irq_entry.type != @intFromEnum(cap_mod.CapType.DeviceIRQ)) return .{ .err = .permission_denied };
    if ((irq_entry.rights & cap_mod.Rights.DeviceIRQBind) == 0) return .{ .err = .permission_denied };

    // 2. Lookup Notification capability
    const notif_entry = cap_mod.cap_table_lookup(&current_proc.cap_table, notif_cap_handle) orelse return .{ .err = .invalid_handle };
    if (notif_entry.type != @intFromEnum(cap_mod.CapType.Notification)) return .{ .err = .permission_denied };
    if ((notif_entry.rights & cap_mod.Rights.NotificationSignal) == 0) return .{ .err = .permission_denied };

    const irq_num = cap_mod.getObjectPtr(irq_entry);
    const notif = @as(*notif_mod.Notification, @ptrFromInt(cap_mod.getObjectPtr(notif_entry)));

    if (irq_num >= 16) return .{ .err = .invalid_argument };

    // Bind them!
    @import("../arch/x86_64/idt.zig").irq_notification_bindings[irq_num] = notif;

    return .{ .value = 0, .err = .success };
}

