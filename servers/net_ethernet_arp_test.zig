const std = @import("std");
const net = @import("net_ethernet_arp.zig");

const Capture = struct {
    frames: [4][1518]u8 = undefined,
    lens: [4]usize = [_]usize{0} ** 4,
    count: usize = 0,
};

fn captureSend(ctx: *anyopaque, frame: []const u8) bool {
    const capture: *Capture = @ptrCast(@alignCast(ctx));
    const slot = capture.count;
    capture.count += 1;
    @memcpy(capture.frames[slot][0..frame.len], frame);
    capture.lens[slot] = frame.len;
    return true;
}

fn stackWithCapture(capture: *Capture) net.Stack {
    return net.Stack.init(
        .{ 0x52, 0x54, 0x00, 0x0a, 0x0a, 0x01 },
        .{ 10, 0, 2, 15 },
        captureSend,
        capture,
    );
}

test "sendEthernetFrame prepends destination source and EtherType" {
    var capture = Capture{};
    var stack = stackWithCapture(&capture);

    try std.testing.expect(stack.sendEthernetFrame(
        .{ 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff },
        net.ETHERTYPE_IPV4,
        "hello",
    ));

    try std.testing.expectEqual(@as(usize, 1), capture.count);
    try std.testing.expectEqualSlices(u8, &.{ 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff }, capture.frames[0][0..6]);
    try std.testing.expectEqualSlices(u8, &.{ 0x52, 0x54, 0x00, 0x0a, 0x0a, 0x01 }, capture.frames[0][6..12]);
    try std.testing.expectEqualSlices(u8, &.{ 0x08, 0x00 }, capture.frames[0][12..14]);
    try std.testing.expectEqualSlices(u8, "hello", capture.frames[0][14..capture.lens[0]]);
}

test "ARP request for our IP sends unicast ARP reply and caches sender" {
    var capture = Capture{};
    var stack = stackWithCapture(&capture);
    const sender_mac: net.Mac = .{ 0xde, 0xad, 0xbe, 0xef, 0x00, 0x01 };
    const sender_ip: net.Ip4 = .{ 10, 0, 2, 2 };
    var frame = [_]u8{0} ** 42;

    net.writeEthernetHeader(frame[0..14], stack.mac, sender_mac, net.ETHERTYPE_ARP);
    net.writeArpPacket(frame[14..42], .{
        .op = net.ARP_OP_REQUEST,
        .sender_mac = sender_mac,
        .sender_ip = sender_ip,
        .target_mac = .{ 0, 0, 0, 0, 0, 0 },
        .target_ip = stack.ip,
    });

    try std.testing.expect(stack.handleReceivedFrame(&frame, 1_000) == null);

    try std.testing.expectEqual(sender_mac, stack.cache.get(sender_ip, 1_000).?);
    try std.testing.expectEqual(@as(usize, 1), capture.count);
    try std.testing.expectEqualSlices(u8, &sender_mac, capture.frames[0][0..6]);
    try std.testing.expectEqualSlices(u8, &.{ 0x08, 0x06 }, capture.frames[0][12..14]);
    try std.testing.expectEqualSlices(u8, &.{ 0x00, 0x02 }, capture.frames[0][20..22]);
    try std.testing.expectEqualSlices(u8, &sender_mac, capture.frames[0][32..38]);
    try std.testing.expectEqualSlices(u8, &sender_ip, capture.frames[0][38..42]);
}

test "unknown IPv4 target sends broadcast ARP request and cache expires after 60 seconds" {
    var capture = Capture{};
    var stack = stackWithCapture(&capture);
    const target_ip: net.Ip4 = .{ 10, 0, 2, 99 };
    const target_mac: net.Mac = .{ 0x10, 0x20, 0x30, 0x40, 0x50, 0x60 };

    try std.testing.expect(stack.resolveIpv4Destination(target_ip, 2_000) == null);
    try std.testing.expectEqual(@as(usize, 1), capture.count);
    try std.testing.expectEqualSlices(u8, &.{ 0xff, 0xff, 0xff, 0xff, 0xff, 0xff }, capture.frames[0][0..6]);
    try std.testing.expectEqualSlices(u8, &.{ 0x00, 0x01 }, capture.frames[0][20..22]);
    try std.testing.expectEqualSlices(u8, &target_ip, capture.frames[0][38..42]);

    stack.cache.put(target_ip, target_mac, 3_000);
    try std.testing.expectEqual(target_mac, stack.resolveIpv4Destination(target_ip, 4_000).?);
    try std.testing.expectEqual(@as(usize, 1), capture.count);

    try std.testing.expect(stack.resolveIpv4Destination(target_ip, 61 * std.time.ns_per_s) == null);
    try std.testing.expectEqual(@as(usize, 2), capture.count);
}

test "IPv4 frames return payload and IPv6 frames are dropped" {
    var capture = Capture{};
    var stack = stackWithCapture(&capture);
    var ipv4 = [_]u8{0} ** 34;
    var ipv6 = [_]u8{0} ** 34;

    net.writeEthernetHeader(ipv4[0..14], stack.mac, stack.mac, net.ETHERTYPE_IPV4);
    @memcpy(ipv4[14..34], "12345678901234567890");
    net.writeEthernetHeader(ipv6[0..14], stack.mac, stack.mac, net.ETHERTYPE_IPV6);

    try std.testing.expectEqualSlices(u8, "12345678901234567890", stack.handleReceivedFrame(&ipv4, 0).?);
    try std.testing.expect(stack.handleReceivedFrame(&ipv6, 0) == null);
}
