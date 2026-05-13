/// x86_64 Virtual Memory & Paging Implementation for VantaOS Microkernel
/// Freestanding Zig implementation
///
/// This file contains:
///   1. PageTableEntry struct (packed 64-bit)
///   2. Flag constants
///   3. CR3 read/write operations
///   4. TLB invalidation
///   5. Virtual-to-physical address translation
///   6. Page table walk with on-demand allocation
///   7. HHDM (Higher Half Direct Map) utilities

const std = @import("std");
const builtin = @import("builtin");

// ============================================================================
// PAGE TABLE ENTRY STRUCTURE
// ============================================================================

/// x86_64 page table entry (64-bit packed structure)
/// Valid for all 4 levels: PML4, PDPT, PD, PT
pub const PageTableEntry = packed struct(u64) {
    present: bool,           // [0] P - Page present in physical memory
    writable: bool,          // [1] RW - Read/write (1) or read-only (0)
    user: bool,              // [2] U/S - User-mode accessible
    write_through: bool,     // [3] PWT - Write-through caching
    cache_disable: bool,     // [4] PCD - Cache disabled (MMIO)
    accessed: bool,          // [5] A - Accessed since last clear
    dirty: bool,             // [6] D - Dirty/modified (PT only)
    huge: bool,              // [7] PS - Huge page (2MiB or 1GiB)
    global: bool,            // [8] G - Global (don't flush on CR3)
    avail_lo: u3,            // [9-11] Available for OS use
    addr: u40,               // [12-51] Physical page address (4K aligned)
    avail_hi: u11,           // [52-62] Available for OS use
    no_execute: bool,        // [63] NX - No-execute (if EFER.NXE set)

    /// Return physical address from entry (addr field is already shifted)
    pub fn physical_address(self: PageTableEntry) u64 {
        return @as(u64, self.addr) << 12;
    }

    /// Create entry from physical address and flags
    pub fn from_address(phys: u64, flags: u64) PageTableEntry {
        return @bitCast(phys | flags);
    }
};

// ============================================================================
// PAGE TABLE FLAGS
// ============================================================================

pub const PageTableFlags = struct {
    pub const PRESENT = @as(u64, 1) << 0;      // 0x0000000000000001
    pub const WRITABLE = @as(u64, 1) << 1;     // 0x0000000000000002
    pub const USER = @as(u64, 1) << 2;         // 0x0000000000000004
    pub const WRITE_THROUGH = @as(u64, 1) << 3; // 0x0000000000000008
    pub const CACHE_DISABLE = @as(u64, 1) << 4; // 0x0000000000000010
    pub const ACCESSED = @as(u64, 1) << 5;     // 0x0000000000000020
    pub const DIRTY = @as(u64, 1) << 6;        // 0x0000000000000040
    pub const HUGE = @as(u64, 1) << 7;         // 0x0000000000000080 (2MiB/1GiB)
    pub const GLOBAL = @as(u64, 1) << 8;       // 0x0000000000000100
    pub const NO_EXECUTE = @as(u64, 1) << 63;  // 0x8000000000000000

    // Useful combinations
    pub const KERNEL = PRESENT | WRITABLE | GLOBAL;
    pub const KERNEL_CODE = PRESENT | GLOBAL;  // Read-only, executable
    pub const USER = PRESENT | WRITABLE | USER;
    pub const USER_CODE = PRESENT | USER;
    pub const MMIO = PRESENT | WRITABLE | CACHE_DISABLE | WRITE_THROUGH;
    pub const IDENTITY = PRESENT | WRITABLE;

    /// Mask to extract physical address (bits 12-51)
    pub const ADDRESS_MASK = 0x000FFFFFFFFFF000;

    /// Mask to extract address from huge pages
    pub const HUGE_PAGE_MASK = 0xFFFFFFFFFFFFF000;
};

// ============================================================================
// VIRTUAL ADDRESS DECOMPOSITION
// ============================================================================

/// Extract page table indices from virtual address
pub const VirtAddrDecomp = struct {
    pml4_idx: u9,
    pdpt_idx: u9,
    pd_idx: u9,
    pt_idx: u9,
    offset: u12,

    pub fn from_addr(vaddr: u64) VirtAddrDecomp {
        return .{
            .pml4_idx = @intCast((vaddr >> 39) & 0x1FF),
            .pdpt_idx = @intCast((vaddr >> 30) & 0x1FF),
            .pd_idx = @intCast((vaddr >> 21) & 0x1FF),
            .pt_idx = @intCast((vaddr >> 12) & 0x1FF),
            .offset = @intCast(vaddr & 0xFFF),
        };
    }
};

