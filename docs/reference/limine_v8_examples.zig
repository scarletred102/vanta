//! Limine v8 Protocol - Practical Code Examples
//! Demonstrates how to use the Limine bootloader protocol in Zig

const std = @import("std");
const limine = @import("limine_v8_protocol_reference");

// ============================================================================
// EXAMPLE 1: Basic Setup - Request HHDM and Memory Map
// ============================================================================

pub var hhdm_req: limine.HhdmRequest = .{
    .id = limine.REQUEST_IDS.hhdm,
    .revision = 0,
    .response = null,
};

pub var memmap_req: limine.MemMapRequest = .{
    .id = limine.REQUEST_IDS.memmap,
    .revision = 0,
    .response = null,
};

pub fn setup_memory() !struct {
    hhdm_offset: u64,
    entries: []*limine.MemMapEntry,
} {
    const hhdm_resp = hhdm_req.response orelse return error.HhdmNotProvided;
    const memmap_resp = memmap_req.response orelse return error.MemmapNotProvided;

    const hhdm_offset = hhdm_resp.offset;

    // Convert bootloader-space pointers to kernel-accessible addresses
    var entries = std.ArrayList(*limine.MemMapEntry).init(std.heap.page_allocator);
    defer entries.deinit();

    for (0..memmap_resp.entry_count) |i| {
        const entry_ptr = @intToPtr(*limine.MemMapEntry,
            @ptrToInt(memmap_resp.entries[i]) + hhdm_offset);
        try entries.append(entry_ptr);
    }

    return .{
        .hhdm_offset = hhdm_offset,
        .entries = entries.items,
    };
}

// ============================================================================
// EXAMPLE 2: Parse Memory Map - Find Usable RAM
// ============================================================================

pub fn find_usable_memory(memmap: []*limine.MemMapEntry, hhdm_offset: u64) struct {
    total_usable: u64,
    entries: std.ArrayList(struct { base: u64, size: u64 }),
} {
    var usable = std.ArrayList(struct { base: u64, size: u64 }).init(
        std.heap.page_allocator);

    var total_usable: u64 = 0;

    for (memmap) |entry| {
        // Access entry via bootloader space (already adjusted by setup_memory)
        if (entry.typ == limine.LIMINE_MEMMAP_USABLE) {
            usable.append(.{
                .base = entry.base,
                .size = entry.length,
            }) catch unreachable;
            total_usable += entry.length;
        }
    }

    return .{
        .total_usable = total_usable,
        .entries = usable,
    };
}

// ============================================================================
// EXAMPLE 3: Framebuffer Setup - Get Video Display
// ============================================================================

pub var framebuffer_req: limine.FramebufferRequest = .{
    .id = limine.REQUEST_IDS.framebuffer,
    .revision = 0,
    .response = null,
};

pub fn setup_framebuffer(hhdm_offset: u64) !?*limine.Framebuffer {
    const fb_resp = framebuffer_req.response orelse return null;

    if (fb_resp.framebuffer_count == 0) {
        return null;
    }

    // First framebuffer
    const fb_ptr = @intToPtr(*limine.Framebuffer,
        @ptrToInt(fb_resp.framebuffers[0]) + hhdm_offset);

    // Now fb_ptr can be used from kernel code
    // framebuffer pixels at fb_ptr.address + hhdm_offset

    return fb_ptr;
}

// ============================================================================
// EXAMPLE 4: CPU/Multiprocessor - Start Additional Processors
// ============================================================================

pub var mp_req: limine.MpRequest = .{
    .id = limine.REQUEST_IDS.mp,
    .revision = 0,
    .response = null,
    .flags = limine.LIMINE_MP_FLAGS_X2APIC_ENABLE, // Try to enable X2APIC
};

pub fn cpu_startup() !struct {
    cpu_count: u64,
    bsp_lapic_id: u32,
    x2apic_enabled: bool,
} {
    const mp_resp = mp_req.response orelse return error.MpNotSupported;

    return .{
        .cpu_count = mp_resp.cpu_count,
        .bsp_lapic_id = mp_resp.bsp_lapic_id,
        .x2apic_enabled = (mp_resp.flags & limine.LIMINE_MP_FLAGS_X2APIC_ENABLED) != 0,
    };
}

