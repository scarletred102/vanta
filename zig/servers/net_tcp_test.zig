const std = @import("std");
const net = @import("net_ethernet_arp.zig");
const tcp = @import("net_tcp.zig");

const Capture = struct {
    segments: [8]tcp.Segment = undefined,
    count: usize = 0,
};

fn captureSend(ctx: *anyopaque, segment: tcp.Segment) bool {
    const capture: *Capture = @ptrCast(@alignCast(ctx));
    capture.segments[capture.count] = segment;
    capture.count += 1;
    return true;
}

test "TCP client path reaches established after SYN ACK" {
    var capture = Capture{};
    var conn = tcp.Connection.init(.{ 10, 0, 2, 15 }, 40000, captureSend, &capture);

    try std.testing.expect(conn.connect(.{ 10, 0, 2, 2 }, 8080, 100, 1_000));
    try std.testing.expectEqual(tcp.State.SynSent, conn.state);
    try std.testing.expectEqual(tcp.Flags{ .syn = true }, capture.segments[0].flags);

    try std.testing.expect(conn.onSegment(.{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .src_port = 8080,
        .dst_port = 40000,
        .seq = 900,
        .ack = 101,
        .flags = .{ .syn = true, .ack = true },
        .window = 4096,
    }, 2_000));

    try std.testing.expectEqual(tcp.State.Established, conn.state);
    try std.testing.expectEqual(@as(u32, 901), conn.recv_next);
    try std.testing.expectEqual(tcp.Flags{ .ack = true }, capture.segments[1].flags);
}

test "TCP server path accepts SYN then establishes on ACK" {
    var capture = Capture{};
    var listener = tcp.Connection.listen(.{ 10, 0, 2, 15 }, 80, captureSend, &capture);

    try std.testing.expect(listener.onSegment(.{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .src_port = 50000,
        .dst_port = 80,
        .seq = 44,
        .flags = .{ .syn = true },
        .window = 4096,
    }, 1_000));

    try std.testing.expectEqual(tcp.State.SynReceived, listener.state);
    try std.testing.expectEqual(tcp.Flags{ .syn = true, .ack = true }, capture.segments[0].flags);

    try std.testing.expect(listener.onSegment(.{
        .src_ip = .{ 10, 0, 2, 2 },
        .dst_ip = .{ 10, 0, 2, 15 },
        .src_port = 50000,
        .dst_port = 80,
        .seq = 45,
        .ack = listener.send_next,
        .flags = .{ .ack = true },
        .window = 4096,
    }, 2_000));

    try std.testing.expectEqual(tcp.State.Established, listener.state);
}

test "TCP retransmit timeout uses exponential backoff capped at 60 seconds" {
    var capture = Capture{};
    var conn = tcp.Connection.init(.{ 10, 0, 2, 15 }, 40000, captureSend, &capture);
    _ = conn.connect(.{ 10, 0, 2, 2 }, 8080, 100, 1_000);

    try std.testing.expect(conn.onTimer(std.time.ns_per_s + 2_000));
    try std.testing.expectEqual(@as(u64, 2 * std.time.ns_per_s), conn.rto_ns);
    try std.testing.expect(conn.onTimer(3 * std.time.ns_per_s + 2_000));
    try std.testing.expectEqual(@as(u64, 4 * std.time.ns_per_s), conn.rto_ns);

    conn.rto_ns = 60 * std.time.ns_per_s;
    try std.testing.expect(conn.onTimer(64 * std.time.ns_per_s));
    try std.testing.expectEqual(@as(u64, 60 * std.time.ns_per_s), conn.rto_ns);
}

test "TCP Reno grows congestion window and halves it on loss" {
    var capture = Capture{};
    var conn = tcp.Connection.init(.{ 10, 0, 2, 15 }, 40000, captureSend, &capture);
    conn.state = .Established;
    conn.send_unacked = 10;
    conn.send_next = 11;

    conn.onAck(11);
    try std.testing.expect(conn.cwnd_bytes > tcp.MSS);

    const before_loss = conn.cwnd_bytes;
    conn.onLoss();
    try std.testing.expectEqual(@max(tcp.MSS * 2, before_loss / 2), conn.ssthresh_bytes);
    try std.testing.expectEqual(tcp.MSS, conn.cwnd_bytes);
}
