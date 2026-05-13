// ============================================================================
// VantaOS — Process Control Block
// ============================================================================

const vmm = @import("../mm/vmm.zig");
const cap = @import("../cap/handle.zig");

pub const Process = struct {
    pid: u32,
    name: [16]u8 = [_]u8{0} ** 16,
    space: vmm.AddressSpace,
    cap_table: cap.CapabilityTable = .{},
    thread_count: u32 = 0,
    parent_pid: u32 = 0,
};

const MAX_PROCS: usize = 32;
var pool: [MAX_PROCS]Process = undefined;
var used: [MAX_PROCS]bool = [_]bool{false} ** MAX_PROCS;
var next_pid: u32 = 1;

pub fn create(name: []const u8, parent_pid: u32) ?*Process {
    for (0..MAX_PROCS) |i| {
        if (!used[i]) {
            used[i] = true;
            const space = vmm.createAddressSpace() orelse {
                used[i] = false;
                return null;
            };
            pool[i] = .{
                .pid = next_pid,
                .space = space,
                .parent_pid = parent_pid,
            };
            const n = @min(name.len, 15);
            @memcpy(pool[i].name[0..n], name[0..n]);
            next_pid += 1;
            return &pool[i];
        }
    }
    return null;
}

pub fn destroy(p: *Process) void {
    const idx = (@intFromPtr(p) - @intFromPtr(&pool[0])) / @sizeOf(Process);
    if (idx < MAX_PROCS) used[idx] = false;
    // TODO: free page tables (walk and PMM-free)
}

pub fn byPid(pid: u32) ?*Process {
    for (0..MAX_PROCS) |i| {
        if (used[i] and pool[i].pid == pid) return &pool[i];
    }
    return null;
}

// ── Kernel process (pid 0) ─────────────────────────────────────
// All early kernel threads belong to this synthetic process.

pub var kernel_proc: Process = undefined;

pub fn initKernelProc() void {
    kernel_proc = .{
        .pid = 0,
        .space = vmm.AddressSpace.current(),
    };
    @memcpy(kernel_proc.name[0..6], "kernel");
}
