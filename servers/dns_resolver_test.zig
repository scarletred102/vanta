const std = @import("std");
const net = @import("net_ethernet_arp.zig");
const dns = @import("dns_resolver.zig");

test "DNS query encodes hostname as labels for UDP port 53" {
    var query: [128]u8 = undefined;
    const len = try dns.writeAQuery(query[0..], 0x1234, "example.com");

    try std.testing.expectEqual(@as(u16, 0x1234), std.mem.readInt(u16, query[0..2], .big));
    try std.testing.expectEqual(@as(u16, 0x0100), std.mem.readInt(u16, query[2..4], .big));
    try std.testing.expectEqual(@as(u16, 1), std.mem.readInt(u16, query[4..6], .big));
    try std.testing.expectEqualSlices(u8, &.{ 7 }, query[12..13]);
    try std.testing.expectEqualSlices(u8, "example", query[13..20]);
    try std.testing.expectEqualSlices(u8, &.{ 3 }, query[20..21]);
    try std.testing.expectEqualSlices(u8, "com", query[21..24]);
    try std.testing.expectEqual(@as(u8, 0), query[24]);
    try std.testing.expect(len > 28);
}

test "DNS response parser extracts first A record" {
    const response = [_]u8{
        0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x00, 0x07, 'e',  'x',  'a',
        'm',  'p',  'l',  'e',  0x03, 'c',  'o',  'm',
        0x00, 0x00, 0x01, 0x00, 0x01, 0xc0, 0x0c, 0x00,
        0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00,
        0x04, 93,   184,  216,  34,
    };

    const ip = try dns.parseAResponse(&response, 0x1234);

    try std.testing.expectEqual(net.Ip4{ 93, 184, 216, 34 }, ip);
}
