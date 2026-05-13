# Limine Bootloader v8 Protocol - Complete Research Summary

## Overview
This document summarizes detailed research into the Limine bootloader v8 protocol specification, including exact struct layouts, magic numbers, request/response formats, and protocol capabilities.

**Sources:**
- https://raw.githubusercontent.com/limine-bootloader/limine/v8.x/PROTOCOL.md
- https://raw.githubusercontent.com/limine-bootloader/limine/v8.x/CONFIG.md

---

## 1. MAGIC NUMBERS & IDENTIFIERS

### Common Magic
All Limine v8 request IDs contain a common magic prefix:
```
LIMINE_COMMON_MAGIC = 0xc7b1dd30df4c8b88
```

Each request is identified by a 4-element u64 array: `[LIMINE_COMMON_MAGIC, ID_HIGH, ID_LOW, 0]`

### Complete Request Magic IDs

| Request Type | ID Array | Format |
|---|---|---|
| **Bootloader Info** | `{ 0xc7b1dd30df4c8b88, 0xf55038d8e2a1202f, 0x279426fcf5f59740, 0 }` | Standard |
| **Firmware Type** | `{ 0xc7b1dd30df4c8b88, 0x8c2f75d90bef28a8, 0x7045a4688eac00c3, 0 }` | Standard |
| **Stack Size** | `{ 0xc7b1dd30df4c8b88, 0x224ef0460a8e8926, 0xe1cb0fc25f46ea3d, 0 }` | Has stack_size field |
| **HHDM** | `{ 0xc7b1dd30df4c8b88, 0x48dcf1cb8ad2b852, 0x63984e959a98244b, 0 }` | Critical for memory access |
| **Framebuffer** | `{ 0xc7b1dd30df4c8b88, 0x9d5827dcd881dd75, 0xa3148604f6fab11b, 0 }` | Video/display |
| **Paging Mode** | `{ 0xc7b1dd30df4c8b88, 0x95c1a0edab0944cb, 0xa4e5cb3842f7488a, 0 }` | Has mode/max_mode/min_mode |
| **Multiprocessor (MP)** | `{ 0xc7b1dd30df4c8b88, 0x95a67b819a1b857e, 0xa0b61b723b6a73e0, 0 }` | CPU/AP bootstrap |
| **Memory Map** | `{ 0xc7b1dd30df4c8b88, 0x67cf3d9d378a806f, 0xe304acdfc50c3c62, 0 }` | Critical memory info |
| **Entry Point** | `{ 0xc7b1dd30df4c8b88, 0x13d86c035a1cd3e1, 0x2b0caa89d8f3026a, 0 }` | Kernel entry |
| **Executable File** | `{ 0xc7b1dd30df4c8b88, 0xad97e90e83f1ed67, 0x31eb5d1c5ff23b69, 0 }` | Kernel file info |
| **Module** | `{ 0xc7b1dd30df4c8b88, 0x3e7e279702be32af, 0xca1c4f3bd1280cee, 0 }` | Boot modules |
| **RSDP** | `{ 0xc7b1dd30df4c8b88, 0xc5e77b6b397e7b43, 0x27637845accdcf3c, 0 }` | ACPI RSDP table |
| **SMBIOS** | `{ 0xc7b1dd30df4c8b88, 0x9e9046f11e095391, 0xaa4a520fefbde5ee, 0 }` | System info |
| **EFI System Table** | `{ 0xc7b1dd30df4c8b88, 0x5ceba5163eaaf6d6, 0x0a6981610cf65fcc, 0 }` | UEFI |
| **EFI Memory Map** | `{ 0xc7b1dd30df4c8b88, 0x7df62a431d6872d5, 0xa4fcdfb3e57306c8, 0 }` | EFI mem |
| **Boot Time** | `{ 0xc7b1dd30df4c8b88, 0x502746e184c088aa, 0xfbc5ec83e6327893, 0 }` | Time/date |
| **Executable Address** | `{ 0xc7b1dd30df4c8b88, 0x71ba76863cc55f63, 0xb2644a48c516a487, 0 }` | Kernel virt/phys |
| **Device Tree Blob** | `{ 0xc7b1dd30df4c8b88, 0xb40ddb48fb54bac7, 0x545081493f81ffb7, 0 }` | Device tree |
| **RISC-V BSP Hart ID** | `{ 0xc7b1dd30df4c8b88, 0x1369359f025525f9, 0x2ff2a56178391bb6, 0 }` | RISC-V CPU |

