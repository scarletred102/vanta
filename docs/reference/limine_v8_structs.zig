//! Limine Bootloader v8 Protocol Reference
//! Complete struct definitions with exact field types and sizes
//! Source: https://raw.githubusercontent.com/limine-bootloader/limine/v8.x/PROTOCOL.md

const std = @import("std");

// ============================================================================
// MAGIC NUMBERS & CONSTANTS
// ============================================================================

/// Common magic prefix for all Limine v8 request IDs
pub const LIMINE_COMMON_MAGIC: u64 = 0xc7b1dd30df4c8b88;

// Memory map entry type constants
pub const LIMINE_MEMMAP_USABLE: u64 = 0;
pub const LIMINE_MEMMAP_RESERVED: u64 = 1;
pub const LIMINE_MEMMAP_ACPI_RECLAIMABLE: u64 = 2;
pub const LIMINE_MEMMAP_ACPI_NVS: u64 = 3;
pub const LIMINE_MEMMAP_BAD_MEMORY: u64 = 4;
pub const LIMINE_MEMMAP_BOOTLOADER_RECLAIMABLE: u64 = 5;
pub const LIMINE_MEMMAP_EXECUTABLE_AND_MODULES: u64 = 6;
pub const LIMINE_MEMMAP_FRAMEBUFFER: u64 = 7;

// Framebuffer memory models
pub const LIMINE_FRAMEBUFFER_RGB: u8 = 1;

// Multiprocessor flags
pub const LIMINE_MP_FLAGS_X2APIC_ENABLE: u64 = 1 << 0;
pub const LIMINE_MP_FLAGS_X2APIC_ENABLED: u32 = 1 << 0;

// ============================================================================
// REQUEST/RESPONSE MAGIC IDs (4-u64 arrays)
// ============================================================================

pub const REQUEST_IDS = struct {
    pub const bootloader_info = [4]u64{
        LIMINE_COMMON_MAGIC,
        0xf55038d8e2a1202f,
        0x279426fcf5f59740,
        0, // padding
    };

    pub const firmware_type = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x8c2f75d90bef28a8,
        0x7045a4688eac00c3,
        0,
    };

    pub const stack_size = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x224ef0460a8e8926,
        0xe1cb0fc25f46ea3d,
        0,
    };

    pub const hhdm = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x48dcf1cb8ad2b852,
        0x63984e959a98244b,
        0,
    };

    pub const framebuffer = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x9d5827dcd881dd75,
        0xa3148604f6fab11b,
        0,
    };

    pub const paging_mode = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x95c1a0edab0944cb,
        0xa4e5cb3842f7488a,
        0,
    };

    pub const mp = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x95a67b819a1b857e,
        0xa0b61b723b6a73e0,
        0,
    };

    pub const memmap = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x67cf3d9d378a806f,
        0xe304acdfc50c3c62,
        0,
    };

    pub const entry_point = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x13d86c035a1cd3e1,
        0x2b0caa89d8f3026a,
        0,
    };

    pub const executable_file = [4]u64{
        LIMINE_COMMON_MAGIC,
        0xad97e90e83f1ed67,
        0x31eb5d1c5ff23b69,
        0,
    };

    pub const module = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x3e7e279702be32af,
        0xca1c4f3bd1280cee,
        0,
    };

    pub const rsdp = [4]u64{
        LIMINE_COMMON_MAGIC,
        0xc5e77b6b397e7b43,
        0x27637845accdcf3c,
        0,
    };

    pub const smbios = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x9e9046f11e095391,
        0xaa4a520fefbde5ee,
        0,
    };

    pub const efi_system_table = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x5ceba5163eaaf6d6,
        0x0a6981610cf65fcc,
        0,
    };

    pub const efi_memmap = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x7df62a431d6872d5,
        0xa4fcdfb3e57306c8,
        0,
    };

    pub const boot_time = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x502746e184c088aa,
        0xfbc5ec83e6327893,
        0,
    };

    pub const executable_address = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x71ba76863cc55f63,
        0xb2644a48c516a487,
        0,
    };

    pub const dtb = [4]u64{
        LIMINE_COMMON_MAGIC,
        0xb40ddb48fb54bac7,
        0x545081493f81ffb7,
        0,
    };

    pub const riscv_bsp_hartid = [4]u64{
        LIMINE_COMMON_MAGIC,
        0x1369359f025525f9,
        0x2ff2a56178391bb6,
        0,
    };
};

// ============================================================================
// CORE STRUCTURES - EXACT FIELD LAYOUTS
// ============================================================================

/// Memory map entry - 24 bytes (3 x u64)
pub const MemMapEntry = extern struct {
    base: u64,
    length: u64,
    typ: u64, // use LIMINE_MEMMAP_* constants
};

