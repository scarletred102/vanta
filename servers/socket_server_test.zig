const std = @import("std");
const net = @import("net_ethernet_arp.zig");
const socket = @import("socket_server.zig");
const tcp = @import("net_tcp.zig");

const Capture = struct {
    udp_count: usize = 0,
    tcp_count: usize = 0,
};

fn captureUdp(ctx: *anyopaque, _: net.Ip4, _: u8, _: []const u8) bool {
    const capture: *Capture = @ptrCast(@alignCast(ctx));
    capture.udp_count += 1;
    return true;
}

fn captureTcp(ctx: *anyopaque, _: tcp.Segment) bool {
    const capture: *Capture = @ptrCast(@alignCast(ctx));
    capture.tcp_count += 1;
    return true;
}

test "socket service opens binds and closes a UDP socket capability" {
    var capture = Capture{};
    var service = socket.Service.init(.{ 10, 0, 2, 15 }, captureUdp, captureTcp, &capture);

    const cap = try service.open(.Udp);
    try service.bind(cap, 5353);
    try service.close(cap);

    try std.testing.expectError(error.BadSocket, service.bind(cap, 5354));
}

test "socket service sends UDP payload through UDP stack" {
    var capture = Capture{};
    var service = socket.Service.init(.{ 10, 0, 2, 15 }, captureUdp, captureTcp, &capture);

    const cap = try service.open(.Udp);
    try service.bind(cap, 40000);
    try service.connect(cap, .{ 10, 0, 2, 2 }, 53, 1_000);
    try std.testing.expect(service.send(cap, "hello", 2_000));

    try std.testing.expectEqual(@as(usize, 1), capture.udp_count);
}

test "socket service TCP connect sends SYN and records established connection" {
    var capture = Capture{};
    var service = socket.Service.init(.{ 10, 0, 2, 15 }, captureUdp, captureTcp, &capture);

    const cap = try service.open(.Tcp);
    try service.bind(cap, 40000);
    try service.connect(cap, .{ 10, 0, 2, 2 }, 8080, 1_000);

    try std.testing.expectEqual(@as(usize, 1), capture.tcp_count);
    try std.testing.expectEqual(socket.SocketState.Connecting, service.stateOf(cap).?);
}
