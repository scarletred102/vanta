// ============================================================================
// VantaOS Userspace — Timer Server
// ============================================================================
//
// Manages oneshot and periodic timers for other processes.
//
// Startup: init passes the LAPIC timer DeviceIRQ cap in caps[0] of the first
// received message.  The server creates its own Notification cap, binds the
// IRQ to it, then spawns a tick-relay thread that waits on the notification
// and sends a synthetic TimerTick message back to our own port so the main
// loop can handle both IRQ ticks and client requests in one place.
//
// IPC codes:
//   0x20  TimerOneshot  – payload[0..8]=delay_ns, caps[0]=notif  → timer_id
//   0x21  TimerPeriodic – payload[0..8]=period_ns, caps[0]=notif → timer_id
//   0x22  TimerCancel   – payload[0..8]=timer_id                 → OK
//   0x23  TimerTick     – internal, sent by tick-relay thread
// ============================================================================

const std = @import("std");
const libvanta = @import("../libvanta/libvanta.zig");

// ── Cap handles ──────────────────────────────────────────────────────────────

pub const IRQ_CAP_HANDLE:      u64 = 0x0001000000000001; // Slot 1, Gen 1 – LAPIC timer DeviceIRQ
pub const REGISTRY_CAP_HANDLE: u64 = 0x0001000000000002; // Slot 2, Gen 1 – registry port
pub const PORT_CAP_HANDLE:     u64 = 0x0001000000000003; // Slot 3, Gen 1 – timer listener port

// ── Message codes ─────────────────────────────────────────────────────────────

pub const MSG_TIMER_ONESHOT:  u32 = 0x20;
pub const MSG_TIMER_PERIODIC: u32 = 0x21;
pub const MSG_TIMER_CANCEL:   u32 = 0x22;
pub const MSG_TIMER_TICK:     u32 = 0x23; // internal
pub const MSG_OK:             u32 = 0x01;
pub const MSG_ERROR:          u32 = 0x03;

// ── Tick conversion ──────────────────────────────────────────────────────────
//
// LAPIC is configured for ~100 Hz → 10 ms per tick.

const NS_PER_TICK: u64 = 10_000_000; // 10 ms

fn nsToTicks(ns: u64) u64 {
    const t = ns / NS_PER_TICK;
    return if (t == 0) 1 else t;
}

// ── Timer table ──────────────────────────────────────────────────────────────

const MAX_TIMERS: usize = 64;

const TimerEntry = struct {
    id:              u64  = 0,
    notification_cap: u64 = 0,
    deadline_ticks:  u64  = 0,
    period_ticks:    u64  = 0,
    active:          bool = false,
    periodic:        bool = false,
};

var timers: [MAX_TIMERS]TimerEntry = [_]TimerEntry{.{}} ** MAX_TIMERS;
var tick_counter: u64 = 0;

// timer_id is 1-based slot index.
fn allocTimer() ?*TimerEntry {
    for (&timers) |*t| {
        if (!t.active) return t;
    }
    return null;
}

fn findTimer(id: u64) ?*TimerEntry {
    if (id == 0 or id > MAX_TIMERS) return null;
    const t = &timers[id - 1];
    if (!t.active) return null;
    return t;
}

fn timerIdOf(t: *const TimerEntry) u64 {
    const idx = (@intFromPtr(t) - @intFromPtr(&timers[0])) / @sizeOf(TimerEntry);
    return idx + 1;
}

// ── Message struct (copied from ns_server pattern) ────────────────────────────

pub const CapEntry = struct {
    type: u4             = 0,
    rights: u8           = 0,
    generation: u16      = 1,
    kernel_object_ptr: u48 = 0,
    next_derived_table:  ?*anyopaque = null,
    next_derived_index:  u16 = 0,
    parent_table:        ?*anyopaque = null,
    parent_index:        u16 = 0,
    parent_generation:   u16 = 0,
    old_table:           ?*anyopaque = null,
    old_index:           u16 = 0,
};

