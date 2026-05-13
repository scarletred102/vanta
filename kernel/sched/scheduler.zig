// ============================================================================
// VantaOS — Round-Robin Scheduler (Phase 1)
//
// Single-CPU, cooperative + preemptive (timer-driven).
// Ready queue is a circular singly-linked list via Thread.next.
// ============================================================================

const thread = @import("thread.zig");
const ctx = @import("../arch/x86_64/context.zig");
const serial = @import("../arch/x86_64/serial.zig");

const Thread = thread.Thread;

pub var current: ?*Thread = null;
var head: ?*Thread = null; // ready queue head
var tail: ?*Thread = null;

// ── Queue ──────────────────────────────────────────────────────

pub fn enqueue(t: *Thread) void {
    t.state = .ready;
    t.next = null;
    if (tail) |tl| {
        tl.next = t;
        tail = t;
    } else {
        head = t;
        tail = t;
    }
}

fn dequeue() ?*Thread {
    const t = head orelse return null;
    head = t.next;
    if (head == null) tail = null;
    t.next = null;
    return t;
}

// ── Scheduling ─────────────────────────────────────────────────

/// Yield: pick next ready thread, switch to it.
pub fn yield() void {
    const next_t = dequeue() orelse return; // nothing else ready

    const prev = current;
    if (prev) |p| {
        if (p.state == .running) {
            p.state = .ready;
            enqueueLocked(p);
        }
    }

    next_t.state = .running;
    current = next_t;

    if (prev) |p| {
        ctx.switch_context(&p.rsp, next_t.rsp);
    } else {
        // No prior thread — first dispatch. Need a scratch slot for old rsp.
        var scratch: u64 = 0;
        ctx.switch_context(&scratch, next_t.rsp);
    }
}

fn enqueueLocked(t: *Thread) void {
    t.next = null;
    if (tail) |tl| {
        tl.next = t;
        tail = t;
    } else {
        head = t;
        tail = t;
    }
}

/// Block current thread (caller marked state). Pick next.
pub fn block() void {
    yield();
}

/// Wake a thread: move from blocked/sleeping → ready queue.
pub fn wake(t: *Thread) void {
    if (t.state == .ready or t.state == .running) return;
    enqueue(t);
}

/// Called from timer IRQ — preempt current, schedule next.
pub fn tick() void {
    yield();
}

// ── Init ───────────────────────────────────────────────────────

pub fn init() void {
    current = null;
    head = null;
    tail = null;
    serial.puts("[SCHED] Round-robin scheduler ready\n");
}

/// Kick off scheduling — never returns. Pulls first thread from ready queue.
pub fn start() noreturn {
    const first = dequeue() orelse {
        serial.puts("[SCHED] PANIC: start() with empty ready queue\n");
        while (true) asm volatile ("hlt");
    };
    first.state = .running;
    current = first;
    var scratch: u64 = 0;
    ctx.switch_context(&scratch, first.rsp);
    while (true) asm volatile ("hlt"); // unreachable
}
