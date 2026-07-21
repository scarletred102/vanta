// ============================================================================
// VantaOS Userspace — AHCI Storage Server
// ============================================================================

const std = @import("std");
const libvanta = @import("../libvanta/libvanta.zig");

// Hardcoded startup capability handles
pub const PCI_CAP_HANDLE: u64 = 0x0001000000000001; // Slot 1, Gen 1 (BAR5 Memory Cap)
pub const PORT_CAP_HANDLE: u64 = 0x0001000000000002; // Slot 2, Gen 1 (Server Listener Port)
pub const REGISTRY_CAP_HANDLE: u64 = 0x0001000000000003; // Slot 3, Gen 1 (Registry Port)
pub const IRQ_CAP_HANDLE: u64 = 0x0001000000000004; // Slot 4, Gen 1 (DeviceIRQ capability)

// Virtual memory mappings
pub const AHCI_VADDR: u64 = 0x20000000;
pub const SHM_VADDR: u64 = 0x30000000;
pub const PORT_DMA_VADDR_BASE: u64 = 0x40000000;

// Message codes
pub const MSG_BLOCK_READ: u32 = 0x0401;
pub const MSG_BLOCK_WRITE: u32 = 0x0402;
pub const MSG_READ: u32 = 0x0101;
pub const MSG_WRITE: u32 = 0x0102;
pub const MSG_ERROR: u32 = 0x0003;

// AHCI Register Offsets
const GHC: usize = 0x04;
const PI: usize = 0x0C;

// Port Register Offsets (relative to port base)
const PxSSTS: usize = 0x28;
const PxSCTL: usize = 0x2C;

pub const CapEntry = struct {
    type: u4 = 0,
    rights: u8 = 0,
    generation: u16 = 1,
    kernel_object_ptr: u48 = 0,
    next_derived_table: ?*anyopaque = null,
    next_derived_index: u16 = 0,
    parent_table: ?*anyopaque = null,
    parent_index: u16 = 0,
    parent_generation: u16 = 0,
    old_table: ?*anyopaque = null,
    old_index: u16 = 0,
};

pub const Message = struct {
    msg_type: u32 = 0,
    flags: packed struct(u32) {
        expects_reply: bool = false,
        is_reply: bool = false,
        has_buffer: bool = false,
        urgent: bool = false,
        _reserved: u28 = 0,
    } = .{},
    payload: [64]u8 = [_]u8{0} ** 64,
    caps: [4]u64 = [_]u64{0} ** 4,
    buffer_cap: u64 = 0,
    transferred_caps: [4]CapEntry = [_]CapEntry{.{}} ** 4,
    transferred_buffer_cap: CapEntry = .{},
};

// Hardware DMA structures
pub const CommandHeader = extern struct {
    opts: u32,
    prdbc: u32,
    ctba: u32,
    ctbau: u32,
    reserved: [4]u32 = [_]u32{0} ** 4,
};

pub const PrdEntry = extern struct {
    dba: u32,
    dbau: u32,
    reserved: u32 = 0,
    opts: u32,
};

pub const FisRegH2D = extern struct {
    fis_type: u8 = 0x27, // Register FIS - Host to Device
    pm_port_c: u8,       // Bit 7: C = 1 (Command), bits 0-3: PM Port = 0
    command: u8,         // ATA Command (0x25 or 0x35)
    features_low: u8 = 0,
    lba0: u8,
    lba1: u8,
    lba2: u8,
    device: u8 = 1 << 6, // Bit 6: LBA mode
    lba3: u8,
    lba4: u8,
    lba5: u8,
    features_high: u8 = 0,
    count_low: u8,
    count_high: u8,
    icc: u8 = 0,
    control: u8 = 0,
    reserved: [4]u8 = [_]u8{0} ** 4,
};

pub const CommandTable = extern struct {
    cfis: [64]u8 = [_]u8{0} ** 64,
    acmd: [16]u8 = [_]u8{0} ** 16,
    reserved: [48]u8 = [_]u8{0} ** 48,
    prdt: [1]PrdEntry = undefined,
};

