// ============================================================================
// VantaOS — Thread Control Block
// ============================================================================

const pmm = @import("../mm/pmm.zig");
const vmm = @import("../mm/vmm.zig");
const ctx = @import("../arch/x86_64/context.zig");

pub const KSTACK_PAGES: usize = 4; // 16 KB
pub const KSTACK_SIZE: u64 = KSTACK_PAGES * pmm.PAGE_SIZE;

pub const State = enum(u8) {
    ready,
    running,
    blocked,    // waiting on IPC / IRQ
    sleeping,   // timed sleep
    dead,
};

pub const PhysAddr = u64;

pub const Thread = struct {
    id: u32,
    state: State,
    rsp: u64,              // saved kernel rsp (during switch)
    kstack_top: u64,       // top (high) of kernel stack
    kstack_pages: u64,     // phys base of kstack
    kstack_virt: u64,      // virtual base of kstack (above guard page)
    entry: u64,            // entry function
    proc_id: u32 = 0,      // owning process (0 = kernel)
    page_table: PhysAddr,  // CR3 physical address of the page table
    next: ?*Thread = null, // run-queue link
    wake_at: u64 = 0,      // for sleeping threads (TSC ticks)
    wait_obj: u64 = 0,     // for blocked threads (object id)
};

var next_tid: u32 = 1;

// Start mapping kernel stacks starting at 0xFFFF900000000000.
// This is safely located in the shared kernel half.
var next_kstack_virt: u64 = 0xFFFF900000000000;

/// Create a kernel thread that runs `entry` on first dispatch.
pub fn create(entry: fn () callconv(.c) noreturn) ?*Thread {
    // Allocate Thread struct itself from a tiny static pool (Phase 1 simple)
    const slot = allocSlot() orelse return null;

    // Allocate kernel stack (contiguous physical pages)
    const phys = pmm.allocContiguous(KSTACK_PAGES) orelse {
        freeSlot(slot);
        return null;
    };

    // Allocate virtual range for stack (with an unmapped guard page)
    const space = vmm.AddressSpace.current();
    const stack_base_virt = next_kstack_virt + pmm.PAGE_SIZE; // Skip first page as guard page
    
    // Map each physical page into our virtual range
    var i: usize = 0;
    while (i < KSTACK_PAGES) : (i += 1) {
        const vaddr = stack_base_virt + i * pmm.PAGE_SIZE;
        const paddr = phys + i * pmm.PAGE_SIZE;
        if (!vmm.map(space, vaddr, paddr, vmm.PTE_WRITE)) {
            // Rollback mappings on failure
            var j: usize = 0;
            while (j < i) : (j += 1) {
                vmm.unmap(space, stack_base_virt + j * pmm.PAGE_SIZE);
            }
            // Free physical memory
            var p: u64 = phys;
            var k: usize = 0;
            while (k < KSTACK_PAGES) : (k += 1) {
                pmm.freePage(p);
                p += pmm.PAGE_SIZE;
            }
            freeSlot(slot);
            return null;
        }
    }

    const kstack_top = stack_base_virt + KSTACK_SIZE;
    
    // Advance virtual stack allocator (leave an extra page of padding between stacks as well)
    next_kstack_virt += (KSTACK_PAGES + 2) * pmm.PAGE_SIZE;

    slot.* = .{
        .id = next_tid,
        .state = .ready,
        .rsp = ctx.initStack(kstack_top, @intFromPtr(&entry)),
        .kstack_top = kstack_top,
        .kstack_pages = phys,
        .kstack_virt = stack_base_virt,
        .entry = @intFromPtr(&entry),
        .page_table = space.pml4_phys,
    };
    next_tid += 1;
    return slot;
}

pub fn destroy(t: *Thread) void {
    // Unmap virtual pages
    const space = vmm.AddressSpace.current();
    var i: usize = 0;
    while (i < KSTACK_PAGES) : (i += 1) {
        vmm.unmap(space, t.kstack_virt + i * pmm.PAGE_SIZE);
    }

    // Free kstack physical pages
    var p: u64 = t.kstack_pages;
    var j: usize = 0;
    while (j < KSTACK_PAGES) : (j += 1) {
        pmm.freePage(p);
        p += pmm.PAGE_SIZE;
    }
    freeSlot(t);
}

// ── Static thread pool ──────────────────────────────────────────
// 64 thread slots, enough for Phase 1 scheduler bring-up.

const MAX_THREADS: usize = 64;
var pool: [MAX_THREADS]Thread = undefined;
var pool_used: [MAX_THREADS]bool = [_]bool{false} ** MAX_THREADS;

fn allocSlot() ?*Thread {
    for (0..MAX_THREADS) |i| {
        if (!pool_used[i]) {
            pool_used[i] = true;
            return &pool[i];
        }
    }
    return null;
}

fn freeSlot(t: *Thread) void {
    const idx = (@intFromPtr(t) - @intFromPtr(&pool[0])) / @sizeOf(Thread);
    if (idx < MAX_THREADS) pool_used[idx] = false;
}