pub const Message = struct {
    msg_type: u32 = 0,
    flags: packed struct(u32) {
        expects_reply: bool = false,
        is_reply:      bool = false,
        has_buffer:    bool = false,
        urgent:        bool = false,
        _reserved:     u28  = 0,
    } = .{},
    payload:              [64]u8      = [_]u8{0} ** 64,
    caps:                 [4]u64      = [_]u64{0} ** 4,
    buffer_cap:           u64         = 0,
    transferred_caps:     [4]CapEntry = [_]CapEntry{.{}} ** 4,
    transferred_buffer_cap: CapEntry  = .{},
};

// ── Tick relay thread ─────────────────────────────────────────────────────────
//
// Runs in a second thread.  Blocks on the IRQ notification, then sends a
// TimerTick message to our own port so the main thread wakes up.
//
// The IRQ notification handle must be visible to this function.  We store it
// in a global written before the thread is spawned.

var irq_notif_handle: u64 = 0;

// Thread entry: the kernel sets up the stack and jumps here after spawn.
// We need it to be exported so the linker can place it, and callconv(.c) so
// the calling convention is well-defined.
export fn tickRelayThread() callconv(.c) void {
    while (true) {
        // Block until the IRQ fires.
        const wait_res = libvanta.vanta_cap_wait(irq_notif_handle, 0xFFFFFFFFFFFFFFFF);
        if (wait_res.err != 0) continue;

        // Send a synthetic tick to our own port so the main loop wakes.
        var tick_msg = Message{};
        tick_msg.msg_type = MSG_TIMER_TICK;
        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&tick_msg));
    }
}

// ── On-tick processing ────────────────────────────────────────────────────────

fn processTick() void {
    tick_counter +%= 1;

    for (&timers) |*t| {
        if (!t.active) continue;
        if (tick_counter < t.deadline_ticks) continue;

        // Fire: signal the client's notification cap.
        _ = libvanta.vanta_cap_notify(t.notification_cap, 1);

        if (t.periodic) {
            t.deadline_ticks = tick_counter + t.period_ticks;
        } else {
            t.active = false;
        }
    }
}

// ── Reply helpers ─────────────────────────────────────────────────────────────

fn sendOkReply(timer_id: u64) void {
    var reply = Message{};
    reply.msg_type = MSG_OK;
    reply.flags.is_reply = true;
    std.mem.writeInt(u64, reply.payload[0..8], timer_id, .little);
    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
}

fn sendErrorReply() void {
    var reply = Message{};
    reply.msg_type = MSG_ERROR;
    reply.flags.is_reply = true;
    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
}

// ── Spawn the tick relay thread ───────────────────────────────────────────────
//
// We allocate one page for the new thread's stack, map it, set RSP to the top
// of that page, and RIP to tickRelayThread.  The thread_spawn syscall (9)
// takes a memory cap and returns a thread handle; kernel fills in RIP/RSP via
// a follow-up message or by convention — check what the existing kernel does.
//
// In this kernel, vanta_thread_spawn takes a mem_cap for the stack and the
// thread starts executing the function whose address is the second argument.
// We pass the function pointer in the message payload after spawn.
// (See kernel thread_spawn implementation for the exact ABI.)
//
// For now we use the simplest viable approach: spawn with a stack cap and
// trust the kernel to jump to the entry recorded in the thread cap's initial
// message, which we send right after spawn.

fn spawnTickRelay() void {
    const stack_res = libvanta.vanta_mem_create(1); // 1 page = 4 KiB stack
    if (stack_res.err != 0) {
        libvanta.vanta_debug_print("timer: failed to alloc stack for tick relay");
        return;
    }

    const spawn_res = libvanta.vanta_thread_spawn(stack_res.handle);
    if (spawn_res.err != 0) {
        libvanta.vanta_debug_print("timer: failed to spawn tick relay thread");
        return;
    }

    // Send a start message to the new thread cap.
    // Convention: payload[0..8] = entry RIP, payload[8..16] = RSP top.
    // Map the stack so we know its virtual address.
    const stack_vaddr_opt = libvanta.vanta_alloc_pages(1);
    const stack_top: u64 = if (stack_vaddr_opt) |v| v + 4096 else 0;

    var start_msg = Message{};
    start_msg.msg_type = 0x01; // ThreadStart (kernel convention)
    std.mem.writeInt(u64, start_msg.payload[0..8],  @intFromPtr(&tickRelayThread), .little);
    std.mem.writeInt(u64, start_msg.payload[8..16], stack_top, .little);
    _ = libvanta.vanta_cap_send(spawn_res.handle, @intFromPtr(&start_msg));
}

