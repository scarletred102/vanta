// ============================================================================
// VantaOS — Spinlock and TicketLock (Phase 6)
// ============================================================================

const std = @import("std");

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
        const current = @atomicLoad(u16, &self.now_serving, .seq_cst);
        @atomicStore(u16, &self.now_serving, current +% 1, .seq_cst);
    }
};

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
