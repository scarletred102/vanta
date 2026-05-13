# x86_64 Virtual Memory & Paging for VantaOS Microkernel

## Overview
x86_64 uses 4-level paging (64-bit capability-based). Mandatory for microkernel isolation.

---

## Part 1: Page Table Entry Structure (64-bit)

### Bit Layout
```
Bit    Name          Meaning
---    ----          -------
[0]    P             Present (1=in physical memory, 0=page fault on access)
[1]    RW            Read/Write (1=rw, 0=ro; only checked if U/S=1)
[2]    U/S           User/Supervisor (1=user+supervisor, 0=supervisor only)
[3]    PWT           Write-Through (1=WT, 0=write-back caching)
[4]    PCD           Cache Disable (1=uncached MMIO, 0=cached)
[5]    A             Accessed (1=accessed; set by CPU hardware)
[6]    D             Dirty (1=written; page tables only, set by CPU)
[7]    PS/PAT        Huge Page flag (1=2MiB/1GiB, depends on level)
[8]    G             Global (1=don't flush on CR3 reload; kernel pages)
[9-11] AVL           Available for OS (user-defined)
[12-51] ADDR         Physical address (40 bits; bits [0:11] always 0)
[52-62] AVL          Available for OS (user-defined)
[63]   NX            No-Execute (1=not executable if EFER.NXE set)
```

### Flag Constants (Zig)
```zig
pub const PageTableFlags = struct {
    pub const PRESENT = @as(u64, 1) << 0;      // 0x0000000000000001
    pub const WRITABLE = @as(u64, 1) << 1;     // 0x0000000000000002
    pub const USER = @as(u64, 1) << 2;         // 0x0000000000000004
    pub const WRITE_THROUGH = @as(u64, 1) << 3; // 0x0000000000000008
    pub const CACHE_DISABLE = @as(u64, 1) << 4; // 0x0000000000000010
    pub const ACCESSED = @as(u64, 1) << 5;     // 0x0000000000000020
    pub const DIRTY = @as(u64, 1) << 6;        // 0x0000000000000040
    pub const HUGE = @as(u64, 1) << 7;         // 0x0000000000000080
    pub const GLOBAL = @as(u64, 1) << 8;       // 0x0000000000000100
    pub const NO_EXECUTE = @as(u64, 1) << 63;  // 0x8000000000000000
    pub const ADDRESS_MASK = 0x000FFFFFFFFFF000; // Extract [12:51]
};
```

### Zig Packed Struct
```zig
pub const PageTableEntry = packed struct(u64) {
    present: bool,           // [0]
    writable: bool,          // [1]
    user: bool,              // [2]
    write_through: bool,     // [3]
    cache_disable: bool,     // [4]
    accessed: bool,          // [5]
    dirty: bool,             // [6]
    huge: bool,              // [7]
    global: bool,            // [8]
    avail_lo: u3,            // [9-11]
    addr: u40,               // [12-51] physical address in 4K pages
    avail_hi: u11,           // [52-62]
    no_execute: bool,        // [63]
};
```

---

## Part 2: 4-Level Paging Hierarchy

x86_64 uses a 4-level hierarchical page table. Each table has 512 entries (9-bit index).

```
Virtual Address (64-bit):
  [63-48]  Sign Extension (canonical form)
  [39-47]  PML4 Index (9 bits) → selects from 512-entry Level 4 table
  [30-38]  PDPT Index (9 bits) → selects from 512-entry Level 3 table
  [21-29]  PD Index   (9 bits) → selects from 512-entry Level 2 table
  [12-20]  PT Index   (9 bits) → selects from 512-entry Level 1 table
  [0-11]   Page Offset (4 KiB = 4096 bytes)
```

### Hierarchy Levels