// ── Main ──────────────────────────────────────────────────────────────────────

pub export fn main() void {
    libvanta.vanta_debug_print("timer: starting timer server...");

    // ── 1. Create a Notification for the IRQ ──────────────────────────────
    const notif_res = libvanta.vanta_notif_create();
    if (notif_res.err != 0) {
        libvanta.vanta_debug_print("timer: failed to create IRQ notification");
        libvanta.vanta_exit(1);
    }
    irq_notif_handle = notif_res.handle;

    // ── 2. Bind the DeviceIRQ cap (slot 1) to our Notification ───────────
    const bind_err = libvanta.vanta_irq_bind(IRQ_CAP_HANDLE, irq_notif_handle);
    if (bind_err != 0) {
        libvanta.vanta_debug_print("timer: failed to bind IRQ to notification");
        libvanta.vanta_exit(1);
    }

    // ── 4. Register with the service registry ────────────────────────────
    var derived_port: u64 = 0;
    const derive_err = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 3, @intFromPtr(&derived_port));
    if (derive_err == 0) {
        var reg_msg = Message{};
        reg_msg.msg_type = 0x10; // RegistryRegister
        @memcpy(reg_msg.payload[0..9], "sys.timer");
        reg_msg.caps[0] = derived_port;
        _ = libvanta.vanta_cap_send(REGISTRY_CAP_HANDLE, @intFromPtr(&reg_msg));
    }
    libvanta.vanta_debug_print("timer: registered as sys.timer");

    // ── 5. Spawn the tick-relay thread ────────────────────────────────────
    spawnTickRelay();
    libvanta.vanta_debug_print("timer: tick relay thread spawned");

    // ── 6. Main message loop ──────────────────────────────────────────────
    libvanta.vanta_debug_print("timer: entering main loop");
    while (true) {
        var msg = Message{};
        const err = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&msg));
        if (err != 0) continue;

        switch (msg.msg_type) {
            MSG_TIMER_TICK => {
                processTick();
            },

            MSG_TIMER_ONESHOT => {
                const delay_ns = std.mem.readInt(u64, msg.payload[0..8], .little);
                const notif_cap = msg.caps[0];

                if (notif_cap == 0) {
                    if (msg.flags.expects_reply) sendErrorReply();
                    continue;
                }

                const entry = allocTimer() orelse {
                    if (msg.flags.expects_reply) sendErrorReply();
                    continue;
                };

                const ticks = nsToTicks(delay_ns);
                entry.* = .{
                    .id              = timerIdOf(entry),
                    .notification_cap = notif_cap,
                    .deadline_ticks  = tick_counter + ticks,
                    .period_ticks    = 0,
                    .active          = true,
                    .periodic        = false,
                };

                if (msg.flags.expects_reply) sendOkReply(entry.id);
            },

            MSG_TIMER_PERIODIC => {
                const period_ns = std.mem.readInt(u64, msg.payload[0..8], .little);
                const notif_cap = msg.caps[0];

                if (notif_cap == 0) {
                    if (msg.flags.expects_reply) sendErrorReply();
                    continue;
                }

                const entry = allocTimer() orelse {
                    if (msg.flags.expects_reply) sendErrorReply();
                    continue;
                };

                const ticks = nsToTicks(period_ns);
                entry.* = .{
                    .id              = timerIdOf(entry),
                    .notification_cap = notif_cap,
                    .deadline_ticks  = tick_counter + ticks,
                    .period_ticks    = ticks,
                    .active          = true,
                    .periodic        = true,
                };

                if (msg.flags.expects_reply) sendOkReply(entry.id);
            },

            MSG_TIMER_CANCEL => {
                const timer_id = std.mem.readInt(u64, msg.payload[0..8], .little);

                if (findTimer(timer_id)) |t| {
                    t.active = false;
                    if (msg.flags.expects_reply) sendOkReply(0);
                } else {
                    if (msg.flags.expects_reply) sendErrorReply();
                }
            },

            else => {
                // Unknown message — ignore (no reply even if expected).
            },
        }
    }
}
