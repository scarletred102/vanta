// Userspace compilation entry stub for the AHCI server
comptime {
    _ = @import("servers/ahci_server.zig");
    _ = &@import("libvanta/libvanta.zig")._start;
}
pub const panic = @import("libvanta/libvanta.zig").panic;
