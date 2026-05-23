// ============================================================================
// VantaOS — Physical Memory Manager
// Phase 1: High-Performance O(log N) Buddy Allocator
// ============================================================================

const limine = @import("../limine.zig");
const serial = @import("../arch/x86_64/serial.zig");
const vmm = @import("vmm.zig");

pub const PAGE_SIZE: u64 = 4096;

// Support up to 4 GB of physical memory (1M pages × 4KB = 4GB)
const MAX_PAGES: usize = 1024 * 1024;

// 1 byte per page frame to track allocated status and buddy block order
// Bit 7: 1 = Allocated, 0 = Free
// Bits 0-6: order (0 to 10)
var page_metadata: [MAX_PAGES]u8 = [_]u8{0x80} ** MAX_PAGES; // All allocated initially

var total_pages: usize = 0;
var free_pages: usize = 0;

// ── Buddy Allocator Structures ──────────────────────────────────

const Block = extern struct {
    next: ?*Block,
};

pub const MAX_ORDER: usize = 11; // Orders 0 to 10 (1 page to 1024 pages / 4MB)
var free_lists: [MAX_ORDER]?*Block = [_]?*Block{null} ** MAX_ORDER;

// ── Metadata Helpers ─────────────────────────────────────────────

inline fn isAllocated(page: usize) bool {
    return (page_metadata[page] & 0x80) != 0;
}

inline fn setAllocated(page: usize, val: bool) void {
    if (val) {
        page_metadata[page] |= 0x80;
    } else {
        page_metadata[page] &= 0x7F;
    }
}

inline fn getOrder(page: usize) u8 {
    return page_metadata[page] & 0x7F;
}

inline fn setOrder(page: usize, order: u8) void {
    page_metadata[page] = (page_metadata[page] & 0x80) | (order & 0x7F);
}

// ── Stats ───────────────────────────────────────────────────────

pub const MemStats = struct {
    total_pages: usize,
    free_pages: usize,
    used_pages: usize,
};

pub fn getStats() MemStats {
    return .{
        .total_pages = total_pages,
        .free_pages = free_pages,
        .used_pages = total_pages - free_pages,
    };
}

// ── Free List Operations ─────────────────────────────────────────

fn removeFreeBlock(phys: u64, order: usize) void {
    const target = @as(*Block, @ptrFromInt(vmm.phys2virt(phys)));
    var prev: ?*Block = null;
    var curr = free_lists[order];

    while (curr) |c| {
        if (c == target) {
            if (prev) |p| {
                p.next = c.next;
            } else {
                free_lists[order] = c.next;
            }
            free_pages -= @as(usize, 1) << @as(u6, @intCast(order));
            return;
        }
        prev = c;
        curr = c.next;
    }
}

// ── Core Buddy Allocator Logic ───────────────────────────────────

fn freeBlockInternal(phys: u64, order: usize) void {
    var cur_phys = phys;
    var cur_order = order;

    while (cur_order < MAX_ORDER - 1) {
        const page_idx = cur_phys / PAGE_SIZE;
        const buddy_idx = page_idx ^ (@as(usize, 1) << @as(u6, @intCast(cur_order)));

        // Check if buddy is within bounds
        if (buddy_idx >= MAX_PAGES) break;

        // Check if buddy is free and belongs to the exact same order
        if (isAllocated(buddy_idx) or getOrder(buddy_idx) != cur_order) {
            break; // Cannot merge
        }

        // Buddy is free and mergeable! Remove buddy from its free list.
        const buddy_phys = @as(u64, buddy_idx) * PAGE_SIZE;
        removeFreeBlock(buddy_phys, cur_order);

        // Merge current block and its buddy
        cur_phys = (page_idx & ~(@as(usize, 1) << @as(u6, @intCast(cur_order)))) * PAGE_SIZE;
        cur_order += 1;
    }

    // Mark final merged block as free and record its final order
    const final_idx = cur_phys / PAGE_SIZE;
    setAllocated(final_idx, false);
    setOrder(final_idx, @intCast(cur_order));
    free_pages += @as(usize, 1) << @as(u6, @intCast(cur_order));

    // Push block onto free list
    const block = @as(*Block, @ptrFromInt(vmm.phys2virt(cur_phys)));
    block.next = free_lists[cur_order];
    free_lists[cur_order] = block;
}

