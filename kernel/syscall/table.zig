// ============================================================================
// VantaOS — Syscall Table
//
// Clean break from POSIX. 23 syscalls total.
// Every operation goes through capabilities — no ambient authority.
//
// Phase 0: Type definitions and dispatch skeleton.
// Phase 2: Full implementation when userspace exists.
// ============================================================================

const std = @import("std");
const serial = @import("../arch/x86_64/serial.zig");

// ── Syscall Numbers ─────────────────────────────────────────────
// Stable ABI: these numbers NEVER change once assigned.

pub const Syscall = enum(u64) {
    // Capability operations (0-9)
    cap_send = 0,
    cap_recv = 1,
    cap_call = 2,
    cap_derive = 3,
    cap_revoke = 4,
    cap_inspect = 5,

    // Memory operations (10-19)
    mem_create = 10,
    mem_map = 11,
    mem_unmap = 12,
    mem_share = 13,

    // Process & thread operations (20-29)
    proc_create = 20,
    thread_create = 21,
    thread_exit = 22,
    thread_yield = 23,
    thread_sleep = 24,

    // Interrupt & I/O (30-39)
    irq_create = 30,
    irq_wait = 31,
    irq_ack = 32,
    io_map = 33,

    // System (40-49)
    sys_info = 40,
    sys_log = 41,
    sys_time = 42,
    sys_shutdown = 43,

    _, // Allow unknown values without panic
};

// ── Error Codes ─────────────────────────────────────────────────
// Returned in rdx after a syscall.

pub const Error = enum(u64) {
    success = 0,
    invalid_handle = 1,
    permission_denied = 2,
    invalid_argument = 3,
    out_of_memory = 4,
    would_block = 5,
    not_found = 6,
    already_exists = 7,
    port_full = 8,
    port_empty = 9,
    bad_syscall = 10,
    interrupted = 11,
    not_implemented = 12,
};

// ── Syscall Result ──────────────────────────────────────────────
// Returned to userspace in rax (value) and rdx (error).

pub const Result = struct {
    value: u64 = 0,
    err: Error = .success,
};

// ── Dispatch ────────────────────────────────────────────────────
// Called from the syscall entry stub (assembly, Phase 2).
// Arguments come from registers: rdi, rsi, rdx, r10, r8, r9.

pub fn dispatch(
    number: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) Result {
    const syscall = std.meta.intToEnum(Syscall, number) catch {
        return .{ .err = .bad_syscall };
    };

    return switch (syscall) {
        // ── Capability ops ──
        .cap_send => stubNotImpl("cap_send"),
        .cap_recv => stubNotImpl("cap_recv"),
        .cap_call => stubNotImpl("cap_call"),
        .cap_derive => stubNotImpl("cap_derive"),
        .cap_revoke => stubNotImpl("cap_revoke"),
        .cap_inspect => stubNotImpl("cap_inspect"),

        // ── Memory ops ──
        .mem_create => stubNotImpl("mem_create"),
        .mem_map => stubNotImpl("mem_map"),
        .mem_unmap => stubNotImpl("mem_unmap"),
        .mem_share => stubNotImpl("mem_share"),

        // ── Process/thread ops ──
        .proc_create => stubNotImpl("proc_create"),
        .thread_create => stubNotImpl("thread_create"),
        .thread_exit => handleThreadExit(arg1),
        .thread_yield => handleThreadYield(),
        .thread_sleep => stubNotImpl("thread_sleep"),

        // ── IRQ/IO ops ──
        .irq_create => stubNotImpl("irq_create"),
        .irq_wait => stubNotImpl("irq_wait"),
        .irq_ack => stubNotImpl("irq_ack"),
        .io_map => stubNotImpl("io_map"),

        // ── System ops ──
        .sys_info => stubNotImpl("sys_info"),
        .sys_log => handleSysLog(arg1, arg2),
        .sys_time => handleSysTime(),
        .sys_shutdown => handleShutdown(arg1),

        else => .{ .err = .bad_syscall },
    };
}

// ── Implemented Handlers ────────────────────────────────────────

fn handleSysLog(msg_ptr: u64, msg_len: u64) Result {
    // Validate pointer and length
    if (msg_len > 4096) return .{ .err = .invalid_argument };
    if (msg_ptr == 0) return .{ .err = .invalid_argument };

    // TODO Phase 2: Validate that msg_ptr is in the calling process's
    // address space and is readable.
    const slice = @as([*]const u8, @ptrFromInt(msg_ptr))[0..@intCast(msg_len)];
    serial.puts("[LOG] ");
    serial.puts(slice);
    serial.puts("\n");

    return .{ .value = msg_len, .err = .success };
}

fn handleSysTime() Result {
    // TODO: Use HPET or TSC for actual time
    // For now, return 0
    return .{ .value = 0, .err = .success };
}

fn handleThreadExit(code: u64) Result {
    serial.puts("[SYSCALL] thread_exit(");
    serial.putDec(code);
    serial.puts(")\n");
    // TODO Phase 2: Actually terminate the thread
    while (true) {
        asm volatile ("hlt");
    }
}

fn handleThreadYield() Result {
    // TODO Phase 1: Invoke scheduler
    asm volatile ("pause");
    return .{ .err = .success };
}

fn handleShutdown(action: u64) Result {
    switch (action) {
        0 => {
            // Poweroff
            serial.puts("[SYSCALL] Shutdown requested\n");
            // QEMU shutdown via ACPI (port 0x604)
            asm volatile ("outw %[val], %[port]"
                :
                : [val] "{ax}" (@as(u16, 0x2000)),
                  [port] "{dx}" (@as(u16, 0x604)),
            );
        },
        1 => {
            // Reboot
            serial.puts("[SYSCALL] Reboot requested\n");
            // Pulse the keyboard controller reset line
            asm volatile ("outb %[val], %[port]"
                :
                : [val] "{al}" (@as(u8, 0xFE)),
                  [port] "{dx}" (@as(u16, 0x64)),
            );
        },
        else => return .{ .err = .invalid_argument },
    }
    // Should not reach here
    return .{ .err = .success };
}

// ── Stub for unimplemented syscalls ─────────────────────────────

fn stubNotImpl(name: []const u8) Result {
    serial.puts("[SYSCALL] ");
    serial.puts(name);
    serial.puts(" — not yet implemented\n");
    return .{ .err = .not_implemented };
}