// Global AHCI Driver State
pub var irq_notif_handle: u64 = 0;
pub var dma_phys_addresses: [32]u64 = [_]u64{0} ** 32;
pub var active_port: ?u5 = null;

// Delay implementation using a busy-loop with pause instructions
fn delayMs(ms: u64) void {
    var i: u64 = 0;
    const count = ms * 100_000;
    while (i < count) : (i += 1) {
        asm volatile ("pause");
    }
}

// Accessor helpers
fn readReg32(offset: usize) u32 {
    const ptr: *volatile u32 = @ptrFromInt(AHCI_VADDR + offset);
    return ptr.*;
}

fn writeReg32(offset: usize, value: u32) void {
    const ptr: *volatile u32 = @ptrFromInt(AHCI_VADDR + offset);
    ptr.* = value;
}

fn getPortBase(port: u5) usize {
    return 0x100 + @as(usize, port) * 0x80;
}

fn getPortDmaVaddr(port: u5) u64 {
    return PORT_DMA_VADDR_BASE + @as(u64, port) * 4096;
}

fn issue_ata_command(port: u5, cmd: u8, lba: u64, count: u16, buf_phys: u64) !void {
    const port_base = getPortBase(port);
    const dma_vaddr = getPortDmaVaddr(port);
    const dma_phys = dma_phys_addresses[port];

    if (dma_phys == 0) return error.DmaNotConfigured;

    // Command Header is at the start of the port's DMA page
    const cmd_header = @as(*volatile CommandHeader, @ptrFromInt(dma_vaddr));
    
    // Command Table is at offset 1280 (leaves 1024 for command list and 256 for FIS)
    const cmd_table = @as(*volatile CommandTable, @ptrFromInt(dma_vaddr + 1280));
    const cmd_table_phys = dma_phys + 1280;

    // 1. Format Command Table's CFIS as a Host to Device Register FIS
    const fis = @as(*volatile FisRegH2D, @ptrCast(&cmd_table.cfis));
    fis.fis_type = 0x27; // Register FIS
    fis.pm_port_c = 0x80; // C = 1 (Command)
    fis.command = cmd; // READ DMA EXT (0x25) or WRITE DMA EXT (0x35)
    
    fis.lba0 = @truncate(lba & 0xFF);
    fis.lba1 = @truncate((lba >> 8) & 0xFF);
    fis.lba2 = @truncate((lba >> 16) & 0xFF);
    fis.device = 1 << 6; // LBA mode
    fis.lba3 = @truncate((lba >> 24) & 0xFF);
    fis.lba4 = @truncate((lba >> 32) & 0xFF);
    fis.lba5 = @truncate((lba >> 40) & 0xFF);
    
    fis.count_low = @truncate(count & 0xFF);
    fis.count_high = @truncate((count >> 8) & 0xFF);

    // 2. Build a single PRD entry pointing to our caller-provided DMA buffer
    const bytes = @as(u32, count) * 512;
    cmd_table.prdt[0] = PrdEntry{
        .dba = @truncate(buf_phys & 0xFFFFFFFF),
        .dbau = @truncate(buf_phys >> 32),
        .opts = ((bytes - 1) & 0x3FFFFF) | (1 << 31), // Count and Interrupt on Completion (IOC bit 31)
    };

    // 3. Configure Command Header
    const is_write = (cmd == 0x35);
    var opts: u32 = 5; // CFL = 5 dwords (20 bytes FIS)
    if (is_write) opts |= (1 << 6); // W (Write)
    opts |= (1 << 2); // C (Clear Busy Class)

    cmd_header.opts = opts;
    cmd_header.prdbc = 0;
    cmd_header.ctba = @truncate(cmd_table_phys & 0xFFFFFFFF);
    cmd_header.ctbau = @truncate(cmd_table_phys >> 32);

    // Clear port interrupt status
    writeReg32(port_base + 0x10, 0xFFFFFFFF); // PxIS

    // 4. Trigger the command execution (PxCI bit 0)
    writeReg32(port_base + 0x38, 1); // PxCI

    // 5. Asynchronous wait via bound DeviceIRQ + Notification
    if (irq_notif_handle != 0) {
        _ = libvanta.vanta_cap_wait(irq_notif_handle, 1);
    } else {
        // Fallback polling loop if running without interrupt bindings
        var timeout: usize = 0;
        while (timeout < 1000) : (timeout += 1) {
            const ci = readReg32(port_base + 0x38);
            if ((ci & 1) == 0) break;
            delayMs(1);
        }
    }

    // 6. Check registers for completion status
    const is_val = readReg32(port_base + 0x10); // PxIS
    const tfd = readReg32(port_base + 0x20); // PxTFD
    
    // Clear interrupt status
    writeReg32(port_base + 0x10, 0xFFFFFFFF);

    if ((is_val & (1 << 30)) != 0 or (tfd & 1) != 0) {
        return error.AtaCommandFailed;
    }
}