pub fn startup_ap(cpu_info: *limine.MpInfo) void {
    // AP entry point - called by bootloader for each AP
    // The goto_address field points here

    // Example AP initialization
    const lapic_id = cpu_info.lapic_id;
    const processor_id = cpu_info.processor_id;
    const extra = cpu_info.extra_argument;

    _ = lapic_id;
    _ = processor_id;
    _ = extra;

    // Set up AP GDT, IDT, etc.
    // Then loop/sleep or jump to main kernel code
}

// ============================================================================
// EXAMPLE 5: Get Kernel Virtual Address Mapping
// ============================================================================

pub var exec_addr_req: limine.ExecutableAddressRequest = .{
    .id = limine.REQUEST_IDS.executable_address,
    .revision = 0,
    .response = null,
};

pub fn get_kernel_addresses() !struct {
    virt_base: u64,
    phys_base: u64,
} {
    const resp = exec_addr_req.response orelse return error.ExecutableAddressNotProvided;
    return .{
        .virt_base = resp.virt_base,
        .phys_base = resp.phys_base,
    };
}

// ============================================================================
// EXAMPLE 6: Modules - Load Boot Modules
// ============================================================================

pub var module_req: limine.ModuleRequest = .{
    .id = limine.REQUEST_IDS.module,
    .revision = 0,
    .response = null,
    .internal_module_count = 0,
    .internal_modules = null,
};

pub fn load_modules(hhdm_offset: u64) ![]struct {
    address: u64,
    size: u64,
    name: []const u8,
} {
    const mod_resp = module_req.response orelse return error.ModulesNotSupported;

    var modules = std.ArrayList(struct {
        address: u64,
        size: u64,
        name: []const u8,
    }).init(std.heap.page_allocator);

    for (0..mod_resp.module_count) |i| {
        const mod_ptr = @intToPtr(*limine.Module,
            @ptrToInt(mod_resp.modules[i]) + hhdm_offset);

        const name = if (mod_ptr.string) |str_ptr| name: {
            var len: usize = 0;
            while (str_ptr[len] != 0) : (len += 1) {}
            break :name str_ptr[0..len];
        } else "unknown";

        try modules.append(.{
            .address = mod_ptr.base,
            .size = mod_ptr.size,
            .name = name,
        });
    }

    return modules.items;
}

// ============================================================================
// EXAMPLE 7: ACPI/RSDP - Get ACPI Root System Description Pointer
// ============================================================================

pub var rsdp_req: limine.RsdpRequest = .{
    .id = limine.REQUEST_IDS.rsdp,
    .revision = 0,
    .response = null,
};

pub fn get_rsdp(base_revision: u64, hhdm_offset: u64) !u64 {
    const rsdp_resp = rsdp_req.response orelse return error.RsdpNotAvailable;

    // For base revision >= 3, address is physical
    // For base revision < 3, may need HHDM adjustment
    if (base_revision >= 3) {
        return rsdp_resp.address; // Physical address
    } else {
        // For older revisions, might need HHDM offset
        // (depends on bootloader implementation)
        return rsdp_resp.address + hhdm_offset;
    }
}

// ============================================================================
// EXAMPLE 8: Bootloader Info - Get Bootloader Name/Version
// ============================================================================

pub var bootloader_info_req: limine.BootloaderInfoRequest = .{
    .id = limine.REQUEST_IDS.bootloader_info,
    .revision = 0,
    .response = null,
};

pub fn get_bootloader_info(hhdm_offset: u64) !struct {
    name: []const u8,
    version: []const u8,
} {
    const info_resp = bootloader_info_req.response orelse
        return error.BootloaderInfoNotAvailable;

    const name = if (info_resp.name) |name_ptr| n: {
        const adjusted = @intToPtr([*:0]const u8,
            @ptrToInt(name_ptr) + hhdm_offset);
        var len: usize = 0;
        while (adjusted[len] != 0) : (len += 1) {}
        break :n adjusted[0..len];
    } else "unknown";

    const version = if (info_resp.version) |ver_ptr| v: {
        const adjusted = @intToPtr([*:0]const u8,
            @ptrToInt(ver_ptr) + hhdm_offset);
        var len: usize = 0;
        while (adjusted[len] != 0) : (len += 1) {}
        break :v adjusted[0..len];
    } else "unknown";

    return .{ .name = name, .version = version };
}