fn allocBlockInternal(order: usize) ?u64 {
    if (order >= MAX_ORDER) return null;

    if (free_lists[order]) |block| {
        free_lists[order] = block.next;
        const phys = vmm.virt2phys_hhdm(@intFromPtr(block));
        const idx = phys / PAGE_SIZE;
        setAllocated(idx, true);
        free_pages -= @as(usize, 1) << @as(u6, @intCast(order));
        return phys;
    }

    // No block available at this order. Scan higher orders to split.
    var next_order = order + 1;
    while (next_order < MAX_ORDER) : (next_order += 1) {
        if (free_lists[next_order]) |block| {
            free_lists[next_order] = block.next;
            const phys = vmm.virt2phys_hhdm(@intFromPtr(block));
            free_pages -= @as(usize, 1) << @as(u6, @intCast(next_order));

            // Recursively split the block
            var cur_order = next_order;
            while (cur_order > order) {
                cur_order -= 1;
                const split_size = @as(u64, 1) << @as(u6, @intCast(cur_order));
                const left_phys = phys;
                const right_phys = phys + split_size * PAGE_SIZE;

                // Mark split blocks as free with their updated order
                const left_idx = left_phys / PAGE_SIZE;
                const right_idx = right_phys / PAGE_SIZE;
                setAllocated(left_idx, false);
                setOrder(left_idx, @intCast(cur_order));
                setAllocated(right_idx, false);
                setOrder(right_idx, @intCast(cur_order));
                free_pages += split_size * 2;

                // Push right block (buddy) into its free list
                const right_block = @as(*Block, @ptrFromInt(vmm.phys2virt(right_phys)));
                right_block.next = free_lists[cur_order];
                free_lists[cur_order] = right_block;
            }

            // Allocate the final left block
            const idx = phys / PAGE_SIZE;
            setAllocated(idx, true);
            free_pages -= @as(usize, 1) << @as(u6, @intCast(order));
            return phys;
        }
    }

    return null; // Out of memory
}

// ── Range Coalescer Helper ──────────────────────────────────────

fn freePageRange(base: u64, page_count: usize) void {
    var cur = base;
    var count = page_count;

    while (count > 0) {
        var order: usize = 0;
        while (order < MAX_ORDER - 1) : (order += 1) {
            const block_size = @as(usize, 1) << @as(u6, @intCast(order + 1));
            if (block_size > count) break;
            if ((cur % (block_size * PAGE_SIZE)) != 0) break;
        }

        freeBlockInternal(cur, order);

        const size = @as(usize, 1) << @as(u6, @intCast(order));
        cur += size * PAGE_SIZE;
        count -= size;
    }
}

// ── Initialization ──────────────────────────────────────────────

pub fn init(memmap: *volatile limine.MemoryMapResponse) void {
    const entry_count: usize = @intCast(memmap.entry_count);

    serial.puts("[PMM]   Initializing Buddy Allocator...\n");

    // Initialize all pages as allocated (0x80) and of order 0
    for (0..MAX_PAGES) |i| {
        page_metadata[i] = 0x80;
    }

    // Reset list state
    for (0..MAX_ORDER) |o| {
        free_lists[o] = null;
    }

    total_pages = 0;
    free_pages = 0;

    for (0..entry_count) |i| {
        const entry = memmap.entries[i];
        const base = entry.base;
        const length = entry.length;
        const kind = entry.kind;

        // Print memory regions map
        serial.puts("          0x");
        serial.putHex(base);
        serial.puts(" - 0x");
        serial.putHex(base + length);
        serial.puts(" (");
        serial.putDec(length / 1024);
        serial.puts(" KB) ");
        serial.puts(switch (kind) {
            .usable => "USABLE",
            .reserved => "reserved",
            .acpi_reclaimable => "ACPI reclaimable",
            .acpi_nvs => "ACPI NVS",
            .bad_memory => "BAD",
            .bootloader_reclaimable => "bootloader",
            .kernel_and_modules => "kernel",
            .framebuffer => "framebuffer",
        });
        serial.puts("\n");

        if (kind == .usable) {
            var base_page = base / PAGE_SIZE;
            var count = length / PAGE_SIZE;

            // Reserve page 0 (null page) so we never allocate it
            if (base_page == 0) {
                base_page = 1;
                if (count > 0) count -= 1;
            }

            if (count > 0) {
                freePageRange(base_page * PAGE_SIZE, count);
                total_pages += count;
            }
        }
    }

    serial.puts("[PMM]   Buddy Allocator online. Total pages: ");
    serial.putDec(total_pages);
    serial.puts(" Free pages: ");
    serial.putDec(free_pages);
    serial.puts("\n");
}

// ── Public Page Allocation API ───────────────────────────────────

/// Allocate a single physical page. Returns physical address or null.
pub fn allocPage() ?u64 {
    return allocBlockInternal(0);
}

/// Free a physical page by address.
pub fn freePage(addr: u64) void {
    if (addr % PAGE_SIZE != 0) return;
    const idx = addr / PAGE_SIZE;
    if (idx >= MAX_PAGES) return;
    if (!isAllocated(idx)) return;
    const order = getOrder(idx);
    freeBlockInternal(addr, order);
}

/// Allocate a contiguous sequence of pages. Returns base physical address or null.
pub fn allocContiguous(count: usize) ?u64 {
    if (count == 0) return null;

    // Find the smallest order capable of holding the requested count
    var order: usize = 0;
    while (order < MAX_ORDER) : (order += 1) {
        const size = @as(usize, 1) << @as(u6, @intCast(order));
        if (size >= count) {
            return allocBlockInternal(order);
        }
    }
    return null;
}
