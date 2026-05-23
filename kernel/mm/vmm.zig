// ============================================================================
// VantaOS — Virtual Memory Manager (x86_64, 4-level paging)
//
// Phase 1: page table walks, map/unmap, HHDM helpers, address-space create.
// Builds on Limine's page tables — kernel already mapped at higher half.
// ============================================================================

const pmm = @import("pmm.zig");
const limine = @import("../limine.zig");
const serial = @import("../arch/x86_64/serial.zig");

pub const PAGE_SIZE: u64 = 4096;

// HHDM offset captured from Limine — phys + offset = virt
pub var hhdm_offset: u64 = 0;

// ── PTE ────────────────────────────────────────────────────────

pub const PTE_PRESENT: u64 = 1 << 0;
pub const PTE_WRITE:   u64 = 1 << 1;
pub const PTE_USER:    u64 = 1 << 2;
pub const PTE_WT:      u64 = 1 << 3;
pub const PTE_CD:      u64 = 1 << 4;
pub const PTE_ACCESS:  u64 = 1 << 5;
pub const PTE_DIRTY:   u64 = 1 << 6;
pub const PTE_HUGE:    u64 = 1 << 7;
pub const PTE_GLOBAL:  u64 = 1 << 8;
pub const PTE_NX:      u64 = 1 << 63;

pub const ADDR_MASK: u64 = 0x000FFFFFFFFFF000;

pub inline fn phys2virt(p: u64) u64 {
    if (p >= hhdm_offset and hhdm_offset != 0) return p;
    return p + hhdm_offset;
}
pub inline fn virt2phys_hhdm(v: u64) u64 { return v - hhdm_offset; }

// ── Address Space ──────────────────────────────────────────────

pub const AddressSpace = struct {
    pml4_phys: u64,

    pub fn current() AddressSpace {
        return .{ .pml4_phys = readCr3() & ADDR_MASK };
    }

    pub fn activate(self: AddressSpace) void {
        writeCr3(self.pml4_phys);
    }
};

pub fn createAddressSpace() ?AddressSpace {
    const pml4 = pmm.allocPage() orelse return null;
    const v = @as([*]volatile u64, @ptrFromInt(phys2virt(pml4)));
    // Zero entire PML4
    var i: usize = 0;
    while (i < 512) : (i += 1) v[i] = 0;
    // Copy kernel-half entries (256-511) from current PML4 — share kernel space
    const cur_pml4 = readCr3() & ADDR_MASK;
    const cur = @as([*]volatile u64, @ptrFromInt(phys2virt(cur_pml4)));
    i = 256;
    while (i < 512) : (i += 1) v[i] = cur[i];
    return .{ .pml4_phys = pml4 };
}

// ── CR3 / TLB ──────────────────────────────────────────────────

pub fn readCr3() u64 {
    var v: u64 = 0;
    asm volatile ("mov %%cr3, %[v]" : [v] "=r" (v));
    return v;
}

pub fn writeCr3(p: u64) void {
    asm volatile ("mov %[p], %%cr3" :: [p] "r" (p) : .{ .memory = true });
}

pub fn invlpg(v: u64) void {
    asm volatile ("invlpg (%[v])" :: [v] "r" (v) : .{ .memory = true });
}

// ── Page Table Walk ────────────────────────────────────────────

inline fn idxPml4(v: u64) usize { return @intCast((v >> 39) & 0x1FF); }
inline fn idxPdpt(v: u64) usize { return @intCast((v >> 30) & 0x1FF); }
inline fn idxPd  (v: u64) usize { return @intCast((v >> 21) & 0x1FF); }
inline fn idxPt  (v: u64) usize { return @intCast((v >> 12) & 0x1FF); }

fn tableAt(phys: u64) [*]volatile u64 {
    return @ptrFromInt(phys2virt(phys));
}

/// Walk page tables; if `create` is true, allocate intermediate tables.
/// Returns pointer to PTE in PT, or null on failure / not-present.
fn walk(pml4_phys: u64, v: u64, create: bool, user: bool) ?*volatile u64 {
    var table = tableAt(pml4_phys);
    const levels = [3]usize{ idxPml4(v), idxPdpt(v), idxPd(v) };

    inline for (levels) |idx| {
        const e = table[idx];
        if ((e & PTE_PRESENT) == 0) {
            if (!create) return null;
            const new_table = pmm.allocPage() orelse return null;
            const nt = tableAt(new_table);
            var i: usize = 0;
            while (i < 512) : (i += 1) nt[i] = 0;
            const flags = PTE_PRESENT | PTE_WRITE | (if (user) PTE_USER else 0);
            table[idx] = new_table | flags;
            table = nt;
        } else {
            // huge page on PDPT/PD — not supported in walk
            if ((e & PTE_HUGE) != 0) return null;
            table = tableAt(e & ADDR_MASK);
        }
    }
    return &table[idxPt(v)];
}

/// Map a single 4K page. Returns false on failure.
pub fn map(space: AddressSpace, v: u64, p: u64, flags: u64) bool {
    const pte = walk(space.pml4_phys, v, true, (flags & PTE_USER) != 0) orelse return false;
    pte.* = (p & ADDR_MASK) | flags | PTE_PRESENT;
    invlpg(v);
    return true;
}

/// Unmap a single 4K page.
pub fn unmap(space: AddressSpace, v: u64) void {
    const pte = walk(space.pml4_phys, v, false, false) orelse return;
    pte.* = 0;
    invlpg(v);
}

/// Translate virt → phys. Returns null if not mapped.
pub fn translate(space: AddressSpace, v: u64) ?u64 {
    const pte = walk(space.pml4_phys, v, false, false) orelse return null;
    if ((pte.* & PTE_PRESENT) == 0) return null;
    return (pte.* & ADDR_MASK) | (v & 0xFFF);
}

// ── Init ───────────────────────────────────────────────────────

pub fn init(hhdm: *volatile limine.HhdmResponse) void {
    hhdm_offset = hhdm.offset;
    serial.puts("[VMM]   HHDM=0x");
    serial.putHex(hhdm_offset);
    serial.puts("\n");
    const cr3 = readCr3();
    serial.puts("[VMM]   CR3=0x");
    serial.putHex(cr3);
    serial.puts(" (using Limine's tables)\n");
}
