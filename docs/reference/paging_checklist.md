# VantaOS Paging Integration Checklist

Use this checklist to integrate paging into your Zig microkernel Phase 1.

---

## Phase 1A: Foundation (Week 1)

- [ ] **Copy reference files**
  - [ ] `PAGING_REFERENCE.md` → `/docs/PAGING_REFERENCE.md`
  - [ ] `PAGING_IMPLEMENTATION.zig` → `/kernel/arch/x86_64/paging.zig`
  - [ ] `PAGING_QUICKSTART.md` → `/docs/PAGING_QUICKSTART.md`

- [ ] **Implement CR3 register operations**
  - [ ] `read_cr3()` - inline assembly
  - [ ] `write_cr3(phys)` - inline assembly
  - [ ] `get_pml4_phys()` - extract bits [12:51]
  - [ ] Test: `write_cr3(read_cr3())` should not crash

- [ ] **Implement TLB invalidation**
  - [ ] `invlpg(vaddr)` - inline assembly
  - [ ] `flush_tlb()` - reload CR3
  - [ ] Test on a non-critical mapping (verify it works)

- [ ] **Implement HHDM integration**
  - [ ] Add Limine HHDM request struct
  - [ ] Call `init_hhdm(response.offset)` at boot
  - [ ] Implement `phys_to_virt(phys)` and `virt_to_phys_hhdm(virt)`
  - [ ] Test: Access bootloader memory via HHDM

- [ ] **Implement virtual address decomposition**
  - [ ] Create `VirtAddrDecomp` struct with 5 fields (pml4, pdpt, pd, pt, offset)
  - [ ] Implement `from_addr(vaddr)` extractor
  - [ ] Unit test: Decompose known vaddrs, verify indices

---

## Phase 1B: Page Walking (Week 2)

- [ ] **Implement page table entry struct**
  - [ ] `PageTableEntry` as packed struct(u64)
  - [ ] 13 fields: 8 bool flags + 3 u* fields + 2 avail
  - [ ] Test: sizeof(PageTableEntry) == 8

- [ ] **Implement page table walk**
  - [ ] `virt_to_phys(vaddr) -> ?u64` - walk 4 levels
  - [ ] Check P bit at each level; return null on missing page
  - [ ] Handle 2MiB huge pages (PS bit in PD)
  - [ ] Handle 1GiB huge pages (PS bit in PDPT)
  - [ ] Test on kernel-loaded regions (should map successfully)

- [ ] **Implement reverse translation**
  - [ ] `is_mapped(vaddr) -> bool` - wrapper around virt_to_phys
  - [ ] Test: Check that kernel space is mapped, HHDM is mapped, identity map is not (after unmap)

- [ ] **Verify Limine initial state**
  - [ ] Walk PML4 entry 0 (identity map) - should be present
  - [ ] Walk PML4 entry 511 (kernel) - should be present
  - [ ] Check that HHDM covers all physical RAM
  - [ ] Log: "Kernel at [vaddr], identity map present, HHDM at [offset]"

---

## Phase 1C: Unmapping & Cleanup (Week 2-3)

- [ ] **Implement identity map unmapping**
  - [ ] `unmap_identity_map()` - clear PML4 entries 0-3
  - [ ] Call `invlpg()` for each unmapped chunk
  - [ ] Test: Verify accesses to 0x0-4GB now page fault

- [ ] **Implement page unmapping**
  - [ ] `unmap_page(vaddr)` - set P bit to 0 in PT entry
  - [ ] Walk to PT level (similar to virt_to_phys)
  - [ ] Call `invlpg(vaddr)` after clearing
  - [ ] Test on non-critical mapping

- [ ] **Implement page frame database**
  - [ ] Allocate array in HHDM: `[max_phys / 4096]PageFrame`
  - [ ] Track: present, refcount, owner (for capabilities)
  - [ ] Mark bootloader regions as reserved
  - [ ] Mark kernel code/data as reserved

---

## Phase 1D: Page Allocation (Week 3-4)

