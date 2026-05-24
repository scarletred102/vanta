// ============================================================================
// VantaOS — Virtual Memory Manager (x86_64, 4-level paging)
//
// Phase 1: page table walks, map/unmap, HHDM helpers, address-space create.
// Builds on Limine's page tables — kernel already mapped at higher half.
// ============================================================================

const pmm = @import("pmm.zig");
const limine = @import("../limine.zig");
const serial = @import("../arch/x86_64/serial.zig");
const cpu_local = @import("../arch/x86_64/cpu_local.zig");
const interrupts = @import("../arch/x86_64/interrupts.zig");

pub const PAGE_SIZE: u64 = 4096;

// HHDM offset captured from Limine — phys + offset = virt
pub var hhdm_offset: u64 = 0;

// TLB shootdown state
pub var shootdown_va: u64 = 0;
pub var shootdown_count: u32 = 0;

// Send TLB invalidation IPI to all CPUs sharing the same pml4 (excluding self).
// Vector 0x40 is the TLB shootdown IPI.
pub fn tlb_shootdown(pml4_phys: u64, va: u64) void {
    const n = @atomicLoad(u32, &cpu_local.cpu_count, .monotonic);
    if (n <= 1) return;

    // Count CPUs that need shootdown
    var targets: u32 = 0;
    var i: usize = 0;
    while (i < n) : (i += 1) {
        const t = cpu_local.cpus[i].current_thread orelse continue;
        if (t.page_table == pml4_phys) targets += 1;
    }
    if (targets == 0) return;

    @atomicStore(u64, &shootdown_va, va, .release);
    @atomicStore(u32, &shootdown_count, targets, .release);

    // Send fixed IPI (vector 0x40) to all other CPUs
    i = 0;
    while (i < n) : (i += 1) {
        const apic_id = cpu_local.cpus[i].apic_id;
        const my_apic = cpu_local.get_cpu_local().apic_id;
        if (apic_id == my_apic) continue;
        interrupts.lapicWrite(0x310, @as(u32, apic_id) << 24);
        interrupts.lapicWrite(0x300, 0x00004040); // fixed, vector 0x40
    }

    // Spin until all targets have acknowledged
    var timeout: u32 = 1_000_000;
    while (@atomicLoad(u32, &shootdown_count, .acquire) > 0 and timeout > 0) : (timeout -= 1) {
        asm volatile ("pause");
    }
}

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
pub const PTE_COW:     u64 = 1 << 9;
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

pub fn create_user_address_space() ?u64 {
    const pml4 = pmm.allocPage() orelse return null;
    const v = @as([*]volatile u64, @ptrFromInt(phys2virt(pml4)));
    // Zero entire PML4
    var i: usize = 0;
    while (i < 512) : (i += 1) v[i] = 0;
    // Copy kernel-half entries (256-511) from current PML4 (shares kernel mappings above 0xFFFF800000000000)
    const cur_pml4 = readCr3() & ADDR_MASK;
    const cur = @as([*]volatile u64, @ptrFromInt(phys2virt(cur_pml4)));
    i = 256;
    while (i < 512) : (i += 1) v[i] = cur[i];
    return pml4;
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
    tlb_shootdown(space.pml4_phys, v);
    return true;
}

/// Unmap a single 4K page.
pub fn unmap(space: AddressSpace, v: u64) void {
    const pte = walk(space.pml4_phys, v, false, false) orelse return;
    pte.* = 0;
    invlpg(v);
    tlb_shootdown(space.pml4_phys, v);
}

/// Map a page as non-present in the page table (but ensure intermediate tables exist with user permission).
/// Returns false on failure to allocate intermediate page tables.
pub fn map_non_present(space: AddressSpace, v: u64) bool {
    const pte = walk(space.pml4_phys, v, true, true) orelse return false;
    pte.* = 0;
    invlpg(v);
    return true;
}

/// Allocate n_pages from buddy, map them top-down at a fixed virtual range (0x7FFF00000000 downward),
/// map one additional unmapped guard page below the bottom. Return top-of-stack virtual address.
pub fn alloc_user_stack(n_pages: usize) ?u64 {
    if (n_pages == 0) return null;
    const space = AddressSpace.current();
    const top_addr: u64 = 0x7FFF00000000;

    var i: usize = 0;
    while (i < n_pages) : (i += 1) {
        const paddr = pmm.allocPage() orelse {
            // Cleanup previously mapped pages
            var j: usize = 0;
            while (j < i) : (j += 1) {
                const cleanup_vaddr = top_addr - (j + 1) * PAGE_SIZE;
                if (translate(space, cleanup_vaddr)) |phys| {
                    unmap(space, cleanup_vaddr);
                    pmm.freePage(phys);
                }
            }
            return null;
        };
        const vaddr = top_addr - (i + 1) * PAGE_SIZE;
        // Map as user, writable
        if (!map(space, vaddr, paddr, PTE_USER | PTE_WRITE)) {
            pmm.freePage(paddr);
            // Cleanup previously mapped pages
            var j: usize = 0;
            while (j < i) : (j += 1) {
                const cleanup_vaddr = top_addr - (j + 1) * PAGE_SIZE;
                if (translate(space, cleanup_vaddr)) |phys| {
                    unmap(space, cleanup_vaddr);
                    pmm.freePage(phys);
                }
            }
            return null;
        }
    }

    // Map one additional guard page below the bottom with no-present PTE
    const guard_vaddr = top_addr - (n_pages + 1) * PAGE_SIZE;
    if (!map_non_present(space, guard_vaddr)) {
        // Cleanup all stack pages
        var j: usize = 0;
        while (j < n_pages) : (j += 1) {
            const cleanup_vaddr = top_addr - (j + 1) * PAGE_SIZE;
            if (translate(space, cleanup_vaddr)) |phys| {
                unmap(space, cleanup_vaddr);
                pmm.freePage(phys);
            }
        }
        return null;
    }

    return top_addr;
}