---

## 2. CORE STRUCT DEFINITIONS (EXACT FIELD LAYOUTS)

### Memory Map Entry (24 bytes)
```c
struct limine_memmap_entry {
    uint64_t base;      // +0  Physical base address
    uint64_t length;    // +8  Size in bytes
    uint64_t type;      // +16 See MEMMAP_* constants below
};
```

**Memory Map Entry Types:**
```
LIMINE_MEMMAP_USABLE                 = 0
LIMINE_MEMMAP_RESERVED               = 1
LIMINE_MEMMAP_ACPI_RECLAIMABLE       = 2
LIMINE_MEMMAP_ACPI_NVS               = 3
LIMINE_MEMMAP_BAD_MEMORY             = 4
LIMINE_MEMMAP_BOOTLOADER_RECLAIMABLE = 5  // Can reclaim after setup
LIMINE_MEMMAP_EXECUTABLE_AND_MODULES = 6  // Kernel & modules
LIMINE_MEMMAP_FRAMEBUFFER            = 7  // Video memory
```

### Framebuffer (80+ bytes, variable)
```c
struct limine_framebuffer {
    void *address;              // +0  Physical address (use HHDM to access)
    uint64_t width;             // +8  Pixels
    uint64_t height;            // +16 Pixels
    uint64_t pitch;             // +24 Bytes per scanline
    uint16_t bpp;               // +32 Bits per pixel
    uint8_t memory_model;       // +34 1=RGB
    uint8_t red_mask_size;      // +35
    uint8_t red_mask_shift;     // +36
    uint8_t green_mask_size;    // +37
    uint8_t green_mask_shift;   // +38
    uint8_t blue_mask_size;     // +39
    uint8_t blue_mask_shift;    // +40
    uint8_t unused[7];          // +41 Padding
    uint64_t edid_size;         // +48 EDID blob size
    void *edid;                 // +56 EDID data (if present)
    
    // Response revision 1+
    uint64_t mode_count;        // +64
    struct limine_video_mode **modes;  // +72 Available video modes
};
```

### Video Mode (32 bytes)
```c
struct limine_video_mode {
    uint64_t pitch;             // +0  Bytes per scanline
    uint64_t width;             // +8  Pixels
    uint64_t height;            // +16 Pixels
    uint16_t bpp;               // +24 Bits per pixel
    uint8_t memory_model;       // +26
    uint8_t red_mask_size;      // +27
    uint8_t red_mask_shift;     // +28
    uint8_t green_mask_size;    // +29
    uint8_t green_mask_shift;   // +30
    uint8_t blue_mask_size;     // +31
    uint8_t blue_mask_shift;    // +32
};
```

### CPU Info / Multiprocessor Info (32 bytes per CPU)
```c
struct limine_mp_info {
    uint32_t processor_id;      // +0  Logical CPU ID
    uint32_t lapic_id;          // +4  Local APIC ID
    uint64_t reserved;          // +8  Reserved
    limine_goto_address goto_address;  // +16 Function pointer (void (*)(struct limine_mp_info *))
    uint64_t extra_argument;    // +24 Free-use field
};
```

---

## 3. REQUEST/RESPONSE STRUCTURES

### Standard Request Pattern
All requests follow this layout:
```c
struct limine_*_request {
    uint64_t id[4];                     // Magic ID (see above)
    uint64_t revision;                  // Request revision (usually 0)
    struct limine_*_response *response; // Pointer to response structure
    // Optional: additional fields follow
};
```

### Standard Response Pattern
```c
struct limine_*_response {
    uint64_t revision;  // Response revision (filled by bootloader)
    // Response-specific fields follow
};
```

### Key Request/Response Pairs