| Level | Name | Covers | Entry Size | Huge Page |
|-------|------|--------|-----------|-----------|
| 4 | PML4 (Page Map L4) | 512 GiB | 512 entries × 8 bytes | No |
| 3 | PDPT (Page Dir Ptr Table) | 1 GiB each | 512 entries × 8 bytes | 1 GiB (if PS=1) |
| 2 | PD (Page Directory) | 2 MiB each | 512 entries × 8 bytes | 2 MiB (if PS=1) |
| 1 | PT (Page Table) | 4 KiB each | 512 entries × 8 bytes | No |

### Index Extraction (Zig)
```zig
const pml4_idx = (vaddr >> 39) & 0x1FF;
const pdpt_idx = (vaddr >> 30) & 0x1FF;
const pd_idx   = (vaddr >> 21) & 0x1FF;
const pt_idx   = (vaddr >> 12) & 0x1FF;
const offset   = vaddr & 0xFFF;
```

---

## Part 3: Page Table Walk Algorithm (VA → PA)

**Input:** virtual address `vaddr`, CR3 register (contains PML4 physical address)  
**Output:** physical address (or page fault exception)

### Step-by-step pseudocode
```
function virt_to_phys(vaddr: u64) -> u64 {
    // Step 1: Load PML4 from CR3
    pml4_addr = read_cr3() & 0xFFFFFFFFFFFFF000;
    
    // Step 2: PML4 walk
    pml4_idx = (vaddr >> 39) & 0x1FF;
    pml4_entry = *(u64*)(pml4_addr + pml4_idx * 8);
    if !(pml4_entry & PRESENT) {
        trigger_page_fault();
        return;
    }
    
    // Step 3: PDPT walk
    pdpt_addr = pml4_entry & 0xFFFFFFFFFFFFF000;
    pdpt_idx = (vaddr >> 30) & 0x1FF;
    pdpt_entry = *(u64*)(pdpt_addr + pdpt_idx * 8);
    if !(pdpt_entry & PRESENT) {
        trigger_page_fault();
        return;
    }
    if (pdpt_entry & HUGE) {
        // 1 GiB page
        return (pdpt_entry & 0xFFFFFFFFC0000000) | (vaddr & 0x3FFFFFFF);
    }
    
    // Step 4: PD walk
    pd_addr = pdpt_entry & 0xFFFFFFFFFFFFF000;
    pd_idx = (vaddr >> 21) & 0x1FF;
    pd_entry = *(u64*)(pd_addr + pd_idx * 8);
    if !(pd_entry & PRESENT) {
        trigger_page_fault();
        return;
    }
    if (pd_entry & HUGE) {
        // 2 MiB page
        return (pd_entry & 0xFFFFFFFFFFE00000) | (vaddr & 0x1FFFFF);
    }
    
    // Step 5: PT walk
    pt_addr = pd_entry & 0xFFFFFFFFFFFFF000;
    pt_idx = (vaddr >> 12) & 0x1FF;
    pt_entry = *(u64*)(pt_addr + pt_idx * 8);
    if !(pt_entry & PRESENT) {
        trigger_page_fault();
        return;
    }
    
    // Step 6: Check permissions (NX, U/S, R/W)
    if (pt_entry & NO_EXECUTE && is_instruction_fetch) {
        trigger_page_fault();
    }
    
    // Step 7: Return physical address
    phys_addr = (pt_entry & 0xFFFFFFFFFFFFF000) | (vaddr & 0xFFF);
    
    // Step 8: Update A and D bits (hardware usually does this)
    pt_entry |= ACCESSED;
    if (is_write_access) {
        pt_entry |= DIRTY;
    }
    
    return phys_addr;
}
```

