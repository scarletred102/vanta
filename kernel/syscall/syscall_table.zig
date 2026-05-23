const std = @import("std");
const serial = @import("../arch/x86_64/serial.zig");
const sched = @import("../sched/scheduler.zig");
const proc = @import("../proc/process.zig");
const port_mod = @import("../ipc/port.zig");
const cap_mod = @import("../cap/handle.zig");
const table_orig = @import("table.zig");

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
    _, // Allow others without compilation error
};

// VantaOS Syscall Result
pub const Result = struct {
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
    _ = arg4; _ = arg5; _ = arg6;
    const sys_num: SyscallNumber = @enumFromInt(number);

    return switch (sys_num) {
        .CapSend => handleCapSend(arg1, arg2),
        .CapRecv => handleCapRecv(arg1, arg2),
        .CapCall => handleCapCall(arg1, arg2, arg3),
        .CapDerive => handleCapDerive(arg1, arg2, arg3),
        .CapRevoke => handleCapRevoke(arg1),
        .DebugPrint => handleDebugPrint(arg1, arg2),
        .Exit => handleExit(arg1),
        .MemMap => stubNotImpl("MemMap"),
        .MemUnmap => stubNotImpl("MemUnmap"),
        .ThreadSpawn => stubNotImpl("ThreadSpawn"),
        .CapWait => stubNotImpl("CapWait"),
        else => .{ .err = .bad_syscall },
    };
}

// Every capability-based system call validates its handle before acting!
fn handleCapSend(port_handle: u64, msg_ptr: u64) Result {
    if (msg_ptr == 0) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();
    
    // Strict validation!
    const cap = current_proc.cap_table.get(@truncate(port_handle)) orelse return .{ .err = .invalid_handle };
    if (cap.obj_type != .ipc_port) return .{ .err = .permission_denied };
    if (!cap.rights.write) return .{ .err = .permission_denied };

    const msg = @as(*const port_mod.Message, @ptrFromInt(msg_ptr));
    const port = @as(*port_mod.Port, @ptrFromInt(cap.object));

    if (port.sendBlocking(msg)) {
        return .{ .value = 0, .err = .success };
    } else {
        return .{ .err = .permission_denied };
    }
}

fn handleCapRecv(port_handle: u64, msg_ptr: u64) Result {
    if (msg_ptr == 0) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    // Strict validation!
    const cap = current_proc.cap_table.get(@truncate(port_handle)) orelse return .{ .err = .invalid_handle };
    if (cap.obj_type != .ipc_port) return .{ .err = .permission_denied };
    if (!cap.rights.read) return .{ .err = .permission_denied };

    const port = @as(*port_mod.Port, @ptrFromInt(cap.object));
    if (port.recvBlocking()) |msg| {
        const dest = @as(*port_mod.Message, @ptrFromInt(msg_ptr));
        dest.* = msg;
        return .{ .value = 0, .err = .success };
    } else {
        return .{ .err = .permission_denied };
    }
}

fn handleCapCall(port_handle: u64, msg_ptr: u64, reply_ptr: u64) Result {
    if (msg_ptr == 0 or reply_ptr == 0) return .{ .err = .invalid_argument };
    const current_proc = table_orig.getCurrentProcess();

    // Strict validation!
    const cap = current_proc.cap_table.get(@truncate(port_handle)) orelse return .{ .err = .invalid_handle };
    if (cap.obj_type != .ipc_port) return .{ .err = .permission_denied };
    if (!cap.rights.write or !cap.rights.read) return .{ .err = .permission_denied };

    const msg = @as(*const port_mod.Message, @ptrFromInt(msg_ptr));
    const port = @as(*port_mod.Port, @ptrFromInt(cap.object));

    if (!port.sendBlocking(msg)) return .{ .err = .permission_denied };
    if (port.recvBlocking()) |reply| {
        const dest = @as(*port_mod.Message, @ptrFromInt(reply_ptr));
        dest.* = reply;
        return .{ .value = 0, .err = .success };
    } else {
        return .{ .err = .permission_denied };
    }
}

fn handleCapDerive(parent_handle: u64, mask_val: u64, child_handle_ptr: u64) Result {
    const current_proc = table_orig.getCurrentProcess();

    // Strict validation!
    _ = current_proc.cap_table.get(@truncate(parent_handle)) orelse return .{ .err = .invalid_handle };

    const mask = @as(cap_mod.Rights, @bitCast(@as(u32, @truncate(mask_val))));
    const child_handle = current_proc.cap_table.derive(@truncate(parent_handle), mask) orelse return .{ .err = .out_of_memory };

    if (child_handle_ptr != 0) {
        const ptr = @as(*cap_mod.Handle, @ptrFromInt(child_handle_ptr));
        ptr.* = child_handle;
    }
    return .{ .value = child_handle, .err = .success };
}

fn handleCapRevoke(handle: u64) Result {
    const current_proc = table_orig.getCurrentProcess();

    // Strict validation!
    _ = current_proc.cap_table.get(@truncate(handle)) orelse return .{ .err = .invalid_handle };
    current_proc.cap_table.revoke(@truncate(handle));
    return .{ .value = 0, .err = .success };
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
