comptime {
    _ = @import("servers/ns_server.zig");
    _ = &@import("libvanta/libvanta.zig")._start;
}
pub const panic = @import("libvanta/libvanta.zig").panic;
