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
    cap_list: @import("../cap/handle.zig").CapListHead = .{},
    user_entry: u64 = 0,
    user_stack: u64 = 0,
    yielded: bool = true,
    user_rsp_scratch: u64 = 0,
};

var next_tid: u32 = 1;

// Start mapping kernel stacks starting at 0xFFFF900000000000.
// This is safely located in the shared kernel half.
var next_kstack_virt: u64 = 0xFFFF900000000000;

fn userThreadTrampoline() callconv(.c) noreturn {
    const cpu = @import("../arch/x86_64/cpu_local.zig").get_cpu_local();
    if (cpu.prev_thread) |p| {
        @atomicStore(bool, &p.yielded, true, .release);
        cpu.prev_thread = null;
    }
    const current_t = cpu.current_thread.?;
    @import("../arch/x86_64/syscall.zig").enter_userspace(current_t.user_entry, current_t.user_stack, 0);
}

fn kernelThreadTrampoline() callconv(.c) noreturn {
    const cpu = @import("../arch/x86_64/cpu_local.zig").get_cpu_local();
    if (cpu.prev_thread) |p| {
        @atomicStore(bool, &p.yielded, true, .release);
        cpu.prev_thread = null;
    }
    const current_t = cpu.current_thread.?;
    const entry_fn = @as(*const fn () callconv(.c) noreturn, @ptrFromInt(current_t.entry));
    entry_fn();
}

pub fn create_user(entry_addr: u64, stack_addr: u64, page_table: PhysAddr, proc_id: u32) ?*Thread {
    const slot = allocSlot() orelse return null;

    // Allocate kernel stack (contiguous physical pages)
    const phys = pmm.allocContiguous(KSTACK_PAGES) orelse {
        freeSlot(slot);
        return null;
    };

    const stack_base_virt = vmm.phys2virt(phys);
    const kstack_top = stack_base_virt + KSTACK_SIZE;

    slot.* = .{
        .id = next_tid,
        .state = .ready,
        .rsp = ctx.initStack(kstack_top, @intFromPtr(&userThreadTrampoline)),
        .kstack_top = kstack_top,
        .kstack_pages = phys,
        .kstack_virt = stack_base_virt,
        .entry = @intFromPtr(&userThreadTrampoline),
        .proc_id = proc_id,
        .page_table = page_table,
        .user_entry = entry_addr,
        .user_stack = stack_addr,
    };
    next_tid += 1;
    return slot;
}

/// Create a kernel thread that runs `entry` on first dispatch.
pub fn create(entry: fn () callconv(.c) noreturn) ?*Thread {
    // Allocate Thread struct itself from a tiny static pool (Phase 1 simple)
    const slot = allocSlot() orelse return null;

    // Allocate kernel stack (contiguous physical pages)
    const phys = pmm.allocContiguous(KSTACK_PAGES) orelse {
        freeSlot(slot);
        return null;
    };

    const stack_base_virt = vmm.phys2virt(phys);
    const kstack_top = stack_base_virt + KSTACK_SIZE;

    const func_ptr: *const fn () callconv(.c) noreturn = entry;
    slot.* = .{
        .id = next_tid,
        .state = .ready,
        .rsp = ctx.initStack(kstack_top, @intFromPtr(&kernelThreadTrampoline)),
        .kstack_top = kstack_top,
        .kstack_pages = phys,
        .kstack_virt = stack_base_virt,
        .entry = @intFromPtr(func_ptr),
        .page_table = vmm.AddressSpace.current().pml4_phys,
    };
    next_tid += 1;
    return slot;
}

pub fn destroy(t: *Thread) void {
    // Free kstack physical pages
    var p: u64 = t.kstack_pages;
    var j: usize = 0;
    while (j < KSTACK_PAGES) : (j += 1) {
        pmm.freePage(p);
        p += pmm.PAGE_SIZE;
    }
    freeSlot(t);
}

const slab = @import("../mm/slab.zig");

fn allocSlot() ?*Thread {
    return slab.alloc_thread();
}

fn freeSlot(t: *Thread) void {
    slab.free_thread(t);
}
