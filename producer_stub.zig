comptime {
    _ = @import("servers/producer.zig");
    _ = &@import("libvanta/libvanta.zig")._start;
}
