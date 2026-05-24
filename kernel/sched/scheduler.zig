// ============================================================================
// VantaOS — SMP-Safe Scheduler (Phase 6)
//
// Per-CPU run queues (TicketLock protected).
// Work stealing on empty local queue.
// Context switch via context.zig switch_context.
// ============================================================================

const thread = @import("thread.zig");
const ctx = @import("../arch/x86_64/context.zig");
const serial = @import("../arch/x86_64/serial.zig");
const cpu_local = @import("../arch/x86_64/cpu_local.zig");

const Thread = thread.Thread;

// Global current pointer kept for single-CPU backward compat (BSP only).
// SMP-safe code should use get_cpu_local().current_thread.
pub var current: ?*Thread = null;

// Reaping is now fully CPU-local

// ── Per-CPU Enqueue / Dequeue ──────────────────────────────────

fn enqueueLocal(q: *cpu_local.RunQueue, t: *Thread) void {
    t.state = .ready;
    t.next = null;
    const flags = q.lock.lock_irqsave();
    if (q.tail) |tl| {
        tl.next = t;
        q.tail = t;
    } else {
        q.head = t;
        q.tail = t;
    }
    q.length += 1;
    q.lock.unlock_irqrestore(flags);
}

fn dequeueLocal(q: *cpu_local.RunQueue) ?*Thread {
    const flags = q.lock.lock_irqsave();
    const t = q.head orelse {
        q.lock.unlock_irqrestore(flags);
        return null;
    };
    q.head = t.next;
    if (q.head == null) q.tail = null;
    t.next = null;
    q.length -= 1;
    q.lock.unlock_irqrestore(flags);
    return t;
}

// Work steal: take half of largest other CPU's queue.
fn stealWork(my_q: *cpu_local.RunQueue) ?*Thread {
    const count = cpu_local.cpu_count;
    var best_cpu: usize = 0;
    var best_len: u32 = 0;

    // Find most loaded CPU (atomic load of length, no lock needed for probe)
    var i: usize = 0;
    while (i < count) : (i += 1) {
        const q = &cpu_local.cpus[i].run_queue;
        if (q == my_q) continue;
        const len = @atomicLoad(u32, &q.length, .monotonic);
        if (len > best_len) {
            best_len = len;
            best_cpu = i;
        }
    }

    if (best_len < 2) return null; // nothing worth stealing

    const src_q = &cpu_local.cpus[best_cpu].run_queue;
    const steal_count = best_len / 2;

    const flags = src_q.lock.lock_irqsave();

    // Re-check under lock
    if (src_q.length < 2) {
        src_q.lock.unlock_irqrestore(flags);
        return null;
    }

    // Pull steal_count threads from src head
    var first: ?*Thread = null;
    var last: ?*Thread = null;
    var stolen: u32 = 0;
    while (stolen < steal_count) : (stolen += 1) {
        const t = src_q.head orelse break;
        src_q.head = t.next;
        if (src_q.head == null) src_q.tail = null;
        src_q.length -= 1;
        t.next = null;
        if (first == null) {
            first = t;
            last = t;
        } else {
            last.?.next = t;
            last = t;
        }
    }

    // Add all but the first stolen thread into local queue (already holding src lock,
    // can't safely take my_q lock too — add via unlocked path, safe since we're on this CPU)
    if (first) |f| {
        var rest = f.next;
        f.next = null;
        while (rest) |r| {
            const nxt = r.next;
            r.next = null;
            r.state = .ready;
            // Append directly without lock (single-CPU local queue mutation)
            const lflags = my_q.lock.lock_irqsave();
            if (my_q.tail) |tl| { tl.next = r; my_q.tail = r; } else { my_q.head = r; my_q.tail = r; }
            my_q.length += 1;
            my_q.lock.unlock_irqrestore(lflags);
            rest = nxt;
        }
        src_q.lock.unlock_irqrestore(flags);
        return f;
    }
    src_q.lock.unlock_irqrestore(flags);
    return null;
}

// ── Public Queue Interface ─────────────────────────────────────

pub fn enqueue(t: *Thread) void {
    // Enqueue to least-loaded CPU
    const count = cpu_local.cpu_count;
    var best: usize = 0;
    var best_len: u32 = 0xFFFF_FFFF;
    var i: usize = 0;
    while (i < count) : (i += 1) {
        const len = @atomicLoad(u32, &cpu_local.cpus[i].run_queue.length, .monotonic);
        if (len < best_len) { best_len = len; best = i; }
    }
    enqueueLocal(&cpu_local.cpus[best].run_queue, t);
}

