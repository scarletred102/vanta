const std = @import("std");
const net = @import("net_ethernet_arp.zig");
const udp = @import("net_udp.zig");

const Capture = struct {
    dst_ip: net.Ip4 = .{ 0, 0, 0, 0 },
    protocol: u8 = 0,
    packet: [1600]u8 = undefined,
    len: usize = 0,
    count: usize = 0,
};

fn captureSend(ctx: *anyopaque, dst_ip: net.Ip4, protocol: u8, payload: []const u8) bool {
    const capture: *Capture = @ptrCast(@alignCast(ctx));
    capture.dst_ip = dst_ip;
    capture.protocol = protocol;
    capture.len = payload.len;
    capture.count += 1;
    @memcpy(capture.packet[0..payload.len], payload);
    return true;
}

test "UDP bind stores local port and notification handle" {
    var capture = Capture{};
    var stack = udp.Stack.init(.{ 10, 0, 2, 15 }, captureSend, &capture);

    const socket = try stack.bind(5353, 0x1111);

    try std.testing.expectEqual(@as(u16, 5353), socket.local_port);
    try std.testing.expectEqual(@as(u64, 0x1111), socket.notification_cap);
}

test "UDP sendto builds a datagram for IPv4 protocol 17" {
    var capture = Capture{};
    var stack = udp.Stack.init(.{ 10, 0, 2, 15 }, captureSend, &capture);
    const socket = try stack.bind(40000, 0x2222);

    try std.testing.expect(stack.sendTo(socket, .{ 10, 0, 2, 2 }, 53, "hello"));

    try std.testing.expectEqual(@as(usize, 1), capture.count);
    try std.testing.expectEqual(@as(u8, 17), capture.protocol);
    try std.testing.expectEqual(net.Ip4{ 10, 0, 2, 2 }, capture.dst_ip);
    const parsed = try udp.parseDatagram(capture.packet[0..capture.len], .{
        .src_ip = .{ 10, 0, 2, 15 },
        .dst_ip = .{ 10, 0, 2, 2 },
    });
    try std.testing.expectEqual(@as(u16, 40000), parsed.src_port);
    try std.testing.expectEqual(@as(u16, 53), parsed.dst_port);
    try std.testing.expectEqualSlices(u8, "hello", parsed.payload);
}

test "UDP receive queues datagram on bound socket and records wake notification" {
    var capture = Capture{};
    var stack = udp.Stack.init(.{ 10, 0, 2, 15 }, captureSend, &capture);
    const socket = try stack.bind(1234, 0x3333);
    var datagram: [12]u8 = undefined;
    udp.writeDatagram(datagram[0..], .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .src_port = 53,
        .dst_port = 1234,
    }, "pong");

    try std.testing.expect(stack.handleDatagram(.{ 10, 0, 2, 2 }, .{ 10, 0, 2, 15 }, &datagram));
    const received = stack.recvFrom(socket).?;

    try std.testing.expectEqual(@as(u64, 0x3333), stack.last_wake_notification.?);
    try std.testing.expectEqual(net.Ip4{ 10, 0, 2, 2 }, received.src_ip);
    try std.testing.expectEqual(@as(u16, 53), received.src_port);
    try std.testing.expectEqualSlices(u8, "pong", received.payload);
    try std.testing.expect(stack.recvFrom(socket) == null);
}

test "UDP parser rejects nonzero checksum mismatch" {
    var datagram: [12]u8 = undefined;
    udp.writeDatagram(datagram[0..], .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .src_port = 53,
        .dst_port = 1234,
    }, "pong");
    datagram[6] ^= 0xff;

    try std.testing.expectError(error.BadChecksum, udp.parseDatagram(&datagram, .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
    }));
}