pub export fn main() void {
    libvanta.vanta_debug_print("AHCI: Starting standalone userspace AHCI driver server...");

    // 1. Map AHCI MMIO Registers (BAR5) via MemMap
    libvanta.vanta_debug_print("AHCI: Mapping AHCI MMIO registers...");
    // Map 1 page (4096 bytes)
    const map_err = libvanta.vanta_mem_map(PCI_CAP_HANDLE, AHCI_VADDR, 1);
    if (map_err != 0) {
        libvanta.vanta_debug_print("AHCI: Failed to map AHCI MMIO registers!");
        libvanta.vanta_exit(1);
    }
    libvanta.vanta_debug_print("AHCI: MMIO registers mapped successfully at 0x20000000");

    // 2. Enable AHCI globally (GHC.AE = 1)
    var ghc = readReg32(GHC);
    ghc |= (@as(u32, 1) << 31);
    writeReg32(GHC, ghc);
    libvanta.vanta_debug_print("AHCI: AHCI mode enabled globally in GHC");

    // 3. Port Setup & Scan
    const pi = readReg32(PI);
    var active_ports_count: u32 = 0;
    
    var port_idx: u8 = 0;
    while (port_idx < 32) : (port_idx += 1) {
        const port_idx_u5 = @as(u5, @intCast(port_idx));
        if ((pi & (@as(u32, 1) << port_idx_u5)) != 0) {
            const port_base = getPortBase(port_idx_u5);
            const ssts = readReg32(port_base + PxSSTS);
            const det = ssts & 0xF;

            if (det == 3) {
                if (active_port == null) {
                    active_port = port_idx_u5;
                }
                active_ports_count += 1;
                var dbg_buf: [64]u8 = [_]u8{0} ** 64;
                const dbg_str = std.fmt.bufPrint(&dbg_buf, "AHCI: Found drive on port {d} (SSTS=0x{x})", .{port_idx_u5, ssts}) catch unreachable;
                libvanta.vanta_debug_print(dbg_str);

                // Configure DMA Structures
                const dma_res = libvanta.vanta_mem_create(1);
                if (dma_res.err == 0) {
                    const dma_vaddr = getPortDmaVaddr(port_idx_u5);
                    const dma_map_err = libvanta.vanta_mem_map(dma_res.handle, dma_vaddr, 1);
                    if (dma_map_err == 0) {
                        const dma_phys = libvanta.vanta_mem_phys(dma_res.handle).phys;
                        dma_phys_addresses[port_idx_u5] = dma_phys;
                        
                        // Clear allocated memory to zero
                        const ptr: [*]u8 = @ptrFromInt(dma_vaddr);
                        @memset(ptr[0..4096], 0);

                        // Set PxCLB & PxFB
                        writeReg32(port_base + 0x00, @truncate(dma_phys & 0xFFFFFFFF)); // PxCLB low
                        writeReg32(port_base + 0x04, @truncate(dma_phys >> 32)); // PxCLB high
                        writeReg32(port_base + 0x08, @truncate((dma_phys + 1024) & 0xFFFFFFFF)); // PxFB low
                        writeReg32(port_base + 0x0C, @truncate((dma_phys + 1024) >> 32)); // PxFB high

                        // Start command processing (PxCMD.FRE = 1, PxCMD.ST = 1)
                        var cmd_reg = readReg32(port_base + 0x18);
                        cmd_reg |= (1 << 4); // FRE (FIS Receive Enable)
                        cmd_reg |= (1 << 0); // ST (Start)
                        writeReg32(port_base + 0x18, cmd_reg);

                        libvanta.vanta_debug_print("AHCI: Port DMA command list and FIS receive enabled.");
                    }
                }

                // Perform COMRESET
                libvanta.vanta_debug_print("AHCI: Initiating COMRESET...");
                var sctl = readReg32(port_base + PxSCTL);
                sctl = (sctl & 0xFFFFFFF0) | 1; // DET = 1 (COMRESET)
                writeReg32(port_base + PxSCTL, sctl);

                delayMs(10); // Hold reset for 10ms

                sctl &= 0xFFFFFFF0; // DET = 0 (Release COMRESET)
                writeReg32(port_base + PxSCTL, sctl);

                // Wait for link to re-establish (DET = 3)
                var timeout: usize = 0;
                var reset_success = false;
                while (timeout < 1000) : (timeout += 1) {
                    const s = readReg32(port_base + PxSSTS);
                    if ((s & 0xF) == 3) {
                        reset_success = true;
                        break;
                    }
                    delayMs(1);
                }

                if (reset_success) {
                    libvanta.vanta_debug_print("AHCI: COMRESET successful, link established.");
                } else {
                    libvanta.vanta_debug_print("AHCI: COMRESET timed out, link failed.");
                }
            }
        }
    }

    if (active_ports_count == 0) {
        libvanta.vanta_debug_print("AHCI: No active ports detected.");
    }

    // 3.5 GPT Partition Parsing
    if (active_port) |p| {
        libvanta.vanta_debug_print("AHCI: Reading MBR and GPT partition table...");
        const gpt_mem = libvanta.vanta_mem_create(1); // 1 page = 4096 bytes
        if (gpt_mem.err == 0) {
            const gpt_vaddr = 0x50000000;
            const gpt_map_err = libvanta.vanta_mem_map(gpt_mem.handle, gpt_vaddr, 1);
            if (gpt_map_err == 0) {
                const gpt_phys = libvanta.vanta_mem_phys(gpt_mem.handle).phys;
                
                // Clear page
                const gpt_ptr: [*]u8 = @ptrFromInt(gpt_vaddr);
                @memset(gpt_ptr[0..4096], 0);

                // Read LBA 0 (MBR)
                libvanta.vanta_debug_print("AHCI: Reading LBA 0 (MBR)...");
                issue_ata_command(p, 0x25, 0, 1, gpt_phys) catch |err| {
                    var err_buf: [128]u8 = [_]u8{0} ** 128;
                    const err_str = std.fmt.bufPrint(&err_buf, "AHCI: Failed to read LBA 0: {s}", .{@errorName(err)}) catch unreachable;
                    libvanta.vanta_debug_print(err_str);
                };

                // Validate Protective MBR
                if (gpt_ptr[510] == 0x55 and gpt_ptr[511] == 0xAA) {
                    libvanta.vanta_debug_print("AHCI: MBR signature 0xAA55 is valid.");
                }

                // Read LBA 1 (GPT Header) - place it at offset 512 in our page
                libvanta.vanta_debug_print("AHCI: Reading LBA 1 (GPT Header)...");
                issue_ata_command(p, 0x25, 1, 1, gpt_phys + 512) catch |err| {
                    var err_buf: [128]u8 = [_]u8{0} ** 128;
                    const err_str = std.fmt.bufPrint(&err_buf, "AHCI: Failed to read LBA 1: {s}", .{@errorName(err)}) catch unreachable;
                    libvanta.vanta_debug_print(err_str);
                };

                // Validate GPT Signature 'EFI PART'
                const sig = gpt_ptr[512..520];
                if (std.mem.eql(u8, sig, "EFI PART")) {
                    libvanta.vanta_debug_print("AHCI: GPT 'EFI PART' signature is valid!");

                    // Read partition entry details
                    const entry_lba = std.mem.readInt(u64, gpt_ptr[512 + 72 .. 512 + 80][0..8], .little);
                    const num_entries = std.mem.readInt(u32, gpt_ptr[512 + 80 .. 512 + 84][0..4], .little);
                    const entry_size = std.mem.readInt(u32, gpt_ptr[512 + 84 .. 512 + 88][0..4], .little);

                    var dbg_buf: [128]u8 = [_]u8{0} ** 128;
                    const dbg_str = std.fmt.bufPrint(&dbg_buf, "AHCI: GPT reports entries LBA={d}, count={d}, size={d}", .{entry_lba, num_entries, entry_size}) catch unreachable;
                    libvanta.vanta_debug_print(dbg_str);

                    // Read Partition Entries Page (typically LBA 2) - place it at offset 1024
                    const pages_needed = (num_entries * entry_size + 511) / 512;
                    libvanta.vanta_debug_print("AHCI: Reading GPT Partition Entry Array...");
                    issue_ata_command(p, 0x25, entry_lba, @truncate(pages_needed), gpt_phys + 1024) catch |err| {
                        var err_buf2: [128]u8 = [_]u8{0} ** 128;
                        const err_str2 = std.fmt.bufPrint(&err_buf2, "AHCI: Failed to read partition entry array: {s}", .{@errorName(err)}) catch unreachable;
                        libvanta.vanta_debug_print(err_str2);
                    };

                    // Enumerate active partitions
                    var entry_idx: usize = 0;
                    var registered_count: usize = 0;
                    while (entry_idx < num_entries) : (entry_idx += 1) {
                        const offset = 1024 + entry_idx * entry_size;
                        const type_guid = gpt_ptr[offset .. offset + 16];
                        
                        // Check if type GUID is non-zero
                        var is_zero = true;
                        for (type_guid) |b| {
                            if (b != 0) {
                                is_zero = false;
                                break;
                            }
                        }

                        if (!is_zero) {
                            const first_lba = std.mem.readInt(u64, gpt_ptr[offset + 32 .. offset + 40][0..8], .little);
                            const last_lba = std.mem.readInt(u64, gpt_ptr[offset + 40 .. offset + 48][0..8], .little);
                            
                            // Convert UTF-16LE partition name to UTF-8
                            const name_utf16 = @as([*]const u16, @ptrCast(@alignCast(&gpt_ptr[offset + 56])))[0..36];
                            var name_utf8_buf: [36]u8 = [_]u8{0} ** 36;
                            var name_len: usize = 0;
                            for (name_utf16) |c| {
                                if (c == 0) break;
                                if (c < 128) {
                                    name_utf8_buf[name_len] = @truncate(c);
                                    name_len += 1;
                                }
                            }
                            const name_utf8 = name_utf8_buf[0..name_len];

                            var dbg_buf2: [256]u8 = [_]u8{0} ** 256;
                            const dbg_str2 = std.fmt.bufPrint(&dbg_buf2, "AHCI: Found Partition '{s}' - First LBA={d}, Last LBA={d}", .{name_utf8, first_lba, last_lba}) catch unreachable;
                            libvanta.vanta_debug_print(dbg_str2);

                            // Register each partition as a derived BlockCap in the registry
                            // Format name: 'block.ahci.0p0', 'block.ahci.0p1', etc.
                            var reg_name_buf: [32]u8 = [_]u8{0} ** 32;
                            const reg_name = std.fmt.bufPrint(&reg_name_buf, "block.ahci.0p{d}", .{registered_count}) catch unreachable;

                            var part_port: u64 = 0;
                            const p_derive_err = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 3, @intFromPtr(&part_port));
                            if (p_derive_err == 0) {
                                var part_reg = Message{};
                                part_reg.msg_type = 0x10; // RegistryRegister
                                @memcpy(part_reg.payload[0..reg_name.len], reg_name);
                                part_reg.caps[0] = part_port;

                                _ = libvanta.vanta_cap_send(REGISTRY_CAP_HANDLE, @intFromPtr(&part_reg));
                                registered_count += 1;
                            }
                        }
                    }
                }
                
                // Clean up map
                _ = libvanta.vanta_mem_unmap(gpt_vaddr);
            }
            _ = libvanta.vanta_cap_revoke(gpt_mem.handle);
        }
    }

    // Setup Interrupt Notification
    libvanta.vanta_debug_print("AHCI: Setting up interrupt notification capability...");
    const notif_res = libvanta.vanta_notif_create();
    if (notif_res.err == 0) {
        irq_notif_handle = notif_res.handle;
        const bind_err = libvanta.vanta_irq_bind(IRQ_CAP_HANDLE, irq_notif_handle);
        if (bind_err == 0) {
            libvanta.vanta_debug_print("AHCI: DeviceIRQ successfully bound to Notification capability.");
        } else {
            libvanta.vanta_debug_print("AHCI: Failed to bind DeviceIRQ (or dummy mode). Using fallback timer wait.");
        }
    } else {
        libvanta.vanta_debug_print("AHCI: Failed to create Notification capability. Using fallback timer wait.");
    }

    // 4. Registry Registration
    libvanta.vanta_debug_print("AHCI: Registering with service registry...");
    var derived_port: u64 = 0;
    // Derive PORT_CAP_HANDLE with Send+Recv rights (3) so the registry can talk to us
    const derive_err = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 3, @intFromPtr(&derived_port));
    if (derive_err != 0) {
        libvanta.vanta_debug_print("AHCI: Failed to derive port capability!");
        libvanta.vanta_exit(2);
    }

    var reg_msg = Message{};
    reg_msg.msg_type = 0x10; // RegistryRegister
    @memcpy(reg_msg.payload[0..12], "block.ahci.0");
    reg_msg.caps[0] = derived_port;

    const reg_err = libvanta.vanta_cap_send(REGISTRY_CAP_HANDLE, @intFromPtr(&reg_msg));
    if (reg_err != 0) {
        libvanta.vanta_debug_print("AHCI: Registry registration failed (or registry absent). Continuing...");
    } else {
        libvanta.vanta_debug_print("AHCI: Registered as 'block.ahci.0' successfully.");
    }

    // 5. IPC Message Handling Loop
    libvanta.vanta_debug_print("AHCI: Entering IPC service loop...");
    while (true) {
        var msg = Message{};
        const recv_err = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&msg));
        if (recv_err != 0) {
            libvanta.vanta_debug_print("AHCI: Recv failed inside service loop!");
            continue;
        }

        switch (msg.msg_type) {
            MSG_BLOCK_READ, MSG_READ => {
                const lba = std.mem.readInt(u64, msg.payload[0..8], .little);
                const count = std.mem.readInt(u64, msg.payload[8..16], .little);
                const shm_cap = msg.buffer_cap;

                if (shm_cap == 0) {
                    libvanta.vanta_debug_print("AHCI: BlockRead failed: missing buffer capability!");
                    sendErrorReply(&msg);
                    continue;
                }

                // Map the shared memory to SHM_VADDR
                const pages_to_map = (count * 512 + 4095) / 4096;
                const shm_map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, pages_to_map);
                if (shm_map_err != 0) {
                    libvanta.vanta_debug_print("AHCI: BlockRead failed: failed to map buffer!");
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                // Translate virtual buffer back to physical for DMA
                const phys_res = libvanta.vanta_mem_phys(shm_cap);
                if (phys_res.err != 0) {
                    libvanta.vanta_debug_print("AHCI: BlockRead failed: failed to get physical address!");
                    _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }
                const buf_phys = phys_res.phys;

                var dbg_buf: [128]u8 = [_]u8{0} ** 128;
                const dbg_str = std.fmt.bufPrint(&dbg_buf, "AHCI: BlockRead - LBA={d}, Count={d}, shm_cap=0x{x} (phys=0x{x})", .{lba, count, shm_cap, buf_phys}) catch unreachable;
                libvanta.vanta_debug_print(dbg_str);

                const p = active_port;
                if (p) |active_p| {
                    // Issue ATA READ DMA EXT (0x25)
                    issue_ata_command(active_p, 0x25, lba, @truncate(count), buf_phys) catch |err| {
                        var err_buf: [128]u8 = [_]u8{0} ** 128;
                        const err_str = std.fmt.bufPrint(&err_buf, "AHCI: ATA READ DMA failed: {s}", .{@errorName(err)}) catch unreachable;
                        libvanta.vanta_debug_print(err_str);
                        _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                        _ = libvanta.vanta_cap_revoke(shm_cap);
                        sendErrorReply(&msg);
                        continue;
                    };
                } else {
                    // Mock Dry-Run Fallback
                    const shm_ptr: [*]u8 = @ptrFromInt(SHM_VADDR);
                    const total_bytes = count * 512;
                    var byte_idx: usize = 0;
                    while (byte_idx < total_bytes) : (byte_idx += 1) {
                        shm_ptr[byte_idx] = @truncate((lba + byte_idx) % 256);
                    }
                }

                // Clean up mapping and capability to prevent leaks
                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                // Send reply
                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = msg.msg_type;
                    reply.flags.is_reply = true;
                    @memcpy(reply.payload[0..4], "OKAY");
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },
            MSG_BLOCK_WRITE, MSG_WRITE => {
                const lba = std.mem.readInt(u64, msg.payload[0..8], .little);
                const count = std.mem.readInt(u64, msg.payload[8..16], .little);
                const shm_cap = msg.buffer_cap;

                if (shm_cap == 0) {
                    libvanta.vanta_debug_print("AHCI: BlockWrite failed: missing buffer capability!");
                    sendErrorReply(&msg);
                    continue;
                }

                // Map the shared memory to SHM_VADDR
                const pages_to_map = (count * 512 + 4095) / 4096;
                const shm_map_err = libvanta.vanta_mem_map(shm_cap, SHM_VADDR, pages_to_map);
                if (shm_map_err != 0) {
                    libvanta.vanta_debug_print("AHCI: BlockWrite failed: failed to map buffer!");
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                // Translate virtual buffer back to physical for DMA
                const phys_res = libvanta.vanta_mem_phys(shm_cap);
                if (phys_res.err != 0) {
                    libvanta.vanta_debug_print("AHCI: BlockWrite failed: failed to get physical address!");
                    _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }
                const buf_phys = phys_res.phys;

                var dbg_buf: [128]u8 = [_]u8{0} ** 128;
                const dbg_str = std.fmt.bufPrint(&dbg_buf, "AHCI: BlockWrite - LBA={d}, Count={d}, shm_cap=0x{x} (phys=0x{x})", .{lba, count, shm_cap, buf_phys}) catch unreachable;
                libvanta.vanta_debug_print(dbg_str);

                const p = active_port;
                if (p) |active_p| {
                    // Issue ATA WRITE DMA EXT (0x35)
                    issue_ata_command(active_p, 0x35, lba, @truncate(count), buf_phys) catch |err| {
                        var err_buf: [128]u8 = [_]u8{0} ** 128;
                        const err_str = std.fmt.bufPrint(&err_buf, "AHCI: ATA WRITE DMA failed: {s}", .{@errorName(err)}) catch unreachable;
                        libvanta.vanta_debug_print(err_str);
                        _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                        _ = libvanta.vanta_cap_revoke(shm_cap);
                        sendErrorReply(&msg);
                        continue;
                    };
                } else {
                    // Mock Dry-Run Fallback: log the payload start
                    const shm_ptr: [*]const u8 = @ptrFromInt(SHM_VADDR);
                    var log_buf: [64]u8 = [_]u8{0} ** 64;
                    const log_str = std.fmt.bufPrint(&log_buf, "AHCI: BlockWrite payload start: 0x{x} 0x{x} 0x{x} 0x{x}", .{shm_ptr[0], shm_ptr[1], shm_ptr[2], shm_ptr[3]}) catch unreachable;
                    libvanta.vanta_debug_print(log_str);
                }

                // Clean up mapping and capability
                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                // Send reply
                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = msg.msg_type;
                    reply.flags.is_reply = true;
                    @memcpy(reply.payload[0..4], "OKAY");
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },
            else => {
                libvanta.vanta_debug_print("AHCI: Received unknown/unsupported message type!");
                sendErrorReply(&msg);
            }
        }
    }
}

fn sendErrorReply(msg: *const Message) void {
    if (msg.flags.expects_reply) {
        var reply = Message{};
        reply.msg_type = MSG_ERROR;
        reply.flags.is_reply = true;
        @memcpy(reply.payload[0..4], "FAIL");
        _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
    }
}