#### 1. BaseRevision (Entry Point)
**Request:**
```c
struct limine_entry_point_request {
    uint64_t id[4];
    uint64_t revision;
    struct limine_entry_point_response *response;
};
```
**Response:**
```c
struct limine_entry_point_response {
    uint64_t revision;
};
```
- Bootloader transfers control to kernel entry point
- Entry address set via `entry_address` in kernel ELF header

#### 2. FramebufferRequest/Response
**Request:**
```c
struct limine_framebuffer_request {
    uint64_t id[4];
    uint64_t revision;
    struct limine_framebuffer_response *response;
};
```
**Response:**
```c
struct limine_framebuffer_response {
    uint64_t revision;
    uint64_t framebuffer_count;
    struct limine_framebuffer **framebuffers;  // Array of framebuffer pointers
};
```
- Multiple framebuffers possible
- All addresses are physical (use HHDM to access)
- EDID blob included if available
- Video modes queryable (revision 1+)

#### 3. MemoryMapRequest/Response
**Request:**
```c
struct limine_memmap_request {
    uint64_t id[4];
    uint64_t revision;
    struct limine_memmap_response *response;
};
```
**Response:**
```c
struct limine_memmap_response {
    uint64_t revision;
    uint64_t entry_count;
    struct limine_memmap_entry **entries;  // Array of entry pointers
};
```
- Entries sorted by base address (lowest to highest)
- Usable and bootloader_reclaimable: 4096-byte aligned
- Base revision <= 2: 0x0000-0x1000 never marked usable

#### 4. HhdmRequest/Response
**Request:**
```c
struct limine_hhdm_request {
    uint64_t id[4];
    uint64_t revision;
    struct limine_hhdm_response *response;
};
```
**Response:**
```c
struct limine_hhdm_response {
    uint64_t revision;
    uint64_t offset;  // Virtual address offset for HHDM mapping
};
```
**CRITICAL INFORMATION:**
- HHDM offset is typically `0xffff800000000000` on x86-64
- To access physical address `phys_addr` from kernel: `phys_addr + hhdm_offset`
- All bootloader data is in HHDM space
- Framebuffers, memory maps, etc. are accessed via HHDM

#### 5. KernelAddressRequest/Response (ExecutableAddress)
**Request:**
```c
struct limine_executable_address_request {
    uint64_t id[4];
    uint64_t revision;
    struct limine_executable_address_response *response;
};
```
**Response:**
```c
struct limine_executable_address_response {
    uint64_t revision;
    uint64_t virt_base;  // Virtual address base where kernel was loaded
    uint64_t phys_base;  // Physical address base of kernel
};
```
- Used to determine kernel virtual<->physical mapping
- Both addresses provided for relocation calculations

#### 6. Multiprocessor (CPU) Request/Response
**Request:**
```c
struct limine_mp_request {
    uint64_t id[4];
    uint64_t revision;
    struct limine_mp_response *response;
    uint64_t flags;  // Bit 0: Enable X2APIC if possible (x86-64)
};
```
**Response:**
```c
struct limine_mp_response {
    uint64_t revision;
    uint32_t flags;      // Bit 0: X2APIC enabled (filled by bootloader)
    uint32_t bsp_lapic_id;
    uint64_t cpu_count;
    struct limine_mp_info **cpus;  // Array of CPU info
};
```
- Must request this feature to have bootloader bootstrap APs
- AP startup via `goto_address` function pointer in mp_info
- MTRRs automatically synchronized to match BSP

---

## 4. BASE REVISION & CAPABILITIES

### Limine v8 Supported Base Revisions

| Revision | Capabilities | Notes |
|---|---|---|
| **0** | Deprecated | Uses `.limine_reqs` section; no inline requests |
| **1** | Standard | Inline request structures; delimiters are hints only |
| **2** | Delimiter Enforcement | Request delimiters must be honored (not hints) |
| **3** | Physical Addresses | RSDP addresses guaranteed physical (not HHDM) |

### Maximum Supported Revision
- **Limine v8: Max revision = 3**
- Use `request->revision = 0` or `request->revision = 2` for compatibility
- Bootloader fills response->revision with highest supported version

### Key Differences by Revision