- [ ] **Implement physical page allocator**
  - [ ] Buddy allocator or bitmap allocator
  - [ ] `allocate_page() -> ?u64` - returns physical address
  - [ ] `free_page(phys: u64)` - releases physical page
  - [ ] Initialize from bootloader memory map (Limine provided)
  - [ ] Test: Allocate 10 pages, verify uniqueness, free all

- [ ] **Implement page table allocation**
  - [ ] Use physical allocator: `allocate_page_table() -> !u64`
  - [ ] Zero-initialize via HHDM before returning
  - [ ] Track allocation in page frame database
  - [ ] Test: Allocate, map via HHDM, verify zero

- [ ] **Implement dynamic page mapping**
  - [ ] `map_page(allocator, vaddr, phys, flags) -> !void`
  - [ ] Walk 4 levels; allocate missing intermediate tables
  - [ ] Call `invlpg(vaddr)` at end
  - [ ] Test: Map kernel heap region (0xffffffff_80100000+)

---

## Phase 1E: Kernel Memory Setup (Week 4)

- [ ] **Set up kernel heap**
  - [ ] Allocate initial heap region (e.g., 1 MiB)
  - [ ] Map via `map_page()` with WRITABLE | GLOBAL flags
  - [ ] Implement slab/buddy allocator using heap
  - [ ] Test: `allocator.alloc(u8, 1024)` returns valid pointer

- [ ] **Map kernel stack**
  - [ ] Allocate initial kernel stack (e.g., 64 KiB)
  - [ ] Map at top of kernel space (0xffffffff_fffff000 - stack_size)
  - [ ] Set up RSP to point to stack top
  - [ ] Test: Recursion doesn't immediately overflow

- [ ] **Map framebuffer (if graphical)**
  - [ ] Get framebuffer info from Limine response
  - [ ] Calculate number of pages needed
  - [ ] Map with CACHE_DISABLE | WRITE_THROUGH | GLOBAL
  - [ ] Test: Write pixel, verify on display (if running on real hardware)

- [ ] **Set up HHDM allocator state**
  - [ ] Page frame database fully initialized
  - [ ] Track kernel's own pages
  - [ ] Prepare for userspace page table allocation

---

## Phase 2A: Userspace Isolation (After Phase 1)

- [ ] **Implement new page table creation**
  - [ ] `create_address_space() -> !u64` - allocate new PML4
  - [ ] Map HHDM at same offset in new PML4 (shared, kernel-only)
  - [ ] Map kernel code/data at same VAs (copy PML4[256-511])
  - [ ] Map userspace regions separately (PML4[0-255])
  - [ ] Test: Switch contexts, verify isolation

- [ ] **Implement context switching**
  - [ ] `switch_address_space(pml4_phys)` - call `write_cr3()`
  - [ ] Save/restore segment registers if needed
  - [ ] Test on scheduler (swap between two tasks)

- [ ] **Implement capability-based access control**
  - [ ] Page frame database tracks owner (capability)
  - [ ] `map_page()` checks ownership before mapping
  - [ ] Capability revocation revokes mappings
  - [ ] Test: Deny cross-capability mapping

---

## Phase 2B: Advanced Features (Optional)

- [ ] **Implement page fault handler**
  - [ ] Catch #PF exception
  - [ ] Log fault address, error code
  - [ ] Demand paging: allocate on first access
  - [ ] Copy-on-write: duplicate on write to shared page

- [ ] **Implement huge page support**
  - [ ] `map_huge_2m(vaddr, phys, flags)` - set PS bit in PD
  - [ ] `map_huge_1g(vaddr, phys, flags)` - set PS bit in PDPT
  - [ ] Test on kernel regions (faster translation)

- [ ] **Implement recursive paging**
  - [ ] `setup_recursive_paging()` - map PML4[511] to self
  - [ ] Accessible PT addresses: `0xff80_0140_4000_0000 + idx`
  - [ ] Test: Modify PT entries without special logic

- [ ] **Implement memory statistics**
  - [ ] Track allocated, free, fragmentation
  - [ ] Report per-task memory usage
  - [ ] Detect memory leaks (reserved but unreferenced frames)

