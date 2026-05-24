// ============================================================================
// VantaOS — Notification Kernel Object (Phase 3)
//
// A Notification wraps a machine-word bitmask.
// cap_notify: atomically ORs bits into bitmask, wakes blocked threads.
// cap_wait: blocks until (bitmask & mask) != 0, atomically clears matched bits.
// Uses atomic CAS — no spinlock.
// ============================================================================

const sched = @import("../sched/scheduler.zig");
const Thread = @import("../sched/thread.zig").Thread;
const cap = @import("../cap/handle.zig");

pub const Notification = struct {
    bitmask: u64 = 0,
    waiters: ?*Thread = null,
    cap_list: cap.CapListHead = .{},

    /// Atomically OR bits into the bitmask, wake any blocked thread.
    pub fn notify(self: *Notification, bits: u64) void {
        _ = @atomicRmw(u64, &self.bitmask, .Or, bits, .release);
        // Wake all waiters; they will re-check the mask atomically
        while (self.waiters) |w| {
            self.waiters = w.next;
            w.next = null;
            sched.wake(w);
        }
    }

    /// Block until (bitmask & mask) != 0. Atomically clears matched bits and returns them.
    /// Returns 0 immediately if bits already set (non-blocking check first).
    pub fn wait(self: *Notification, mask: u64) u64 {
        while (true) {
            // Atomic CAS loop: try to clear matching bits
            var cur = @atomicLoad(u64, &self.bitmask, .acquire);
            while (true) {
                const matched = cur & mask;
                if (matched == 0) break;
                const newval = cur & ~matched;
                const prev = @cmpxchgWeak(u64, &self.bitmask, cur, newval, .acq_rel, .acquire);
                if (prev == null) return matched;
                cur = prev.?;
            }
            // No bits set — block
            const t = @import("../arch/x86_64/cpu_local.zig").get_cpu_local().current_thread orelse return 0;
            t.state = .blocked;
            t.wait_obj = @intFromPtr(self);
            var already_in = false;
            var curr_w = self.waiters;
            while (curr_w) |w| {
                if (w == t) {
                    already_in = true;
                    break;
                }
                curr_w = w.next;
            }
            if (!already_in) {
                t.next = self.waiters;
                self.waiters = t;
            }
            sched.block();
            // Resumed — loop and re-check
        }
    }
};

// ── Static pool of Notification objects ─────────────────────────

const MAX_NOTIFICATIONS: usize = 64;
var pool: [MAX_NOTIFICATIONS]Notification = [_]Notification{.{}} ** MAX_NOTIFICATIONS;
var used: [MAX_NOTIFICATIONS]bool = [_]bool{false} ** MAX_NOTIFICATIONS;

pub fn create() ?*Notification {
    for (0..MAX_NOTIFICATIONS) |i| {
        if (!used[i]) {
            used[i] = true;
            pool[i] = .{};
            return &pool[i];
        }
    }
    return null;
}

pub fn destroy(n: *Notification) void {
    const idx = (@intFromPtr(n) - @intFromPtr(&pool[0])) / @sizeOf(Notification);
    if (idx < MAX_NOTIFICATIONS) used[idx] = false;
}
