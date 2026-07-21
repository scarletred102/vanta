// ============================================================================
// VantaOS — Per-CPU Local Storage (Phase 6)
// ============================================================================

const std = @import("std");
const Thread = @import("../../sched/thread.zig").Thread;
const tss_mod = @import("tss.zig");

// Run queue definition (Phase 6)
pub const RunQueue = struct {
    head: ?*Thread = null,
    tail: ?*Thread = null,
    lock: TicketLock = .{},
    length: u32 = 0,
};

pub const TicketLock = struct {
    next_ticket: u16 = 0,
    now_serving: u16 = 0,

    pub fn lock(self: *TicketLock) void {
        const my_ticket = @atomicRmw(u16, &self.next_ticket, .Add, 1, .seq_cst);
        while (@atomicLoad(u16, &self.now_serving, .seq_cst) != my_ticket) {
            asm volatile ("pause");
        }
    }

    pub fn unlock(self: *TicketLock) void {
        const cur = @atomicLoad(u16, &self.now_serving, .seq_cst);
        @atomicStore(u16, &self.now_serving, cur +% 1, .seq_cst);
    }

    pub fn lock_irqsave(self: *TicketLock) u64 {
        var flags: u64 = 0;
        asm volatile (
            \\ pushfq
            \\ popq %[flags]
            \\ cli
            : [flags] "=r" (flags),
            :
            : .{ .memory = true }
        );
        self.lock();
        return flags;
    }

    pub fn unlock_irqrestore(self: *TicketLock, flags: u64) void {
        self.unlock();
        asm volatile (
            \\ pushq %[flags]
            \\ popfq
            :
            : [flags] "r" (flags),
            : .{ .memory = true }
        );
    }
};

pub const CpuLocal = struct {
    // Offset 0: self pointer (essential for %gs:0 to work)
    self_ptr: u64 = 0,
    // Offset 8: kernel RSP for syscall entry
    kernel_rsp: u64 = 0,
    // Offset 16: scratch space for user RSP
    user_rsp_scratch: u64 = 0,

    cpu_id: u8 = 0,
    apic_id: u8 = 0,
    lapic_base: u64 = 0,
    current_thread: ?*Thread = null,
    idle_thread: ?*Thread = null,
    
    // Per-CPU runqueue
    run_queue: RunQueue = .{},

    thread_to_reap: ?*Thread = null,

    // Slab magazines for Thread and CapEntry (Phase 5 magazines)
    thread_magazine: [64]u64 = [_]u64{0} ** 64,
    thread_mag_count: u32 = 0,
    cap_magazine: [64]u64 = [_]u64{0} ** 64,
    cap_mag_count: u32 = 0,

    timer_ticks: u64 = 0,
    watchdog_last_ticks: u64 = 0,
    watchdog_miss_count: u32 = 0,
    tss_ptr: ?*tss_mod.Tss = null,
    prev_thread: ?*Thread = null,
};

pub var cpus: [64]CpuLocal = [_]CpuLocal{.{}} ** 64;
pub var cpu_count: u32 = 1;

pub inline fn get_cpu_local() *CpuLocal {
    var val: u64 = undefined;
    asm volatile ("movq %%gs:0, %[val]" : [val] "=r" (val));
    return @ptrFromInt(val);
}
