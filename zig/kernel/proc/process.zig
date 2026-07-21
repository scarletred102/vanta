// ============================================================================
// VantaOS — Process Control Block
// ============================================================================

const vmm = @import("../mm/vmm.zig");
const cap = @import("../cap/handle.zig");

pub const Vma = struct {
    start: u64,
    end: u64,
    flags: u64,
    lazy: bool = false,
    cow: bool = false,
};

pub const Process = struct {
    pid: u32,
    name: [16]u8 = [_]u8{0} ** 16,
    space: vmm.AddressSpace,
    cap_table: cap.CapTable = .{},
    thread_count: u32 = 0,
    parent_pid: u32 = 0,
    vmas: [16]Vma = [_]Vma{.{ .start = 0, .end = 0, .flags = 0, .lazy = false, .cow = false }} ** 16,
    vma_count: usize = 0,
    next_mmap_virt: u64 = 0x2000_0000,
    user_stack_top: u64 = 0x7FFF_0000_0000,

    pub fn addVma(self: *Process, start: u64, end: u64, flags: u64, lazy: bool) bool {
        if (self.vma_count >= 16) return false;
        self.vmas[self.vma_count] = .{
            .start = start,
            .end = end,
            .flags = flags,
            .lazy = lazy,
        };
        self.vma_count += 1;
        return true;
    }

    pub fn findVma(self: *const Process, addr: u64) ?*const Vma {
        var i: usize = 0;
        while (i < self.vma_count) : (i += 1) {
            const vma = &self.vmas[i];
            if (addr >= vma.start and addr < vma.end) {
                return vma;
            }
        }
        return null;
    }
};

const MAX_PROCS: usize = 32;
var pool: [MAX_PROCS]Process = [_]Process{.{
    .pid = 0,
    .space = .{ .pml4_phys = 0 },
}} ** MAX_PROCS;
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
            pool[i].pid = next_pid;
            pool[i].space = space;
            pool[i].parent_pid = parent_pid;
            pool[i].thread_count = 0;
            pool[i].vma_count = 0;
            pool[i].next_mmap_virt = 0x2000_0000;
            pool[i].user_stack_top = 0x7FFF_0000_0000;
            pool[i].cap_table.count = 0;
            var c: usize = 0;
            while (c < cap.MAX_CAPS) : (c += 1) {
                pool[i].cap_table.entries[c] = .{
                    .type = 0,
                    .rights = 0,
                    .generation = 1,
                    .kernel_object_ptr = 0,
                };
            }
            const n = @min(name.len, 15);
            @memcpy(pool[i].name[0..n], name[0..n]);
            next_pid += 1;
            return &pool[i];
        }
    }
    return null;
}

pub fn destroy(p: *Process) void {
    var i: usize = 1;
    while (i < cap.MAX_CAPS) : (i += 1) {
        if (p.cap_table.entries[i].type != 0) {
            cap.cap_revoke(&p.cap_table, cap.encodeHandle(@intCast(i), p.cap_table.entries[i].generation));
        }
    }
    if (p.space.pml4_phys != 0) {
        vmm.freeAddressSpacePages(p.space.pml4_phys);
    }
    const idx = (@intFromPtr(p) - @intFromPtr(&pool[0])) / @sizeOf(Process);
    if (idx < MAX_PROCS) used[idx] = false;
}

pub fn byPid(pid: u32) ?*Process {
    for (0..MAX_PROCS) |i| {
        if (used[i] and pool[i].pid == pid) return &pool[i];
    }
    return null;
}

// ── Kernel process (pid 0) ─────────────────────────────────────
// All early kernel threads belong to this synthetic process.

pub var kernel_proc: Process = .{
    .pid = 0,
    .space = .{ .pml4_phys = 0 },
};

pub fn initKernelProc() void {
    kernel_proc.space = vmm.AddressSpace.current();
    @memcpy(kernel_proc.name[0..6], "kernel");
}
