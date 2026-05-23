// ============================================================================
// VantaOS — PCI Bus Scanner
// Phase 1: Bus scanning, Mass Storage detection (AHCI / NVMe) & BAR logging.
// ============================================================================

const cpu = @import("../arch/x86_64/cpu.zig");
const serial = @import("../arch/x86_64/serial.zig");

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

/// Read a 32-bit register from the PCI configuration space.
pub fn configRead32(bus: u8, slot: u8, func: u8, offset: u8) u32 {
    const address = (@as(u32, bus) << 16) |
                    (@as(u32, slot) << 11) |
                    (@as(u32, func) << 8) |
                    (@as(u32, offset) & 0xFC) |
                    @as(u32, 0x80000000);
    cpu.outl(PCI_CONFIG_ADDRESS, address);
    return cpu.inl(PCI_CONFIG_DATA);
}

/// Initialize and scan the PCI bus for devices, logging mass storage controllers.
pub fn init() void {
    serial.puts("[PCI]   Scanning PCI bus...\n");

    var bus: u16 = 0;
    while (bus < 256) : (bus += 1) {
        var slot: u8 = 0;
        while (slot < 32) : (slot += 1) {
            // Read Function 0's vendor ID first
            const val0 = configRead32(@truncate(bus), slot, 0, 0x00);
            const vendor_id = @as(u16, @truncate(val0 & 0xFFFF));
            if (vendor_id == 0xFFFF or vendor_id == 0x0000) continue; // No device at this slot

            // Read header type to check if multi-function
            const val3 = configRead32(@truncate(bus), slot, 0, 0x0C);
            const header_type = @as(u8, @truncate((val3 >> 16) & 0xFF));
            const is_multi = (header_type & 0x80) != 0;

            const max_func: u8 = if (is_multi) 8 else 1;
            var func: u8 = 0;
            while (func < max_func) : (func += 1) {
                const val = configRead32(@truncate(bus), slot, func, 0x00);
                const func_vendor = @as(u16, @truncate(val & 0xFFFF));
                if (func_vendor == 0xFFFF or func_vendor == 0x0000) continue;

                const func_device = @as(u16, @truncate(val >> 16));

                // Read Class and Subclass
                const val2 = configRead32(@truncate(bus), slot, func, 0x08);
                const class_code = @as(u8, @truncate((val2 >> 24) & 0xFF));
                const subclass = @as(u8, @truncate((val2 >> 16) & 0xFF));

                // Standard device logging for mass storage
                if (class_code == 0x01) {
                    serial.puts("[PCI]   Found Mass Storage Controller at ");
                    serial.putDec(bus);
                    serial.puts(":");
                    serial.putDec(slot);
                    serial.puts(".");
                    serial.putDec(func);
                    serial.puts(" [Vendor: 0x");
                    serial.putHex(func_vendor);
                    serial.puts(" Device: 0x");
                    serial.putHex(func_device);
                    serial.puts("] ");

                    if (subclass == 0x06) {
                        serial.puts("(AHCI / SATA)\n");
                    } else if (subclass == 0x08) {
                        serial.puts("(NVMe Controller)\n");
                    } else {
                        serial.puts("(Other Storage, Subclass: 0x");
                        serial.putHex(subclass);
                        serial.puts(")\n");
                    }

                    // Scan BARs (only header type 0 has 6 standard BARs)
                    if ((header_type & 0x7F) == 0) {
                        var bar_idx: u8 = 0;
                        while (bar_idx < 6) : (bar_idx += 1) {
                            const bar_offset = 0x10 + bar_idx * 4;
                            const bar_val = configRead32(@truncate(bus), slot, func, bar_offset);
                            if (bar_val != 0) {
                                serial.puts("          BAR");
                                serial.putDec(bar_idx);
                                serial.puts(": 0x");
                                serial.putHex(bar_val);
                                serial.puts("\n");
                            }
                        }
                    }
                }
            }
        }
    }

    serial.puts("[PCI]   Scan complete\n");
}