---

## Testing & Validation

### Unit Tests (Phase 1)
```
✓ CR3 read/write
✓ invlpg executes without fault
✓ HHDM offset retrieval
✓ Virtual address decomposition
✓ Page table walk (hit, miss, huge pages)
✓ Identity map unmapping
✓ Page frame allocator
✓ Page table allocation
✓ Single-page mapping
✓ Multi-level table creation
```

### Integration Tests (Phase 1)
```
✓ Kernel boots and maps memory
✓ HHDM is accessible
✓ Identity map is unmapped
✓ Heap allocator works
✓ Framebuffer (if graphical) is accessible
✓ No spurious page faults
```

### Stress Tests (Phase 2)
```
✓ 1000+ allocations without fragmentation
✓ Task context switches don't corrupt mappings
✓ Concurrent allocations (if SMP)
✓ Revoke capability → pages inaccessible
✓ Memory stats accurate
```

---

## Debugging Checklist

If paging breaks:

- [ ] Check CR3 is 4K-aligned (bits [0:11] = 0)
- [ ] Verify HHDM offset is correct (run Limine protocol check)
- [ ] Ensure page table entries are 8-byte aligned
- [ ] Confirm bit 63 (NX) is set/clear as intended
- [ ] Log page walk: print each level's entry value
- [ ] Check for canonical address violations (bits [63:48])
- [ ] Verify allocator doesn't return same page twice
- [ ] Confirm all allocated tables are zero-initialized
- [ ] Use `virt_to_phys()` to verify mappings
- [ ] Check hardware returns correct A/D bits (optional instrumentation)

---

## Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Page walk latency | <1 µs | Typical; TLB hit is 1-10 ns |
| Allocation latency | <10 µs | Buddy allocator typical |
| Context switch TLB cost | <1 ms | Depends on working set |
| Fragmentation | <10% | After 1000+ allocs/frees |
| Memory overhead | <5% | Page frame DB + allocator |

---

## Sign-Off Criteria (Phase 1 Complete)

- [ ] All Phase 1A-1E items checked
- [ ] Kernel boots without page faults
- [ ] Heap allocator functional
- [ ] Virtual memory isolation works (test task gets crashed when accessing kernel)
- [ ] Code reviewed for alignment, flag handling, off-by-one errors
- [ ] Documentation matches implementation
- [ ] No TODO/FIXME in paging code (or tracked as Phase 2)

---

## Timeline Estimate

| Phase | Duration | Checkpoints |
|-------|----------|-------------|
| 1A    | 3-4 days | CR3, invlpg, HHDM working |
| 1B    | 4-5 days | Page walk + reverse translation |
| 1C    | 3-4 days | Unmapping, page frame DB |
| 1D    | 5-6 days | Allocators + dynamic mapping |
| 1E    | 4-5 days | Kernel heap, stack, framebuffer |
| **Total** | **3-4 weeks** | Microkernel with working VM |

**Critical Path:** HHDM → page walk → allocator → dynamic mapping. Other items can parallelize.

---

## Resources

- **Code:** `PAGING_IMPLEMENTATION.zig` (copy-paste ready)
- **Spec:** `PAGING_REFERENCE.md` (10 detailed sections)
- **Guide:** `PAGING_QUICKSTART.md` (action steps)
- **Limine:** bootloader documentation for protocol details
- **Test Suite:** Write `tests/paging_test.zig` alongside implementation

---

## Questions to Ask Yourself

1. **Does my physical allocator prevent double-allocation?** (use refcount in page frame DB)
2. **Are all page table entries 8-byte aligned?** (packed struct should enforce)
3. **Do I invalidate TLB after every mapping change?** (easy to forget; add assertions)
4. **Can I handle 2MiB and 1GiB pages?** (important for performance later)
5. **Is HHDM accessible from all tasks?** (kernel-only; verify U/S bits)
6. **Does my capability model prevent unauthorized mapping?** (check page frame owner)

---

Good luck! Paging is foundational; get it right and the rest is easier.