/// Framebuffer structure - complex layout with video mode info
pub const Framebuffer = extern struct {
    address: ?*anyopaque,              // +0 (8 bytes)
    width: u64,                        // +8 (8 bytes)
    height: u64,                       // +16 (8 bytes)
    pitch: u64,                        // +24 (8 bytes)
    bpp: u16,                          // +32 (2 bytes)
    memory_model: u8,                  // +34 (1 byte)
    red_mask_size: u8,                 // +35 (1 byte)
    red_mask_shift: u8,                // +36 (1 byte)
    green_mask_size: u8,               // +37 (1 byte)
    green_mask_shift: u8,              // +38 (1 byte)
    blue_mask_size: u8,                // +39 (1 byte)
    blue_mask_shift: u8,               // +40 (1 byte)
    unused: [7]u8,                     // +41 (7 bytes)
    edid_size: u64,                    // +48 (8 bytes)
    edid: ?*anyopaque,                 // +56 (8 bytes)
    // Response revision 1+
    mode_count: u64,                   // +64 (8 bytes)
    modes: ?[*]*VideoMode,             // +72 (8 bytes)
};

/// Video mode descriptor - 32 bytes
pub const VideoMode = extern struct {
    pitch: u64,                        // +0 (8 bytes)
    width: u64,                        // +8 (8 bytes)
    height: u64,                       // +16 (8 bytes)
    bpp: u16,                          // +24 (2 bytes)
    memory_model: u8,                  // +26 (1 byte)
    red_mask_size: u8,                 // +27 (1 byte)
    red_mask_shift: u8,                // +28 (1 byte)
    green_mask_size: u8,               // +29 (1 byte)
    green_mask_shift: u8,              // +30 (1 byte)
    blue_mask_size: u8,                // +31 (1 byte)
    blue_mask_shift: u8,               // +32 (1 byte)
};

/// CPU Info structure - 32 bytes per CPU
pub const MpInfo = extern struct {
    processor_id: u32,                 // +0 (4 bytes)
    lapic_id: u32,                     // +4 (4 bytes)
    reserved: u64,                     // +8 (8 bytes)
    goto_address: ?*const fn (*MpInfo) void, // +16 (8 bytes function pointer)
    extra_argument: u64,               // +24 (8 bytes)
};

/// Module structure
pub const Module = extern struct {
    base: u64,
    size: u64,
    string: ?[*:0]const u8,
};

/// Executable file structure
pub const ExecutableFile = extern struct {
    address: u64,
    size: u64,
};

/// Internal module structure (for bootloader config)
pub const InternalModule = extern struct {
    path: ?[*:0]const u8,
    cmdline: ?[*:0]const u8,
    flags: u64,
};

// ============================================================================
// REQUEST STRUCTURES (u64 id[4] + u64 revision + *response + optional fields)
// ============================================================================

/// Base request template - all requests follow this pattern
pub const BaseRequest = extern struct {
    id: [4]u64,
    revision: u64,
    response: ?*anyopaque,
};

pub const BootloaderInfoRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*BootloaderInfoResponse = null,
};

pub const BootloaderInfoResponse = extern struct {
    revision: u64,
    name: ?[*:0]const u8,
    version: ?[*:0]const u8,
};

pub const FirmwareTypeRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*FirmwareTypeResponse = null,
};

pub const FirmwareTypeResponse = extern struct {
    revision: u64,
    firmware_type: u64,
};

pub const StackSizeRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*StackSizeResponse = null,
    stack_size: u64,
};

pub const StackSizeResponse = extern struct {
    revision: u64,
};

pub const HhdmRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*HhdmResponse = null,
};

pub const HhdmResponse = extern struct {
    revision: u64,
    offset: u64, // Virtual address offset of higher half direct map
};

pub const FramebufferRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*FramebufferResponse = null,
};

pub const FramebufferResponse = extern struct {
    revision: u64,
    framebuffer_count: u64,
    framebuffers: ?[*]*Framebuffer,
};

pub const MemMapRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*MemMapResponse = null,
};

pub const MemMapResponse = extern struct {
    revision: u64,
    entry_count: u64,
    entries: ?[*]*MemMapEntry,
};

pub const MpRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*MpResponse = null,
    flags: u64,
};

pub const MpResponse = extern struct {
    revision: u64,
    flags: u32,
    bsp_lapic_id: u32,
    cpu_count: u64,
    cpus: ?[*]*MpInfo,
};

pub const PagingModeRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*PagingModeResponse = null,
    mode: u64,
    // Request revision 1+
    max_mode: u64 = 0,
    min_mode: u64 = 0,
};

pub const PagingModeResponse = extern struct {
    revision: u64,
    mode: u64,
};

pub const ExecutableAddressRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*ExecutableAddressResponse = null,
};

