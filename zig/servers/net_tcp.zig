const std = @import("std");
const eth = @import("net_ethernet_arp.zig");

pub const MSS: usize = 1460;

pub const State = enum {
    Closed,
    Listen,
    SynSent,
    SynReceived,
    Established,
    FinWait1,
    FinWait2,
    TimeWait,
};

pub const Flags = packed struct {
    fin: bool = false,
    syn: bool = false,
    rst: bool = false,
    psh: bool = false,
    ack: bool = false,
    urg: bool = false,
};

pub const Segment = struct {
    src_ip: eth.Ip4,
    dst_ip: eth.Ip4,
    src_port: u16,
    dst_port: u16,
    seq: u32 = 0,
    ack: u32 = 0,
    flags: Flags = .{},
    window: u16 = 65535,
    payload: []const u8 = "",
};

pub const SendSegmentFn = *const fn (ctx: *anyopaque, segment: Segment) bool;

pub const Connection = struct {
    state: State = .Closed,
    local_ip: eth.Ip4,
    remote_ip: eth.Ip4 = .{ 0, 0, 0, 0 },
    local_port: u16,
    remote_port: u16 = 0,
    send_unacked: u32 = 0,
    send_next: u32 = 0,
    recv_next: u32 = 0,
    remote_window: u16 = 0,
    rto_ns: u64 = std.time.ns_per_s,
    retransmit_at_ns: u64 = 0,
    cwnd_bytes: usize = MSS,
    ssthresh_bytes: usize = 64 * MSS,
    nagle_enabled: bool = true,
    last_unacked: ?Segment = null,
    send_fn: SendSegmentFn,
    send_ctx: *anyopaque,

    pub fn init(local_ip: eth.Ip4, local_port: u16, send_fn: SendSegmentFn, send_ctx: *anyopaque) Connection {
        return .{
            .local_ip = local_ip,
            .local_port = local_port,
            .send_fn = send_fn,
            .send_ctx = send_ctx,
        };
    }

    pub fn listen(local_ip: eth.Ip4, local_port: u16, send_fn: SendSegmentFn, send_ctx: *anyopaque) Connection {
        var conn = init(local_ip, local_port, send_fn, send_ctx);
        conn.state = .Listen;
        return conn;
    }

    pub fn connect(self: *Connection, remote_ip: eth.Ip4, remote_port: u16, initial_seq: u32, now_ns: u64) bool {
        if (self.state != .Closed) return false;
        self.remote_ip = remote_ip;
        self.remote_port = remote_port;
        self.send_unacked = initial_seq;
        self.send_next = initial_seq + 1;
        self.state = .SynSent;
        return self.sendTracked(.{
            .src_ip = self.local_ip,
            .dst_ip = self.remote_ip,
            .src_port = self.local_port,
            .dst_port = self.remote_port,
            .seq = initial_seq,
            .flags = .{ .syn = true },
        }, now_ns);
    }

    pub fn onSegment(self: *Connection, segment: Segment, now_ns: u64) bool {
        switch (self.state) {
            .Listen => {
                if (!segment.flags.syn) return false;
                self.remote_ip = segment.src_ip;
                self.remote_port = segment.src_port;
                self.recv_next = segment.seq + 1;
                self.send_unacked = 1_000;
                self.send_next = 1_001;
                self.state = .SynReceived;
                return self.sendTracked(self.makeSegment(.{
                    .seq = self.send_unacked,
                    .ack = self.recv_next,
                    .flags = .{ .syn = true, .ack = true },
                }), now_ns);
            },
            .SynSent => {
                if (!segment.flags.syn or !segment.flags.ack or segment.ack != self.send_next) return false;
                self.recv_next = segment.seq + 1;
                self.remote_window = segment.window;
                self.onAck(segment.ack);
                self.state = .Established;
                return self.send(self.makeSegment(.{
                    .seq = self.send_next,
                    .ack = self.recv_next,
                    .flags = .{ .ack = true },
                }));
            },
            .SynReceived => {
                if (!segment.flags.ack or segment.ack != self.send_next) return false;
                self.onAck(segment.ack);
                self.state = .Established;
                return true;
            },
            .Established => {
                if (segment.flags.ack) self.onAck(segment.ack);
                if (segment.payload.len > 0 and segment.seq == self.recv_next) {
                    self.recv_next += @intCast(segment.payload.len);
                    return self.send(self.makeSegment(.{
                        .seq = self.send_next,
                        .ack = self.recv_next,
                        .flags = .{ .ack = true },
                    }));
                }
                return true;
            },
            else => return false,
        }
    }

    pub fn sendData(self: *Connection, payload: []const u8, now_ns: u64) bool {
        if (self.state != .Established or payload.len == 0) return false;
        if (self.nagle_enabled and self.send_unacked != self.send_next and payload.len < MSS) return false;
        const len = @min(payload.len, @min(MSS, self.cwnd_bytes));
        const segment = self.makeSegment(.{
            .seq = self.send_next,
            .ack = self.recv_next,
            .flags = .{ .ack = true, .psh = true },
            .payload = payload[0..len],
        });
        self.send_next += @intCast(len);
        return self.sendTracked(segment, now_ns);
    }

    pub fn onTimer(self: *Connection, now_ns: u64) bool {
        if (self.last_unacked == null or now_ns < self.retransmit_at_ns) return false;
        self.onLoss();
        const segment = self.last_unacked.?;
        _ = self.send(segment);
        self.rto_ns = @min(self.rto_ns * 2, 60 * std.time.ns_per_s);
        self.retransmit_at_ns = now_ns + self.rto_ns;
        return true;
    }

    pub fn onAck(self: *Connection, ack: u32) void {
        if (ack <= self.send_unacked) return;
        self.send_unacked = ack;
        if (self.send_unacked == self.send_next) self.last_unacked = null;
        if (self.cwnd_bytes < self.ssthresh_bytes) {
            self.cwnd_bytes += MSS;
        } else {
            self.cwnd_bytes += @max(1, (MSS * MSS) / self.cwnd_bytes);
        }
    }

    pub fn onLoss(self: *Connection) void {
        self.ssthresh_bytes = @max(MSS * 2, self.cwnd_bytes / 2);
        self.cwnd_bytes = MSS;
    }

    fn sendTracked(self: *Connection, segment: Segment, now_ns: u64) bool {
        if (!self.send(segment)) return false;
        self.last_unacked = segment;
        self.retransmit_at_ns = now_ns + self.rto_ns;
        return true;
    }

    fn send(self: *Connection, segment: Segment) bool {
        return self.send_fn(self.send_ctx, segment);
    }

    const SegmentOptions = struct {
        seq: u32,
        ack: u32 = 0,
        flags: Flags,
        payload: []const u8 = "",
    };

    fn makeSegment(self: *Connection, options: SegmentOptions) Segment {
        return .{
            .src_ip = self.local_ip,
            .dst_ip = self.remote_ip,
            .src_port = self.local_port,
            .dst_port = self.remote_port,
            .seq = options.seq,
            .ack = options.ack,
            .flags = options.flags,
            .window = 65535,
            .payload = options.payload,
        };
    }
};
