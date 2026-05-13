# x86_64 Paging Quick Start for VantaOS Phase 1

This is your **action guide** for implementing paging in your Zig-based microkernel. See `PAGING_REFERENCE.md` for detailed specs and `PAGING_IMPLEMENTATION.zig` for code.

---

## TL;DR

**4-level page tables on x86_64:**
- PML4 → PDPT → PD → PT (each 512 entries, 8 bytes each)
- Virtual address indices extracted from bits [39-47], [30-38], [21-29], [12-20]
- Page offset is bottom 12 bits (0-11)
- CR3 register holds PML4 physical address
- TLB invalidation: `invlpg(vaddr)` or reload CR3
- Limine provides HHDM (physical RAM accessible at high virtual address)

---

## Step 1: Understand the Data Structure

### Page Table Entry (64-bit)
```
Bit 0:     P (Present)
Bit 1:     RW (Read/Write)
Bit 2:     U/S (User/Supervisor)
Bits 3-4:  PWT, PCD (caching flags)
Bits 5-6:  A, D (Accessed, Dirty - set by CPU)
Bit 7:     PS (Huge page flag: 2MiB or 1GiB)
Bit 8:     G (Global - don't flush on CR3 reload)
Bits 9-11: Available (use for refcount, page type, etc.)
Bits 12-51: Physical address (4K-aligned, so bits 0-11 are 0)
Bits 52-62: Available
Bit 63:    NX (No-Execute)
```

**In Zig:** Use the `PageTableEntry` packed struct in `PAGING_IMPLEMENTATION.zig`.

### Virtual Address Decomposition
```
Bits 39-47:  PML4 index (9 bits) → selector into 512-entry PML4 table
Bits 30-38:  PDPT index (9 bits) → selector into 512-entry PDPT table
Bits 21-29:  PD index (9 bits)   → selector into 512-entry PD table
Bits 12-20:  PT index (9 bits)   → selector into 512-entry PT table
Bits 0-11:   Page offset (4096 bytes = 4 KiB)
Bits 48-63:  Sign extension (must match bit 47 for canonical address)
```

Extract in Zig:
```zig
const pml4_idx = (vaddr >> 39) & 0x1FF;
const pdpt_idx = (vaddr >> 30) & 0x1FF;
const pd_idx   = (vaddr >> 21) & 0x1FF;
const pt_idx   = (vaddr >> 12) & 0x1FF;
const offset   = vaddr & 0xFFF;
```

---

## Step 2: Understand the Page Walk

**Goal:** Convert virtual address → physical address

**Algorithm:**
1. Read CR3 → get PML4 physical address
2. Use PML4 index to read PML4 entry → get PDPT physical address
3. Use PDPT index to read PDPT entry → get PD physical address (or 1GiB page if PS=1)
4. Use PD index to read PD entry → get PT physical address (or 2MiB page if PS=1)
5. Use PT index to read PT entry → get physical page address
6. Combine page address + offset → final physical address

**In Zig:** Use `virt_to_phys(vaddr)` from `PAGING_IMPLEMENTATION.zig`.

---

## Step 3: Know When to Use HHDM

**HHDM** = Higher Half Direct Map (provided by Limine bootloader)

**What it gives you:**
- All physical RAM is accessible at virtual address `hhdm_offset + phys_addr`
- No need to set up individual page table entries for memory management
- Kernel can read/write page tables themselves via HHDM

**Request from bootloader:**
```zig
pub const HhdmRequest = extern struct {
    id: [4]u64 = .{...},  // Magic number from Limine spec
    revision: u64 = 0,
    response: ?*HhdmResponse = null,
};

pub const HhdmResponse = extern struct {
    revision: u64,
    offset: u64,  // HHDM virtual address offset
};
```

**Use in code:**
```zig
const hhdm_offset = response.offset;
const virt_of_phys = phys_addr + hhdm_offset;  // Access physical memory
const pt_entry = @as(*PageTableEntry, @ptrFromInt(virt_of_phys)).*;
```

---

## Step 4: Implement Core Functions

### Read/Write CR3
```zig
pub fn read_cr3() u64 {
    var cr3: u64 = undefined;
    asm volatile ("mov %%cr3, %[cr3]"
        : [cr3] "=r" (cr3),
    );
    return cr3;
}

pub fn write_cr3(pml4_phys: u64) void {
    asm volatile ("mov %[pml4], %%cr3"
        :
        : [pml4] "r" (pml4_phys & 0xFFFFFFFFFFFFF000)
    );
}
```

### TLB Invalidation
```zig
pub fn invlpg(vaddr: u64) void {
    asm volatile ("invlpg (%[vaddr])"
        :
        : [vaddr] "r" (vaddr)
        : "memory"
    );
}

pub fn flush_tlb() void {
    const cr3 = read_cr3();
    write_cr3(cr3);  // Reloading CR3 flushes TLB
}
```

### Virtual → Physical Walk
```zig
pub fn virt_to_phys(vaddr: u64) ?u64 {
    const pml4_idx = (vaddr >> 39) & 0x1FF;
    const pml4_addr = read_cr3() & 0xFFFFFFFFFFFFF000;
    const pml4 = @as(*[512]PageTableEntry, @ptrFromInt(hhdm_offset + pml4_addr));
    
    // Check PML4 entry
    if (!pml4[pml4_idx].present) return null;
    
    // Repeat for PDPT, PD, PT...
    // See PAGING_IMPLEMENTATION.zig for full walk
}
```

---

## Step 5: Map and Unmap Pages