pub fn alloc_user_stack_in_space(space: AddressSpace, n_pages: usize) ?u64 {
    if (n_pages == 0) return null;
    const top_addr: u64 = 0x7FFF00000000;

    var i: usize = 0;
    while (i < n_pages) : (i += 1) {
        const paddr = pmm.allocPage() orelse {
            var j: usize = 0;
            while (j < i) : (j += 1) {
                const cleanup_vaddr = top_addr - (j + 1) * PAGE_SIZE;
                if (translate(space, cleanup_vaddr)) |phys| {
                    unmap(space, cleanup_vaddr);
                    pmm.freePage(phys);
                }
            }
            return null;
        };
        const vaddr = top_addr - (i + 1) * PAGE_SIZE;
        if (!map(space, vaddr, paddr, PTE_USER | PTE_WRITE)) {
            pmm.freePage(paddr);
            var j: usize = 0;
            while (j < i) : (j += 1) {
                const cleanup_vaddr = top_addr - (j + 1) * PAGE_SIZE;
                if (translate(space, cleanup_vaddr)) |phys| {
                    unmap(space, cleanup_vaddr);
                    pmm.freePage(phys);
                }
            }
            return null;
        }
    }

    const guard_vaddr = top_addr - (n_pages + 1) * PAGE_SIZE;
    if (!map_non_present(space, guard_vaddr)) {
        var j: usize = 0;
        while (j < n_pages) : (j += 1) {
            const cleanup_vaddr = top_addr - (j + 1) * PAGE_SIZE;
            if (translate(space, cleanup_vaddr)) |phys| {
                unmap(space, cleanup_vaddr);
                pmm.freePage(phys);
            }
        }
        return null;
    }

    return top_addr;
}


/// Translate virt → phys. Returns null if not mapped.
pub fn translate(space: AddressSpace, v: u64) ?u64 {
    const pte = walk(space.pml4_phys, v, false, false) orelse return null;
    if ((pte.* & PTE_PRESENT) == 0) return null;
    return (pte.* & ADDR_MASK) | (v & 0xFFF);
}

/// Retrieve the pointer to the leaf page table entry (PTE) for a virtual address.
/// Returns null if not mapped.
pub fn getPte(space: AddressSpace, v: u64) ?*volatile u64 {
    return walk(space.pml4_phys, v, false, false);
}

pub fn freeAddressSpacePages(pml4_phys: u64) void {
    const pml4 = tableAt(pml4_phys);
    var idx4: usize = 0;
    while (idx4 < 256) : (idx4 += 1) {
        const e4 = pml4[idx4];
        if ((e4 & PTE_PRESENT) != 0) {
            const pdpt_phys = e4 & ADDR_MASK;
            const pdpt = tableAt(pdpt_phys);
            var idx3: usize = 0;
            while (idx3 < 512) : (idx3 += 1) {
                const e3 = pdpt[idx3];
                if ((e3 & PTE_PRESENT) != 0) {
                    if ((e3 & PTE_HUGE) != 0) {
                        pmm.freePage(e3 & ADDR_MASK);
                    } else {
                        const pd_phys = e3 & ADDR_MASK;
                        const pd = tableAt(pd_phys);
                        var idx2: usize = 0;
                        while (idx2 < 512) : (idx2 += 1) {
                            const e2 = pd[idx2];
                            if ((e2 & PTE_PRESENT) != 0) {
                                if ((e2 & PTE_HUGE) != 0) {
                                    pmm.freePage(e2 & ADDR_MASK);
                                } else {
                                    const pt_phys = e2 & ADDR_MASK;
                                    const pt = tableAt(pt_phys);
                                    var idx1: usize = 0;
                                    while (idx1 < 512) : (idx1 += 1) {
                                        const e1 = pt[idx1];
                                        if ((e1 & PTE_PRESENT) != 0) {
                                            pmm.freePage(e1 & ADDR_MASK);
                                        }
                                    }
                                    pmm.freePage(pt_phys);
                                }
                            }
                        }
                        pmm.freePage(pd_phys);
                    }
                }
            }
            pmm.freePage(pdpt_phys);
        }
    }
    pmm.freePage(pml4_phys);
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