| Feature | Rev 0 | Rev 1 | Rev 2 | Rev 3 |
|---|---|---|---|---|
| Inline Requests | ✗ | ✓ | ✓ | ✓ |
| Request Delimiters | N/A | Hint | Mandatory | Mandatory |
| Memory 0x0-0x1000 | Never Usable | Never Usable | Never Usable | Never Usable |
| RSDP Address | N/A | HHDM | HHDM | Physical |

---

## 5. IMPORTANT PROTOCOL DETAILS

### Memory Layout
- **Lower Half:** Not supported for executables
- **Kernel Load Address:** Must be >= `0xffffffff80000000`
- **Kernel Relocation:** Not performed by bootloader; use ExecutableAddress for mapping

### HHDM (Higher Half Direct Map)
- **Purpose:** Access physical memory from kernel
- **Offset:** Retrieved from HhdmResponse
- **Typical Value (x86-64):** `0xffff800000000000`
- **Usage:** `kernel_address = physical_address + hhdm_offset`
- **Scope:** All bootloader-allocated structures (framebuffers, memory maps, etc.) are accessed via HHDM

### Memory Map Entry Semantics
- **Bootloader Reclaimable (Type 5):** Can be freed after kernel initialization
- **Executable/Modules (Type 6):** Kernel and boot modules (not marked usable)
- **Framebuffer (Type 7):** Video memory regions
- **Usable (Type 0):** Available for allocation
- **Other Types:** For reference only; use specific features to find data

### Framebuffer Behavior
- **Multiple Framebuffers:** Possible (query frame count in response)
- **Physical Addresses:** Direct physical access; add HHDM offset to use from kernel
- **EDID Data:** Included if `edid` pointer is non-NULL
- **Video Modes (Rev 1+):** List of available resolutions/modes per framebuffer
- **Memory Model:** Typically RGB (value = 1)

### CPU/Multiprocessor
- **Bootstrap:** Bootloader only starts APs if MP feature is requested
- **BSP LAPIC ID:** Provided in response for identification
- **X2APIC:** Can be enabled via flags (x86-64)
- **MTRR Synchronization:** Bootloader auto-synchronizes AP MTRRs to BSP
- **AP Startup:** Via `goto_address` function pointer with mp_info parameter

### RISC-V Support
- RISC-V Hart ID query available
- Separate MP implementation for RISC-V
- DTB (Device Tree Blob) support

### EFI/UEFI Features
- **System Table:** Physical address provided
- **Memory Map:** EFI memory map in bootloader reclaimable memory
- **ACPI:** RSDP table address (physical for base rev >= 3)

---

## 6. CHANGES FROM v7 TO v8

### Struct Compatibility
- Core request/response structures remain compatible
- New optional fields added for extended features
- Revision numbers indicate capability sets

### New/Enhanced Features in v8
1. **Better Architecture Support** - RISC-V improvements
2. **EFI Enhancements** - Memory map and system table guarantees
3. **Framebuffer Modes** - Video mode querying (revision 1+)
4. **Physical Address Guarantees** - Base revision 3 improvements

### Recommended Compatibility Approach
- Check `response->revision` after each request
- Use version-specific fields only if response revision indicates support
- Keep request `revision = 0` or `revision = 2` for widest compatibility

---

## 7. INITIALIZATION CHECKLIST

```
1. [ ] Request BaseRevision / EntryPoint
2. [ ] Request HHDM (critical for memory access)
3. [ ] Request MemoryMap (required for memory management)
4. [ ] Request Framebuffer (optional, for graphics)
5. [ ] Request ExecutableAddress (if needed for relocation)
6. [ ] Request Multiprocessor (if AP startup needed)
7. [ ] Request Modules (if using module system)
8. [ ] Request RSDP (if using ACPI)
9. [ ] Request EFI tables (if using EFI)
10. [ ] Validate all responses and use HHDM offset for data access
```

---

## 8. REFERENCE IMPLEMENTATION

See `limine_v8_protocol_reference.zig` for complete struct definitions in Zig extern struct format with exact field offsets and sizes.

---

**Research Date:** May 13, 2026
**Limine Version:** v8.x
**Documentation Source:** Official Limine GitHub Protocol Specification
