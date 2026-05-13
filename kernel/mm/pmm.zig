// ============================================================================
// VantaOS — Physical Memory Manager
// Phase 0: Simple bitmap allocator.
// Phase 1 TODO: Buddy allocator for O(log n) alloc and contiguous ranges.
// ============================================================================

const limine = @import("../limine.zig");
const serial = @import("../arch/x86_64/serial.zig");

pub const PAGE_SIZE: u64 = 4096;

// Support up to 4 GB of physical memory (1M pages × 4KB = 4GB)
// The bitmap lives in BSS — 128 KB, no runtime allocation needed.
const MAX_PAGES: usize = 1024 * 1024;

// Bitmap: 1 = used/reserved, 0 = free
var bitmap: [MAX_PAGES / 8]u8 = [_]u8{0xFF} ** (MAX_PAGES / 8); // All used initially

var total_pages: usize = 0;
var free_pages: usize = 0;

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

// ── Initialization ──────────────────────────────────────────────

pub fn init(memmap: *volatile limine.MemoryMapResponse) void {
    const entry_count: usize = @intCast(memmap.entry_count);

    serial.puts("[PMM]   Memory map:\n");

    for (0..entry_count) |i| {
        const entry = memmap.entries[i];
        const base = entry.base;
        const length = entry.length;
        const kind = entry.kind;

        // Print each region
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

        // Only mark usable regions as free
        if (kind == .usable) {
            const start_page = base / PAGE_SIZE;
            const page_count = length / PAGE_SIZE;

            var p: usize = 0;
            while (p < page_count) : (p += 1) {
                const page: usize = @intCast(start_page + p);
                if (page < MAX_PAGES) {
                    clearBit(page);
                    total_pages += 1;
                    free_pages += 1;
                }
            }
        }
    }

    // Reserve page 0 (null page — never allocate this)
    if (total_pages > 0) {
        setBit(0);
        if (free_pages > 0) free_pages -= 1;
    }
}

// ── Page Allocation ─────────────────────────────────────────────

/// Allocate a single physical page. Returns the physical address or null.
pub fn allocPage() ?u64 {
    // Linear scan — slow but correct. Phase 1 replaces with buddy.
    for (0..MAX_PAGES / 8) |byte_idx| {
        if (bitmap[byte_idx] != 0xFF) {
            // This byte has at least one free bit
            for (0..8) |bit_idx| {
                const page = byte_idx * 8 + bit_idx;
                if (!testBit(page)) {
                    setBit(page);
                    free_pages -= 1;
                    return @as(u64, @intCast(page)) * PAGE_SIZE;
                }
            }
        }
    }
    return null;
}

/// Free a physical page by address.
pub fn freePage(addr: u64) void {
    const page: usize = @intCast(addr / PAGE_SIZE);
    if (page >= MAX_PAGES) return;

    if (testBit(page)) {
        clearBit(page);
        free_pages += 1;
    }
}

/// Allocate `count` contiguous physical pages.
/// Returns the physical address of the first page, or null.
pub fn allocContiguous(count: usize) ?u64 {
    if (count == 0) return null;
    if (count == 1) return allocPage();

    var run_start: usize = 0;
    var run_len: usize = 0;

    for (0..MAX_PAGES) |page| {
        if (!testBit(page)) {
            if (run_len == 0) run_start = page;
            run_len += 1;
            if (run_len == count) {
                // Found enough contiguous pages — mark them all used
                for (run_start..run_start + count) |p| {
                    setBit(p);
                }
                free_pages -= count;
                return @as(u64, @intCast(run_start)) * PAGE_SIZE;
            }
        } else {
            run_len = 0;
        }
    }
    return null;
}

// ── Bitmap Operations ───────────────────────────────────────────

fn setBit(page: usize) void {
    bitmap[page / 8] |= @as(u8, 1) << @as(u3, @intCast(page % 8));
}

fn clearBit(page: usize) void {
    bitmap[page / 8] &= ~(@as(u8, 1) << @as(u3, @intCast(page % 8)));
}

fn testBit(page: usize) bool {
    return (bitmap[page / 8] & (@as(u8, 1) << @as(u3, @intCast(page % 8)))) != 0;
}