### Zig Implementation Skeleton
```zig
pub fn virt_to_phys(vaddr: u64) ?u64 {
    const pml4_addr = read_cr3() & 0xFFFFFFFFFFFFF000;
    
    // PML4
    const pml4_idx = (vaddr >> 39) & 0x1FF;
    const pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pml4_addr));
    const pml4_entry = pml4_ptr[pml4_idx];
    if (!pml4_entry.present) return null;
    
    // PDPT
    const pdpt_addr = @as(u64, pml4_entry.addr) << 12;
    const pdpt_idx = (vaddr >> 30) & 0x1FF;
    const pdpt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pdpt_addr));
    const pdpt_entry = pdpt_ptr[pdpt_idx];
    if (!pdpt_entry.present) return null;
    if (pdpt_entry.huge) return (@as(u64, pdpt_entry.addr) << 12) | (vaddr & 0x3FFFFFFF);
    
    // PD
    const pd_addr = @as(u64, pdpt_entry.addr) << 12;
    const pd_idx = (vaddr >> 21) & 0x1FF;
    const pd_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pd_addr));
    const pd_entry = pd_ptr[pd_idx];
    if (!pd_entry.present) return null;
    if (pd_entry.huge) return (@as(u64, pd_entry.addr) << 12) | (vaddr & 0x1FFFFF);
    
    // PT
    const pt_addr = @as(u64, pd_entry.addr) << 12;
    const pt_idx = (vaddr >> 12) & 0x1FF;
    const pt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pt_addr));
    const pt_entry = pt_ptr[pt_idx];
    if (!pt_entry.present) return null;
    
    // Return with offset
    return (@as(u64, pt_entry.addr) << 12) | (vaddr & 0xFFF);
}
```

---

## Part 4: CR3 Register (Page Directory Base Register)

### Format
```
Bit Range   Name            Meaning
---------   ----            -------
[11-0]      Reserved        Must be 0 (page alignment)
[12-51]     PDBR            Physical address of PML4 (40 bits)
[52-63]     Reserved        Must be 0
```

### Usage
```zig
pub fn write_cr3(pml4_phys: u64) void {
    // pml4_phys must be 4K-aligned (bits [0:11] = 0)
    asm volatile ("mov %%rax, %%cr3"
        :
        : [pml4] "a" (pml4_phys & 0xFFFFFFFFFFFFF000)
    );
}

pub fn read_cr3() u64 {
    var cr3: u64 = undefined;
    asm volatile ("mov %%cr3, %%rax"
        : [cr3] "=a" (cr3)
    );
    return cr3;
}
```

**Note:** Loading CR3 automatically invalidates the TLB (all entries, except those marked G).

---

## Part 5: TLB Invalidation

The Translation Lookaside Buffer (TLB) caches recent page table walks for performance.

### Methods

| Instruction | Scope | Purpose |
|-------------|-------|---------|
| `invlpg vaddr` | Single entry | Invalidate TLB entry for one virtual address |
| `mov CR3, rax` | All (current ASID) | Reload CR3 → flush entire TLB |
| `invpcid eax, ecx` | Selective (Intel) | Modern CPUs; leaf-based invalidation |

### Zig Inline Assembly
```zig
pub fn invlpg(vaddr: u64) void {
    asm volatile ("invlpg (%[vaddr])"
        :
        : [vaddr] "r" (vaddr)
        : "memory"
    );
}

pub fn flush_tlb_all() void {
    const cr3 = read_cr3();
    write_cr3(cr3);  // Reload same value → full flush
}
```

### Multicore Synchronization
For multicore systems, use Inter-Processor Interrupts (IPI) to invalidate TLB on other CPUs:
```zig
pub fn flush_tlb_shootdown(vaddr: u64) void {
    invlpg(vaddr);
    // Send IPI to other CPUs to call invlpg(vaddr)
}
```

---

## Part 6: Limine HHDM (Higher Half Direct Map)

Limine bootloader provides a virtual address mapping for **all physical RAM**.

### Concept
```
Physical Address 0x0000000000001000
         ↓ (add HHDM offset)
Virtual  Address 0xffff800000001000 (approx)
```

