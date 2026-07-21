comptime {
    _ = @import("servers/consumer.zig");
    _ = &@import("libvanta/libvanta.zig")._start;
}