// ============================================================================
// EXAMPLE 9: Complete Kernel Entry - Full Initialization
// ============================================================================

pub var entry_point_req: limine.EntryPointRequest = .{
    .id = limine.REQUEST_IDS.entry_point,
    .revision = 0,
    .response = null,
};

pub fn kernel_main() noreturn {
    // Step 1: Get HHDM offset (CRITICAL - needed for all other access)
    const hhdm_resp = hhdm_req.response orelse @panic("No HHDM response");
    const hhdm_offset = hhdm_resp.offset;

    // Step 2: Get memory map
    const memmap_resp = memmap_req.response orelse @panic("No memmap response");

    // Step 3: Parse memory
    var total_memory: u64 = 0;
    var usable_memory: u64 = 0;

    for (0..memmap_resp.entry_count) |i| {
        const entry_ptr = @intToPtr(*limine.MemMapEntry,
            @ptrToInt(memmap_resp.entries[i]) + hhdm_offset);

        total_memory += entry_ptr.length;
        if (entry_ptr.typ == limine.LIMINE_MEMMAP_USABLE) {
            usable_memory += entry_ptr.length;
        }
    }

    // Step 4: Get CPU info
    const cpu_count = if (mp_req.response) |mp_resp|
        mp_resp.cpu_count
    else
        1;

    // Step 5: Get framebuffer for debug output
    const fb = setup_framebuffer(hhdm_offset) catch |e| b: {
        _ = e;
        break :b null;
    };

    // Step 6: Get bootloader info
    const bootloader_info = get_bootloader_info(hhdm_offset) catch |e| bi: {
        _ = e;
        break :bi .{ .name = "unknown", .version = "unknown" };
    };

    // Now kernel is fully initialized with:
    // - HHDM offset for physical memory access
    // - Memory map with usable regions
    // - CPU count and BSP LAPIC ID
    // - Framebuffer for display
    // - Bootloader identification

    // TODO: Initialize kernel subsystems (paging, interrupts, etc)
    // TODO: Call main kernel code

    @panic("kernel_main not fully implemented");
}

// ============================================================================
// HELPER: Access Bootloader-Provided Data Via HHDM
// ============================================================================

pub fn hhdm_access(comptime T: type, bootloader_ptr: *T, hhdm_offset: u64) *T {
    // Convert bootloader-space pointer to kernel-accessible address
    const adjusted_addr = @ptrToInt(bootloader_ptr) + hhdm_offset;
    return @intToPtr(*T, adjusted_addr);
}

// Usage:
//   const entry = hhdm_access(limine.MemMapEntry, memmap_resp.entries[0], hhdm_offset);
//   std.debug.print("Entry: base=0x{x}, size=0x{x}\n", .{entry.base, entry.length});

// ============================================================================
// HELPER: Physical to Virtual Address
// ============================================================================

pub fn phys_to_virt(phys: u64, hhdm_offset: u64) u64 {
    return phys + hhdm_offset;
}

pub fn virt_to_phys(virt: u64, hhdm_offset: u64) u64 {
    return virt - hhdm_offset;
}

// ============================================================================
// NOTE: CRITICAL PROTOCOL RULES
// ============================================================================

// 1. ALL bootloader-provided data is in bootloader address space
// 2. To access from kernel: address + hhdm_offset
// 3. Framebuffer pixels are physical: fb.address + hhdm_offset
// 4. Memory map entries: memmap_entries[i] + hhdm_offset
// 5. Any pointers in bootloader structures: +hhdm_offset
// 6. NEVER access bootloader data without HHDM offset - will fault/corrupt
// 7. Request HHDM FIRST before accessing any other data
// 8. All request->revision = 0 for compatibility
// 9. Check response->revision to determine capability level
// 10. Base revision >= 3: RSDP/EFI addresses are physical (not HHDM)
