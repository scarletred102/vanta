const std = @import("std");
const net = @import("net_ethernet_arp.zig");
const ip = @import("net_ipv4_icmp.zig");

const Capture = struct {
    packets: [4][1600]u8 = undefined,
    lens: [4]usize = [_]usize{0} ** 4,
    dst_ips: [4]net.Ip4 = [_]net.Ip4{.{ 0, 0, 0, 0 }} ** 4,
    count: usize = 0,
};

fn captureSend(ctx: *anyopaque, dst_ip: net.Ip4, packet: []const u8) bool {
    const capture: *Capture = @ptrCast(@alignCast(ctx));
    const slot = capture.count;
    capture.count += 1;
    capture.dst_ips[slot] = dst_ip;
    capture.lens[slot] = packet.len;
    @memcpy(capture.packets[slot][0..packet.len], packet);
    return true;
}

fn stackWithCapture(capture: *Capture) ip.Stack {
    return ip.Stack.init(.{ 10, 0, 2, 15 }, captureSend, capture);
}

test "IPv4 parser rejects packets with bad header checksum" {
    var capture = Capture{};
    var stack = stackWithCapture(&capture);
    var packet: [28]u8 = undefined;
    ip.writeIpv4Packet(packet[0..], .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .protocol = ip.PROTO_ICMP,
        .identification = 7,
    }, "payload!");
    packet[10] ^= 0xff;

    try std.testing.expect(stack.handleIpv4Packet(&packet, 1_000) == null);
    try std.testing.expectEqual(@as(usize, 0), capture.count);
}

test "ICMP echo request to our IP sends echo reply with swapped IPv4 addresses" {
    var capture = Capture{};
    var stack = stackWithCapture(&capture);
    var icmp_payload: [16]u8 = undefined;
    ip.writeIcmpEcho(icmp_payload[0..], .request, 0x1234, 5, "data1234");
    var packet: [36]u8 = undefined;
    ip.writeIpv4Packet(packet[0..], .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .protocol = ip.PROTO_ICMP,
        .identification = 9,
    }, &icmp_payload);

    try std.testing.expect(stack.handleIpv4Packet(&packet, 2_000) == null);

    try std.testing.expectEqual(@as(usize, 1), capture.count);
    try std.testing.expectEqual(net.Ip4{ 10, 0, 2, 2 }, capture.dst_ips[0]);
    const reply = capture.packets[0][0..capture.lens[0]];
    const parsed = try ip.parseIpv4Packet(reply);
    try std.testing.expectEqual(net.Ip4{ 10, 0, 2, 15 }, parsed.src_ip);
    try std.testing.expectEqual(net.Ip4{ 10, 0, 2, 2 }, parsed.dst_ip);
    try std.testing.expectEqual(ip.PROTO_ICMP, parsed.protocol);
    try std.testing.expectEqual(@as(u8, 0), parsed.payload[0]);
    try std.testing.expect(ip.verifyChecksum(parsed.payload));
    try std.testing.expectEqualSlices(u8, "data1234", parsed.payload[8..16]);
}

test "IPv4 fragments reassemble by source IP and identification" {
    var capture = Capture{};
    var stack = stackWithCapture(&capture);
    var first: [28]u8 = undefined;
    var second: [28]u8 = undefined;

    ip.writeIpv4Packet(first[0..], .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .protocol = 17,
        .identification = 0xbeef,
        .more_fragments = true,
        .fragment_offset = 0,
    }, "ABCDEFGH");
    ip.writeIpv4Packet(second[0..], .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .protocol = 17,
        .identification = 0xbeef,
        .fragment_offset = 1,
    }, "IJKLMNOP");

    try std.testing.expect(stack.handleIpv4Packet(&second, 1_000) == null);
    const datagram = stack.handleIpv4Packet(&first, 2_000).?;

    try std.testing.expectEqual(net.Ip4{ 10, 0, 2, 2 }, datagram.src_ip);
    try std.testing.expectEqual(@as(u8, 17), datagram.protocol);
    try std.testing.expectEqualSlices(u8, "ABCDEFGHIJKLMNOP", datagram.payload);
}

test "stale IPv4 fragments are discarded after 30 seconds" {
    var capture = Capture{};
    var stack = stackWithCapture(&capture);
    var first: [28]u8 = undefined;
    var second: [28]u8 = undefined;

    ip.writeIpv4Packet(first[0..], .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .protocol = 17,
        .identification = 0x55aa,
        .more_fragments = true,
        .fragment_offset = 0,
    }, "ABCDEFGH");
    ip.writeIpv4Packet(second[0..], .{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .protocol = 17,
        .identification = 0x55aa,
        .fragment_offset = 1,
    }, "IJKLMNOP");

    try std.testing.expect(stack.handleIpv4Packet(&first, 1_000) == null);
    try std.testing.expect(stack.handleIpv4Packet(&second, 31 * std.time.ns_per_s) == null);
}