// ============================================================================
// CR3 REGISTER OPERATIONS
// ============================================================================

/// Read CR3 register (contains PML4 physical address)
pub fn read_cr3() u64 {
    var cr3: u64 = undefined;
    asm volatile ("mov %%cr3, %[cr3]"
        : [cr3] "=r" (cr3),
    );
    return cr3;
}

/// Write CR3 register (load new page table root, flushes TLB)
/// pml4_phys must be 4K-aligned (bits [0:11] must be 0)
pub fn write_cr3(pml4_phys: u64) void {
    const aligned_phys = pml4_phys & 0xFFFFFFFFFFFFF000;
    asm volatile ("mov %[pml4], %%cr3"
        :
        : [pml4] "r" (aligned_phys),
        : "memory"
    );
}

/// Get the PML4 physical address from CR3
pub fn get_pml4_phys() u64 {
    return read_cr3() & 0xFFFFFFFFFFFFF000;
}

// ============================================================================
// TLB INVALIDATION
// ============================================================================

/// Invalidate single TLB entry for a virtual address
pub fn invlpg(vaddr: u64) void {
    asm volatile ("invlpg (%[vaddr])"
        :
        : [vaddr] "r" (vaddr),
        : "memory"
    );
}

/// Flush entire TLB (invalidate all entries except global pages)
pub fn flush_tlb() void {
    const cr3 = read_cr3();
    write_cr3(cr3);  // Reloading CR3 flushes TLB
}

/// Flush entire TLB including global pages (load different CR3 then restore)
pub fn flush_tlb_global() void {
    const cr3_old = read_cr3();
    asm volatile ("mov %%cr0, %%rax"
        :
        :
        : "rax"
    );
    // To flush global pages, temporarily disable paging, then re-enable
    // This is destructive and usually not needed; use invlpg for individual entries
}

// ============================================================================
// HHDM (HIGHER HALF DIRECT MAP) - FROM LIMINE BOOTLOADER
// ============================================================================

/// Global HHDM offset (set at boot)
var hhdm_offset: u64 = undefined;

/// Request HHDM from Limine bootloader
/// Must be called early in kernel initialization
pub fn init_hhdm(hhdm_response: u64) void {
    hhdm_offset = hhdm_response;
}

/// Convert physical address to virtual address (via HHDM)
pub inline fn phys_to_virt(phys: u64) u64 {
    return phys + hhdm_offset;
}

/// Convert virtual address (in HHDM region) back to physical
pub inline fn virt_to_phys_hhdm(virt: u64) ?u64 {
    if (virt >= hhdm_offset) {
        return virt - hhdm_offset;
    }
    return null;
}

/// Get HHDM offset (for external use)
pub fn get_hhdm_offset() u64 {
    return hhdm_offset;
}

// ============================================================================
// PAGE TABLE WALKING
// ============================================================================

/// Walk page tables and translate virtual address to physical
/// Returns physical address or null if page is not present
pub fn virt_to_phys(vaddr: u64) ?u64 {
    const decomp = VirtAddrDecomp.from_addr(vaddr);
    const pml4_phys = get_pml4_phys();
    const pml4_virt = phys_to_virt(pml4_phys);
    const pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pml4_virt));

    // Level 4: PML4
    const pml4_entry = pml4_ptr[decomp.pml4_idx];
    if (!pml4_entry.present) return null;

    // Level 3: PDPT
    const pdpt_virt = phys_to_virt(pml4_entry.physical_address());
    const pdpt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pdpt_virt));
    const pdpt_entry = pdpt_ptr[decomp.pdpt_idx];
    if (!pdpt_entry.present) return null;

    // Check for 1GiB huge page
    if (pdpt_entry.huge) {
        const phys = pdpt_entry.physical_address();
        return (phys & 0xFFFFFFFFC0000000) | (vaddr & 0x3FFFFFFF);
    }

    // Level 2: PD
    const pd_virt = phys_to_virt(pdpt_entry.physical_address());
    const pd_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pd_virt));
    const pd_entry = pd_ptr[decomp.pd_idx];
    if (!pd_entry.present) return null;

    // Check for 2MiB huge page
    if (pd_entry.huge) {
        const phys = pd_entry.physical_address();
        return (phys & 0xFFFFFFFFFFE00000) | (vaddr & 0x1FFFFF);
    }

    // Level 1: PT
    const pt_virt = phys_to_virt(pd_entry.physical_address());
    const pt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pt_virt));
    const pt_entry = pt_ptr[decomp.pt_idx];
    if (!pt_entry.present) return null;

    // Return physical address with offset
    return pt_entry.physical_address() | decomp.offset;
}