### Usage in Zig
```zig
var hhdm_offset: u64 = undefined;

pub fn request_hhdm() void {
    const hhdm_request = limine.hhdm_request;
    if (hhdm_request.response) |response| {
        hhdm_offset = response.offset;
    } else {
        @panic("HHDM not provided by bootloader");
    }
}

pub fn phys_to_virt(phys: u64) u64 {
    return phys + hhdm_offset;
}

pub fn virt_to_phys_hhdm(virt: u64) ?u64 {
    if (virt >= hhdm_offset) {
        return virt - hhdm_offset;
    }
    return null;
}
```

### Benefits
- Kernel can access all physical RAM without mapping each page individually
- Simplifies physical memory allocator (frame database at fixed address)
- Identity map (phys ≈ virt) can be unmapped after boot

---

## Part 7: Kernel Memory Layout

### From Limine Boot
```
Physical Range                Virtual Range
──────────────────────────────────────────────────────────────
0x0000000000001000 - ...      0x0000000000001000 - ... [Identity map, 4GB]
                              0xffffffff80000000      [Kernel loaded here]
                              hhdm_offset + 0x0       [HHDM: all phys RAM]
```

### Recommended Layout (after setup)
```
Virtual Address Space:
  0x0000000000000000 - 0x7fffffff7fffffff   [User space (128 TiB)]
  0x7fffffff80000000 - 0xffffffff7fffffff   [Unmapped / Reserved]
  0xffffffff80000000 - 0xffffffffffffffff   [Kernel space (2 MiB per module)]
    ├─ 0xffffffff80000000: Kernel code/data (from Limine)
    ├─ 0xffffffff80200000 - 0xffffffffffff7fff: Driver/module space
    └─ 0xffffffffffff8000 - 0xffffffffffffffff: HHDM (last 32 KiB high)
```

### Unmapping Identity Map (After Boot)
```zig
pub fn unmap_identity_map() void {
    const pml4_addr = read_cr3() & 0xFFFFFFFFFFFFF000;
    const pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pml4_addr));
    
    // Entries 0-3 cover 0x0-0x100000000 (4 GiB)
    for (0..4) |i| {
        pml4_ptr[i] = PageTableEntry{
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
        invlpg(@as(u64, i) << 39);
    }
}
```

---

## Part 8: Recursive Page Tables (Optional Microkernel Pattern)

### Idea
Map the PML4 into itself at entry 511, allowing modifications to any page table without special allocation tricks.

### Implementation
```zig
pub fn setup_recursive_paging(pml4_phys: u64) void {
    const pml4_addr = hhdm_offset + pml4_phys;
    const pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(pml4_addr));
    
    // Map PML4 into itself at index 511
    pml4_ptr[511] = PageTableEntry{
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
    invlpg(0xff80000000000000);  // Invalidate recursive mapping
}

// With recursive paging, any page table is accessible:
//   PML4 @  0xff80000000000000
//   PDPT @  0xff80010000000000  (for each PML4 entry)
//   PD   @  0xff80014000000000  (for each PDPT entry)
//   PT   @  0xff80014040000000  (for each PD entry)
```

### Tradeoffs
- **Pro:** No special allocation logic for modifying page tables
- **Con:** Loses 512 GiB address space (acceptable in 64-bit)

---

## Part 9: Initial Page Table Setup Sequence

### From Bootloader Entry
```zig
pub fn init_paging() void {
    // 1. Request HHDM from Limine
    request_hhdm();
    
    // 2. Read current CR3 (set by bootloader)
    const initial_pml4 = read_cr3();
    
    // 3. Unmap identity map (reclaim 4 GiB)
    unmap_identity_map();
    
    // 4. (Optional) Install recursive paging
    // setup_recursive_paging(initial_pml4 & 0xFFFFFFFFFFFFF000);
    
    // 5. Allocate and map framebuffer (from Limine framebuffer response)
    // allocate_framebuffer_pages();
    
    // 6. Allocate and map kernel heap
    // allocate_kernel_heap();
    
    // 7. Set up on-demand paging for user tasks
    // (happens later when tasks are created)
}
```

