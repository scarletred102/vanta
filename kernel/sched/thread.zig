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
    personality_shm_phys: u64 = 0,     // phys addr of SHM page (0 = not a Linux thread)
    personality_ping: ?*@import("../ipc/notification.zig").Notification = null,
    personality_pong: ?*@import("../ipc/notification.zig").Notification = null,
    fs_base: u64 = 0,
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

    // Map kernel stack at dedicated VA with guard page below.
    // Layout: [next_kstack_virt + 0 = guard (non-present)] [+PAGE_SIZE..+KSTACK_SIZE = stack pages]
    const base_virt = next_kstack_virt;
    next_kstack_virt += (KSTACK_PAGES + 1) * pmm.PAGE_SIZE;
    const kern_space = vmm.AddressSpace.current();
    // Guard page — allocates intermediate tables but leaves PTE non-present
    _ = vmm.map_non_present(kern_space, base_virt);
    // Actual stack pages above the guard
    var ki: usize = 0;
    while (ki < KSTACK_PAGES) : (ki += 1) {
        _ = vmm.map(kern_space, base_virt + (ki + 1) * pmm.PAGE_SIZE,
                    phys + ki * pmm.PAGE_SIZE,
                    vmm.PTE_PRESENT | vmm.PTE_WRITE);
    }
    const kstack_top = base_virt + (KSTACK_PAGES + 1) * pmm.PAGE_SIZE;

    slot.* = .{
        .id = next_tid,
        .state = .ready,
        .rsp = ctx.initStack(kstack_top, @intFromPtr(&userThreadTrampoline)),
        .kstack_top = kstack_top,
        .kstack_pages = phys,
        .kstack_virt = base_virt + pmm.PAGE_SIZE, // first mapped stack page
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
    const slot = allocSlot() orelse return null;

    const phys = pmm.allocContiguous(KSTACK_PAGES) orelse {
        freeSlot(slot);
        return null;
    };

    // Same guard-page layout as create_user
    const base_virt = next_kstack_virt;
    next_kstack_virt += (KSTACK_PAGES + 1) * pmm.PAGE_SIZE;
    const kern_space = vmm.AddressSpace.current();
    _ = vmm.map_non_present(kern_space, base_virt);
    var ki: usize = 0;
    while (ki < KSTACK_PAGES) : (ki += 1) {
        _ = vmm.map(kern_space, base_virt + (ki + 1) * pmm.PAGE_SIZE,
                    phys + ki * pmm.PAGE_SIZE,
                    vmm.PTE_PRESENT | vmm.PTE_WRITE);
    }
    const kstack_top = base_virt + (KSTACK_PAGES + 1) * pmm.PAGE_SIZE;

    const func_ptr: *const fn () callconv(.c) noreturn = entry;
    slot.* = .{
        .id = next_tid,
        .state = .ready,
        .rsp = ctx.initStack(kstack_top, @intFromPtr(&kernelThreadTrampoline)),
        .kstack_top = kstack_top,
        .kstack_pages = phys,
        .kstack_virt = base_virt + pmm.PAGE_SIZE,
        .entry = @intFromPtr(func_ptr),
        .page_table = kern_space.pml4_phys,
    };
    next_tid += 1;
    return slot;
}

pub fn destroy(t: *Thread) void {
    const kern_space = vmm.AddressSpace.current();
    // Unmap guard page and stack pages from dedicated VA range
    vmm.unmap(kern_space, t.kstack_virt - pmm.PAGE_SIZE); // guard
    var j: usize = 0;
    while (j < KSTACK_PAGES) : (j += 1) {
        vmm.unmap(kern_space, t.kstack_virt + j * pmm.PAGE_SIZE);
    }
    // Free physical pages
    var p: u64 = t.kstack_pages;
    j = 0;
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
