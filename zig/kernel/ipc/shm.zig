// ============================================================================
// VantaOS — Shared Memory Kernel Object (Phase 9)
//
// ShmObject wraps a contiguous range of physical pages with a refcount.
// shm_create: allocates pages, creates ShmCap with full R/W rights.
// shm_map: maps pages into calling process's address space.
// Unmap on cap_revoke is deferred to Phase 10 cleanup.
// ============================================================================

const pmm = @import("../mm/pmm.zig");
const cap = @import("../cap/handle.zig");

pub const ShmObject = struct {
    pages_phys: u64,
    n_pages: usize,
    refcount: u32,
    cap_list: cap.CapListHead = .{},
};

const MAX_SHM: usize = 64;
var pool: [MAX_SHM]ShmObject = undefined;
var used: [MAX_SHM]bool = [_]bool{false} ** MAX_SHM;

pub fn create(n_pages: usize) ?*ShmObject {
    for (0..MAX_SHM) |i| {
        if (!used[i]) {
            const phys = pmm.allocContiguous(@intCast(n_pages)) orelse return null;
            used[i] = true;
            pool[i] = .{
                .pages_phys = phys,
                .n_pages = n_pages,
                .refcount = 1,
                .cap_list = .{},
            };
            return &pool[i];
        }
    }
    return null;
}
