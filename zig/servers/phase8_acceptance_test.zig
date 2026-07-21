const std = @import("std");
const net = @import("net_ethernet_arp.zig");
const socket = @import("socket_server.zig");
const tcp = @import("net_tcp.zig");

const Harness = struct {
    tcp_segments: [8]tcp.Segment = undefined,
    tcp_count: usize = 0,
};

fn ignoreUdp(_: *anyopaque, _: net.Ip4, _: u8, _: []const u8) bool {
    return true;
}

fn captureTcp(ctx: *anyopaque, segment: tcp.Segment) bool {
    const harness: *Harness = @ptrCast(@alignCast(ctx));
    harness.tcp_segments[harness.tcp_count] = segment;
    harness.tcp_count += 1;
    return true;
}

test "Phase 8 TCP acceptance path sends HELLO VANTA after handshake" {
    var harness = Harness{};
    var service = socket.Service.init(.{ 10, 0, 2, 15 }, ignoreUdp, captureTcp, &harness);
    const cap = try service.open(.Tcp);
    try service.bind(cap, 40000);
    try service.connect(cap, .{ 10, 0, 2, 2 }, 8080, 1_000);

    try std.testing.expectEqual(@as(usize, 1), harness.tcp_count);
    try std.testing.expectEqual(tcp.Flags{ .syn = true }, harness.tcp_segments[0].flags);

    try std.testing.expect(service.onTcpSegment(cap, .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .src_port = 8080,
        .dst_port = 40000,
        .seq = 9000,
        .ack = 101,
        .flags = .{ .syn = true, .ack = true },
        .window = 4096,
    }, 2_000));

    try std.testing.expect(service.send(cap, "HELLO VANTA\n", 3_000));
    try std.testing.expectEqual(tcp.Flags{ .ack = true, .psh = true }, harness.tcp_segments[2].flags);
    try std.testing.expectEqualSlices(u8, "HELLO VANTA\n", harness.tcp_segments[2].payload);
}