### Create a new mapping
```zig
pub fn map_page(
    allocator: std.mem.Allocator,
    vaddr: u64,
    phys: u64,
    flags: u64,
) !void {
    // 1. For each level (PML4, PDPT, PD), check if next table exists
    // 2. If not, allocate a new page table and insert it
    // 3. At PT level, set the entry to point to phys with flags
    // 4. invlpg(vaddr) to flush TLB for this address
    
    // See PAGING_IMPLEMENTATION.zig map_page() for full logic
}
```

### Remove a mapping
```zig
pub fn unmap_page(vaddr: u64) !void {
    // Walk to the PT entry for vaddr
    // Set its P (present) bit to 0
    // invlpg(vaddr)
}
```

---

## Step 6: Early Boot Setup

### In your kernel entry point:
```zig
pub fn kernel_main() void {
    // 1. Receive HHDM offset from bootloader
    const hhdm_response = get_limine_hhdm_response();
    init_hhdm(hhdm_response.offset);
    
    // 2. Bootloader has set up:
    //    - PML4 with identity map (phys 0x1000-4GB visible at virt 0x1000-4GB)
    //    - Kernel at 0xffffffff80000000
    //    - HHDM at hhdm_offset + all_physical_ram
    
    // 3. Unmap identity map to reclaim 4 GiB
    unmap_identity_map();
    
    // 4. Now set up your allocator using HHDM
    //    (page frame database, buddy allocator, etc.)
    init_physical_allocator();
    
    // 5. (Optional) Set up recursive paging
    // setup_recursive_paging();
    
    // 6. Set up heap, drivers, scheduler...
}
```

### Unmap identity map
```zig
pub fn unmap_identity_map() void {
    const pml4_phys = get_pml4_phys();
    const pml4_virt = phys_to_virt(pml4_phys);
    var pml4 = @as(*[512]PageTableEntry, @ptrFromInt(pml4_virt));
    
    // PML4 entries 0-3 cover first 4 GiB
    for (0..4) |i| {
        pml4[i].present = false;
        invlpg(@as(u64, i) << 39);  // Invalidate all 512 GiB chunks
    }
}
```

---

## Step 7: Advanced Topics

### Huge Pages (2 MiB / 1 GiB)
- Set `PS` bit (bit 7) in PD/PDPT entry
- **2 MiB:** Set PS in PD entry; physical address bits [21-63] form the page address
- **1 GiB:** Set PS in PDPT entry; physical address bits [30-63] form the page address
- Benefit: Faster translation, fewer TLB entries needed

### Recursive Paging (for microkernels)
Map PML4 into itself at index 511:
```zig
pml4[511] = PageTableEntry{
    .present = true,
    .writable = true,
    .addr = pml4_phys >> 12,  // Points to PML4 itself
    .global = true,
    ...
};
```
Result: PML4 accessible at `0xff80000000000000`, any PT at `0xff80014040000000 + index`, etc.
- Pro: Modify page tables without special tricks
- Con: Loses 512 GiB of address space

### Capability-Based Memory Isolation
For your capability microkernel:
- Each task/domain gets its own page tables (new PML4)
- Use `map_page(allocator, vaddr, phys, flags)` to install mappings
- Use permission bits (U/S, RW, NX) to enforce capabilities
- Switch contexts: `write_cr3(task->pml4_phys)`

---

## Quick Flag Reference

Common flag combinations (bitwise OR):
```zig
PRESENT | WRITABLE | GLOBAL           // Kernel data
PRESENT | GLOBAL                       // Kernel code (read-only)
PRESENT | WRITABLE | USER             // User data
PRESENT | USER                        // User code (read-only)
PRESENT | WRITABLE | CACHE_DISABLE    // MMIO device memory
PRESENT | WRITABLE | WRITE_THROUGH    // Framebuffer (write-combining)
```

---

## Common Mistakes to Avoid

1. **Forgetting invlpg after modifying page tables**
   - TLB caches mappings; changes don't take effect until invalidated
   
2. **Not aligning addresses to 4 KiB**
   - Physical addresses in entries must have bits [0:11] = 0
   - Use `phys & 0xFFFFFFFFFFFFF000` when loading into entry

3. **Assuming all 64 bits of virtual address are valid**
   - x86_64 enforces canonical form: bits [63:48] must match bit 47
   - Valid ranges: `0x0000_0000_0000_0000 - 0x00007fff_ffff_ffff` and `0xffff_8000_0000_0000 - 0xffff_ffff_ffff_ffff`

4. **Mixing physical and virtual addresses**
   - Limine gives physical; use HHDM to convert to virtual for access
   - CR3 holds physical; extract with `& 0xFFFFFFFFFFFFF000`

5. **Not checking P (present) bit before dereferencing**
   - Missing page = page fault exception
   - Always check `if (!entry.present) return null;`

---

## Files Provided

1. **PAGING_REFERENCE.md** — Complete technical specification (10 parts)
2. **PAGING_IMPLEMENTATION.zig** — Ready-to-integrate Zig code (250+ lines)
3. **PAGING_QUICKSTART.md** — This file (your action guide)

---

## Next Steps

1. Copy `PAGING_IMPLEMENTATION.zig` into your kernel codebase
2. Implement `allocate_page_table()` using your physical memory allocator
3. Call `init_paging(hhdm_offset)` at boot
4. Use `map_page()` and `virt_to_phys()` for memory management
5. Implement page fault handler for demand paging (later phase)

---

## References

- **Limine Bootloader v8 Protocol:** Describes HHDM, memory map, bootloader setup
- **Intel Manual Vol. 3:** Full MMU and paging specification
- **Wikipedia: Page Table:** Overview of paging concepts
- **SerenityOS Kernel:** Example x86_64 paging in C++

Good luck with VantaOS Phase 1!