### Page Table Entry Creation
```zig
pub fn create_page_mapping(vaddr: u64, phys: u64, flags: u64) !void {
    const pml4_addr = read_cr3() & 0xFFFFFFFFFFFFF000;
    
    // Indices
    const pml4_idx = (vaddr >> 39) & 0x1FF;
    const pdpt_idx = (vaddr >> 30) & 0x1FF;
    const pd_idx   = (vaddr >> 21) & 0x1FF;
    const pt_idx   = (vaddr >> 12) & 0x1FF;
    
    // Walk / allocate tables
    const pml4_ptr = @as(*[512]PageTableEntry, @ptrFromInt(hhdm_offset + pml4_addr));
    if (!pml4_ptr[pml4_idx].present) {
        const pdpt_phys = allocate_page();
        pml4_ptr[pml4_idx] = PageTableEntry{
            .present = true,
            .writable = true,
            .user = false,
            .addr = @intCast(pdpt_phys >> 12),
            // ...other fields
        };
    }
    
    // Similar for PDPT → PD → PT
    // ...
    
    // Finally, set PT entry
    const pt_phys = @as(u64, pml4_ptr[pml4_idx].addr) << 12;
    const pdpt_ptr = @as(*[512]PageTableEntry, @ptrFromInt(hpdm_offset + pt_phys));
    // ... continue descent
    
    // Install mapping
    pt_ptr[pt_idx] = PageTableEntry{
        .present = true,
        .writable = (flags & WRITABLE) != 0,
        .user = (flags & USER) != 0,
        .write_through = (flags & WRITE_THROUGH) != 0,
        .cache_disable = (flags & CACHE_DISABLE) != 0,
        .accessed = false,
        .dirty = false,
        .huge = false,
        .global = (flags & GLOBAL) != 0,
        .avail_lo = 0,
        .addr = @intCast(phys >> 12),
        .avail_hi = 0,
        .no_execute = (flags & NO_EXECUTE) != 0,
    };
    
    invlpg(vaddr);
}
```

---

## Part 10: Quick Reference Cheat Sheet

### Extract vaddr components
```zig
pml4_idx = (vaddr >> 39) & 0x1FF;
pdpt_idx = (vaddr >> 30) & 0x1FF;
pd_idx   = (vaddr >> 21) & 0x1FF;
pt_idx   = (vaddr >> 12) & 0x1FF;
offset   = vaddr & 0xFFF;
```

### Extract phys from PTE
```zig
phys_addr = (pte & 0xFFFFFFFFFFFFF000) | (vaddr & 0xFFF);
// or with packed struct:
phys_addr = (@as(u64, pte.addr) << 12) | (vaddr & 0xFFF);
```

### Common flag combinations
```zig
const KERNEL_PAGE = PRESENT | WRITABLE | GLOBAL | NO_EXECUTE;
const KERNEL_CODE = PRESENT | GLOBAL;  // Read-only, executable
const USER_PAGE = PRESENT | WRITABLE | USER | NO_EXECUTE;
const MMIO_PAGE = PRESENT | WRITABLE | CACHE_DISABLE | WRITE_THROUGH;
```

### Page sizes
```
Small page (PT):   4 KiB   (2^12)
Huge page (PD):    2 MiB   (2^21)
Huge page (PDPT):  1 GiB   (2^30)
```

### Critical operations
```zig
write_cr3(pml4_phys);          // Load new page table root
invlpg(vaddr);                 // Invalidate one TLB entry
flush_tlb_all();               // Invalidate entire TLB
virt_to_phys(vaddr) -> ?u64;   // Walk page tables
```

---

## References
- **Limine v8 Protocol:** HHDM format, initial mappings, memory layout
- **Wikipedia: Page Table:** PTE structure, paging hierarchy, TLB
- **x86 Instruction Set:** MOV CR3, invlpg, EFER.NXE
- **Intel Manual Vol. 3:** Detailed MMU/paging specification