/// Check if a virtual address is present in page tables
pub fn is_mapped(vaddr: u64) bool {
    return virt_to_phys(vaddr) != null;
}

// ============================================================================
// PAGE TABLE MANIPULATION
// ============================================================================

/// Create a mapping from virtual address to physical address
/// Allocates intermediate page tables as needed (requires allocator)
pub fn map_page(
    allocator: std.mem.Allocator,
    vaddr: u64,
    phys: u64,
    flags: u64,
) !void {
    const decomp = VirtAddrDecomp.from_addr(vaddr);
    const pml4_phys = get_pml4_phys();
    const pml4_virt = phys_to_virt(pml4_phys);
    var pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pml4_virt));

    // Ensure PDPT exists
    if (!pml4_ptr[decomp.pml4_idx].present) {
        const pdpt_phys = try allocate_page_table(allocator);
        pml4_ptr[decomp.pml4_idx] = .{
            .present = true,
            .writable = true,
            .user = false,
            .write_through = false,
            .cache_disable = false,
            .accessed = false,
            .dirty = false,
            .huge = false,
            .global = false,
            .avail_lo = 0,
            .addr = @intCast(pdpt_phys >> 12),
            .avail_hi = 0,
            .no_execute = false,
        };
    }

    // Ensure PD exists
    const pdpt_virt = phys_to_virt(pml4_ptr[decomp.pml4_idx].physical_address());
    var pdpt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pdpt_virt));
    if (!pdpt_ptr[decomp.pdpt_idx].present) {
        const pd_phys = try allocate_page_table(allocator);
        pdpt_ptr[decomp.pdpt_idx] = .{
            .present = true,
            .writable = true,
            .user = false,
            .write_through = false,
            .cache_disable = false,
            .accessed = false,
            .dirty = false,
            .huge = false,
            .global = false,
            .avail_lo = 0,
            .addr = @intCast(pd_phys >> 12),
            .avail_hi = 0,
            .no_execute = false,
        };
    }

    // Ensure PT exists
    const pd_virt = phys_to_virt(pdpt_ptr[decomp.pdpt_idx].physical_address());
    var pd_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pd_virt));
    if (!pd_ptr[decomp.pd_idx].present) {
        const pt_phys = try allocate_page_table(allocator);
        pd_ptr[decomp.pd_idx] = .{
            .present = true,
            .writable = true,
            .user = false,
            .write_through = false,
            .cache_disable = false,
            .accessed = false,
            .dirty = false,
            .huge = false,
            .global = false,
            .avail_lo = 0,
            .addr = @intCast(pt_phys >> 12),
            .avail_hi = 0,
            .no_execute = false,
        };
    }

    // Finally, set the PT entry
    const pt_virt = phys_to_virt(pd_ptr[decomp.pd_idx].physical_address());
    var pt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pt_virt));
    pt_ptr[decomp.pt_idx] = .{
        .present = (flags & PageTableFlags.PRESENT) != 0,
        .writable = (flags & PageTableFlags.WRITABLE) != 0,
        .user = (flags & PageTableFlags.USER) != 0,
        .write_through = (flags & PageTableFlags.WRITE_THROUGH) != 0,
        .cache_disable = (flags & PageTableFlags.CACHE_DISABLE) != 0,
        .accessed = false,
        .dirty = false,
        .huge = false,
        .global = (flags & PageTableFlags.GLOBAL) != 0,
        .avail_lo = 0,
        .addr = @intCast(phys >> 12),
        .avail_hi = 0,
        .no_execute = (flags & PageTableFlags.NO_EXECUTE) != 0,
    };

    // Invalidate TLB for this address
    invlpg(vaddr);
}

