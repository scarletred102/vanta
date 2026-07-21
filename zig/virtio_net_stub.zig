comptime {
    _ = @import("servers/virtio_net_server.zig");
    _ = &@import("libvanta/libvanta.zig")._start;
}
pub const panic = @import("libvanta/libvanta.zig").panic;
