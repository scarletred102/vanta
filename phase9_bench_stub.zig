comptime {
    _ = @import("servers/phase9_latency_bench.zig");
    _ = &@import("libvanta/libvanta.zig")._start;
}
pub const panic = @import("libvanta/libvanta.zig").panic;