fn dequeueNext() ?*Thread {
    const cpu = cpu_local.get_cpu_local();
    if (dequeueLocal(&cpu.run_queue)) |t| return t;
    return stealWork(&cpu.run_queue);
}

// ── Scheduling ─────────────────────────────────────────────────

pub fn yield() void {
    const cpu = cpu_local.get_cpu_local();
    if (cpu.thread_to_reap) |t| {
        thread.destroy(t);
        cpu.thread_to_reap = null;
    }

    const next_t = dequeueNext() orelse return;

    const prev = cpu.current_thread;
    if (prev) |p| {
        if (p.state == .running) {
            p.state = .ready;
            enqueueLocal(&cpu.run_queue, p);
        }
    }

    next_t.state = .running;
    cpu.current_thread = next_t;
    current = next_t;

    if (prev) |p| {
        if (p.page_table != next_t.page_table) {
            @import("../mm/vmm.zig").writeCr3(next_t.page_table);
        }
    } else {
        @import("../mm/vmm.zig").writeCr3(next_t.page_table);
    }

    cpu.tss_ptr.?.rsp0 = next_t.kstack_top;
    @import("../arch/x86_64/syscall.zig").setCpuKernelRsp(next_t.kstack_top);

    if (prev) |p| {
        @atomicStore(bool, &p.yielded, false, .release);
        cpu.prev_thread = p;
        ctx.switch_context(&p.rsp, next_t.rsp);
    } else {
        cpu.prev_thread = null;
        var scratch: u64 = 0;
        ctx.switch_context(&scratch, next_t.rsp);
    }

    if (cpu.prev_thread) |p| {
        @atomicStore(bool, &p.yielded, true, .release);
        cpu.prev_thread = null;
    }
}

pub fn block() void {
    yield();
}

pub fn wake(t: *Thread) void {
    if (t.state == .ready or t.state == .running) return;
    while (!@atomicLoad(bool, &t.yielded, .acquire)) {
        asm volatile ("pause");
    }
    enqueue(t);
}

pub fn tick() void {
    yield();
}

// ── Init ───────────────────────────────────────────────────────

pub fn init() void {
    current = null;
    serial.puts("[SCHED] Per-CPU scheduler ready\n");
}

pub fn start() noreturn {
    const cpu = cpu_local.get_cpu_local();
    if (cpu.thread_to_reap) |t| {
        thread.destroy(t);
        cpu.thread_to_reap = null;
    }

    const first = dequeueNext() orelse {
        serial.puts("[SCHED] PANIC: start() with empty ready queue\n");
        while (true) asm volatile ("hlt");
    };

    first.state = .running;
    cpu.current_thread = first;
    current = first;

    @import("../mm/vmm.zig").writeCr3(first.page_table);
    cpu.tss_ptr.?.rsp0 = first.kstack_top;
    @import("../arch/x86_64/syscall.zig").setCpuKernelRsp(first.kstack_top);

    var scratch: u64 = 0;
    ctx.switch_context(&scratch, first.rsp);
    while (true) asm volatile ("hlt");
}

pub fn exitCurrentThread() noreturn {
    const cpu = cpu_local.get_cpu_local();
    if (cpu.thread_to_reap) |t| {
        thread.destroy(t);
        cpu.thread_to_reap = null;
    }

    if (cpu.current_thread) |c| {
        c.state = .dead;
        if (c.proc_id != 0) {
            if (@import("../proc/process.zig").byPid(c.proc_id)) |p| {
                p.thread_count -= 1;
                if (p.thread_count == 0) {
                    serial.puts("[PROC] Last thread exited, destroying process\n");
                    @import("../proc/process.zig").destroy(p);
                }
            }
        }
        cpu.thread_to_reap = c;
    }

    const next_t = dequeueNext() orelse {
        serial.puts("[SCHED] No more threads. Halting.\n");
        asm volatile ("outw %[val], %[port]"
            :
            : [val] "{ax}" (@as(u16, 0x2000)),
              [port] "{dx}" (@as(u16, 0x604)),
        );
        while (true) asm volatile ("cli; hlt");
    };

    next_t.state = .running;
    cpu.current_thread = next_t;
    current = next_t;

    @import("../mm/vmm.zig").writeCr3(next_t.page_table);
    cpu.tss_ptr.?.rsp0 = next_t.kstack_top;
    @import("../arch/x86_64/syscall.zig").setCpuKernelRsp(next_t.kstack_top);

    cpu.prev_thread = null;
    var scratch: u64 = 0;
    ctx.switch_context(&scratch, next_t.rsp);
    while (true) asm volatile ("hlt");
}
