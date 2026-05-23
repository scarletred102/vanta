// ============================================================================
// VantaOS — Limine Boot Protocol Definitions
// Protocol reference: https://github.com/limine-bootloader/limine/blob/trunk/PROTOCOL.md
// ============================================================================

// ── Section Markers ─────────────────────────────────────────────
// These delimit the request section in the kernel binary.
// The bootloader scans between these markers for request structures.

pub const REQUESTS_START_MARKER = [_]u64{
    0xf9562b2d5c95a6c8,
    0x6a7b384944536bdc,
};

pub const REQUESTS_END_MARKER = [_]u64{
    0xadc0e0531bb10d03,
    0x9572709f31764c62,
};

// ── Common Magic ────────────────────────────────────────────────
// First two u64s of every request ID.

const COMMON_MAGIC = [2]u64{
    0xc7b1dd30df4c8b88,
    0x0a82e883a194f07b,
};

// ── Base Revision ───────────────────────────────────────────────
// Declares which protocol revision the kernel expects.
// Bootloader sets `revision` to 0 if it supports the requested revision.

pub const BaseRevision = extern struct {
    magic: [2]u64 = .{ 0xf9562b2d5c95a6c8, 0x6a7b384944536bdc },
    revision: u64 = 1, // Request revision 1 — supported by Limine v5+

    pub fn isSupported(self: *volatile @This()) bool {
        return self.revision == 0; // Bootloader clears this to 0 if supported
    }
};

// ── Framebuffer Request ─────────────────────────────────────────

pub const FramebufferRequest = extern struct {
    id: [4]u64 = COMMON_MAGIC ++ .{ 0x9d5827dcd881dd75, 0xa3148604f6fab11b },
    revision: u64 = 0,
    response: ?*volatile FramebufferResponse = null,
};

pub const FramebufferResponse = extern struct {
    revision: u64,
    framebuffer_count: u64,
    framebuffers: [*]*volatile Framebuffer,
};

pub const Framebuffer = extern struct {
    address: [*]u8,
    width: u64,
    height: u64,
    pitch: u64,
    bpp: u16,
    memory_model: u8,
    red_mask_size: u8,
    red_mask_shift: u8,
    green_mask_size: u8,
    green_mask_shift: u8,
    blue_mask_size: u8,
    blue_mask_shift: u8,
    unused: [7]u8,
    edid_size: u64,
    edid: ?[*]u8,
};

// ── Memory Map Request ──────────────────────────────────────────

pub const MemoryMapRequest = extern struct {
    id: [4]u64 = COMMON_MAGIC ++ .{ 0x67cf3d9d378a806f, 0xe304acdfc50c3c62 },
    revision: u64 = 0,
    response: ?*volatile MemoryMapResponse = null,
};

pub const MemoryMapResponse = extern struct {
    revision: u64,
    entry_count: u64,
    entries: [*]*volatile MemoryMapEntry,
};

pub const MemoryMapEntry = extern struct {
    base: u64,
    length: u64,
    kind: MemoryKind, // Named 'kind' to avoid Zig keyword 'type'
};

pub const MemoryKind = enum(u64) {
    usable = 0,
    reserved = 1,
    acpi_reclaimable = 2,
    acpi_nvs = 3,
    bad_memory = 4,
    bootloader_reclaimable = 5,
    kernel_and_modules = 6,
    framebuffer = 7,
};

// ── HHDM (Higher Half Direct Map) Request ───────────────────────

pub const HhdmRequest = extern struct {
    id: [4]u64 = COMMON_MAGIC ++ .{ 0x48dcf1cb8ad2b852, 0x63984e959a98244b },
    revision: u64 = 0,
    response: ?*volatile HhdmResponse = null,
};

pub const HhdmResponse = extern struct {
    revision: u64,
    offset: u64,
};

// ── Kernel Address Request ──────────────────────────────────────

pub const KernelAddressRequest = extern struct {
    id: [4]u64 = COMMON_MAGIC ++ .{ 0x71ba76863cc55f63, 0xb2644a48c516a487 },
    revision: u64 = 0,
    response: ?*volatile KernelAddressResponse = null,
};

pub const KernelAddressResponse = extern struct {
    revision: u64,
    physical_base: u64,
    virtual_base: u64,
};

// ── RSDP (ACPI) Request ─────────────────────────────────────────

pub const RsdpRequest = extern struct {
    id: [4]u64 = COMMON_MAGIC ++ .{ 0xc5e77b6b397e7b43, 0x27637845accdcf3c },
    revision: u64 = 0,
    response: ?*volatile RsdpResponse = null,
};

pub const RsdpResponse = extern struct {
    revision: u64,
    address: u64, // Physical address of RSDP table
};