pub const ExecutableAddressResponse = extern struct {
    revision: u64,
    virt_base: u64,  // Virtual address base
    phys_base: u64,  // Physical address base
};

pub const ModuleRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*ModuleResponse = null,
    // Request revision 1+
    internal_module_count: u64 = 0,
    internal_modules: ?[*]*InternalModule = null,
};

pub const ModuleResponse = extern struct {
    revision: u64,
    module_count: u64,
    modules: ?[*]*Module,
};

pub const RsdpRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*RsdpResponse = null,
};

pub const RsdpResponse = extern struct {
    revision: u64,
    address: u64, // ACPI RSDP table address (physical for base revision >= 3)
};

pub const SmbiosRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*SmbiosResponse = null,
};

pub const SmbiosResponse = extern struct {
    revision: u64,
    address: u64,
};

pub const ExecutableFileRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*ExecutableFileResponse = null,
};

pub const ExecutableFileResponse = extern struct {
    revision: u64,
    executable: ?*ExecutableFile,
};

pub const BootTimeRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*BootTimeResponse = null,
};

pub const BootTimeResponse = extern struct {
    revision: u64,
    boot_time: i64, // UNIX timestamp in milliseconds
};

pub const EfiSystemTableRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*EfiSystemTableResponse = null,
};

pub const EfiSystemTableResponse = extern struct {
    revision: u64,
    address: u64, // EFI system table address
};

pub const EfiMemMapRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*EfiMemMapResponse = null,
};

pub const EfiMemMapResponse = extern struct {
    revision: u64,
    memmap: ?*anyopaque,        // Address in HHDM (bootloader reclaimable)
    memmap_size: u64,
    desc_size: u64,
    desc_version: u64,
};

pub const DtbRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*DtbResponse = null,
};

pub const DtbResponse = extern struct {
    revision: u64,
    address: u64, // Device tree blob address
};

pub const RiscvBspHartIdRequest = extern struct {
    id: [4]u64,
    revision: u64 = 0,
    response: ?*RiscvBspHartIdResponse = null,
};

pub const RiscvBspHartIdResponse = extern struct {
    revision: u64,
    hart_id: u64,
};

// ============================================================================
// PROTOCOL INFORMATION & CAPABILITIES
// ============================================================================

/// Base Revision Capabilities
pub const BaseRevisionCapabilities = enum(u64) {
    /// Base revision 0: Deprecated, uses .limine_reqs section
    /// - No request delimiters
    /// - Memory 0-0x1000 never marked usable
    revision_0 = 0,

    /// Base revision 1: Modern standard
    /// - Support for inline request structures
    /// - Request delimiters are hints only
    /// - Memory 0-0x1000 never marked usable
    revision_1 = 1,

    /// Base revision 2: Delimiter enforcement
    /// - Request delimiters MUST be honored (not just hints)
    /// - Memory 0-0x1000 never marked usable
    revision_2 = 2,

    /// Base revision 3: Physical address guarantee
    /// - RSDP and other physical addresses guaranteed (not HHDM-mapped)
    /// - HHDM still used for bootloader data
    revision_3 = 3,
};

/// Maximum supported base revision for Limine v8
pub const MAX_BASE_REVISION = 3;

// ============================================================================
// PROTOCOL NOTES
// ============================================================================

// Key Information about Limine v8:
//
// 1. HHDM (Higher Half Direct Map):
//    - Offset is returned in HhdmResponse.offset
//    - Typical offset: 0xffff800000000000 (on x86-64)
//    - Used to access physical memory from kernel (higher half)
//    - All bootloader-allocated memory is in HHDM space
//
// 2. Memory Map:
//    - Entries are sorted by base address (lowest to highest)
//    - Usable and bootloader_reclaimable entries are 4096-byte aligned
//    - Executable and modules NOT marked as usable, but as EXECUTABLE_AND_MODULES
//    - BOOTLOADER_RECLAIMABLE can be freed after OS setup
//
// 3. Framebuffer:
//    - Addresses are physical (use HHDM offset to access from kernel)
//    - Multiple framebuffers supported (framebuffer_count)
//    - EDID blob available if edid != NULL
//    - Video modes available for framebuffers (revision 1+)
//
// 4. CPU/MP:
//    - Must request MP feature to have bootloader bootstrap APs
//    - BSP LAPIC ID provided in response
//    - X2APIC can be enabled via flags
//    - MTRRs synchronized to match BSP
//
// 5. Kernel Loading:
//    - Kernel must be loaded at or above 0xffffffff80000000
//    - Lower half executables NOT supported
//    - ExecutableAddress gives virtual<->physical mapping
//
// 6. Changes from v7 to v8:
//    - Struct layouts remain compatible where possible
//    - New request fields added for expanded capabilities
//    - Some response fields now guaranteed populated (RSDP, etc)
//    - Better architecture support (RISC-V BSP Hart ID)