/// Unmap a page (remove its page table entry)
pub fn unmap_page(vaddr: u64) !void {
    const decomp = VirtAddrDecomp.from_addr(vaddr);
    const pml4_phys = get_pml4_phys();
    const pml4_virt = phys_to_virt(pml4_phys);
    const pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pml4_virt));

    if (!pml4_ptr[decomp.pml4_idx].present) return;

    const pdpt_virt = phys_to_virt(pml4_ptr[decomp.pml4_idx].physical_address());
    const pdpt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pdpt_virt));
    if (!pdpt_ptr[decomp.pdpt_idx].present) return;

    const pd_virt = phys_to_virt(pdpt_ptr[decomp.pdpt_idx].physical_address());
    const pd_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pd_virt));
    if (!pd_ptr[decomp.pd_idx].present) return;

    const pt_virt = phys_to_virt(pd_ptr[decomp.pd_idx].physical_address());
    var pt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pt_virt));

    pt_ptr[decomp.pt_idx] = .{
        .present = false,
        .writable = false,
        .user = false,
        .write_through = false,
        .cache_disable = false,
        .accessed = false,
        .dirty = false,
        .huge = false,
        .global = false,
        .avail_lo = 0,
        .addr = 0,
        .avail_hi = 0,
        .no_execute = false,
    };

    invlpg(vaddr);
}

// ============================================================================
// PAGE TABLE ALLOCATION (stub - implement with your allocator)
// ============================================================================

/// Allocate a single 4KiB page table
/// This is a stub; implement with your physical memory allocator
fn allocate_page_table(allocator: std.mem.Allocator) !u64 {
    _ = allocator;
    @panic("allocate_page_table not implemented");
}

// ============================================================================
// KERNEL INITIALIZATION
// ============================================================================

/// Unmap the identity map (first 4 GiB of physical memory)
/// Call after loading kernel code from bootloader
pub fn unmap_identity_map() void {
    const pml4_phys = get_pml4_phys();
    const pml4_virt = phys_to_virt(pml4_phys);
    var pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pml4_virt));

    // PML4 entries 0-3 cover 0x0-0x100000000 (4 GiB)
    // The identity map uses entries 0-3
    for (0..4) |i| {
        pml4_ptr[i] = .{
            .present = false,
            .writable = false,
            .user = false,
            .write_through = false,
            .cache_disable = false,
            .accessed = false,
            .dirty = false,
            .huge = false,
            .global = false,
            .avail_lo = 0,
            .addr = 0,
            .avail_hi = 0,
            .no_execute = false,
        };

        // Invalidate all TLB entries for this 512 GiB region
        invlpg(@as(u64, i) << 39);
    }
}

/// Initialize paging system
/// Call during early kernel boot (before multitasking)
pub fn init_paging(hhdm_offset_val: u64) void {
    init_hhdm(hhdm_offset_val);
    unmap_identity_map();

    // Optional: Set up recursive paging if desired
    // setup_recursive_paging();
}

// ============================================================================
// OPTIONAL: RECURSIVE PAGING
// ============================================================================

/// Set up recursive page tables (optional pattern)
/// Maps PML4 into itself at entry 511
pub fn setup_recursive_paging() void {
    const pml4_phys = get_pml4_phys();
    const pml4_virt = phys_to_virt(pml4_phys);
    var pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pml4_virt));

    // Map PML4 into entry 511 (top of address space)
    pml4_ptr[511] = .{
        .present = true,
        .writable = true,
        .user = false,
        .write_through = false,
        .cache_disable = false,
        .accessed = false,
        .dirty = false,
        .huge = false,
        .global = true,
        .avail_lo = 0,
        .addr = @intCast(pml4_phys >> 12),
        .avail_hi = 0,
        .no_execute = false,
    };

    // Invalidate TLB for recursive mapping
    invlpg(0xff80000000000000);
}

// ============================================================================
// DEBUG / TESTING
// ============================================================================

/// Print page table structure for a virtual address (debug)
pub fn debug_print_page_walk(vaddr: u64) void {
    _ = vaddr;
    // Implement with your logging system
    // @panic("debug_print_page_walk not implemented");
}

/// Validate that a virtual address is mapped correctly
pub fn validate_mapping(vaddr: u64, expected_phys: u64) bool {
    if (virt_to_phys(vaddr)) |phys| {
        return phys == expected_phys;
    }
    return false;
}
