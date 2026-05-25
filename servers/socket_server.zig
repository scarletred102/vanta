const std = @import("std");
const eth = @import("net_ethernet_arp.zig");
const udp = @import("net_udp.zig");
const tcp = @import("net_tcp.zig");

pub const Protocol = enum { Tcp, Udp };
pub const SocketState = enum { Closed, Open, Bound, Connecting, Connected, Listening };

pub const SocketCap = u64;
pub const SERVICE_NAME = "sys.socket";

pub const MSG_SOCKET_OPEN: u32 = 0x0301;
pub const MSG_SOCKET_BIND: u32 = 0x0302;
pub const MSG_SOCKET_CONNECT: u32 = 0x0303;
pub const MSG_SOCKET_ACCEPT: u32 = 0x0304;
pub const MSG_SOCKET_SEND: u32 = 0x0305;
pub const MSG_SOCKET_RECV: u32 = 0x0306;
pub const MSG_SOCKET_CLOSE: u32 = 0x0307;

const MAX_SOCKETS: usize = 64;

const SocketEntry = struct {
    generation: u16 = 1,
    protocol: Protocol = .Udp,
    state: SocketState = .Closed,
    local_port: u16 = 0,
    remote_ip: eth.Ip4 = .{ 0, 0, 0, 0 },
    remote_port: u16 = 0,
    udp_socket: ?udp.Socket = null,
    tcp_connection: ?tcp.Connection = null,
    valid: bool = false,
};

pub const Service = struct {
    local_ip: eth.Ip4,
    udp_stack: udp.Stack,
    tcp_send_fn: tcp.SendSegmentFn,
    tcp_send_ctx: *anyopaque,
    sockets: [MAX_SOCKETS]SocketEntry = [_]SocketEntry{.{}} ** MAX_SOCKETS,
    next_tcp_seq: u32 = 100,

    pub fn init(
        local_ip: eth.Ip4,
        udp_send_fn: udp.SendUdpFn,
        tcp_send_fn: tcp.SendSegmentFn,
        send_ctx: *anyopaque,
    ) Service {
        return .{
            .local_ip = local_ip,
            .udp_stack = udp.Stack.init(local_ip, udp_send_fn, send_ctx),
            .tcp_send_fn = tcp_send_fn,
            .tcp_send_ctx = send_ctx,
        };
    }

    pub fn open(self: *Service, protocol: Protocol) !SocketCap {
        for (&self.sockets, 0..) |*entry, idx| {
            if (!entry.valid) {
                entry.* = .{
                    .generation = entry.generation,
                    .protocol = protocol,
                    .state = .Open,
                    .valid = true,
                };
                return makeCap(idx, entry.generation);
            }
        }
        return error.NoSockets;
    }

    pub fn bind(self: *Service, cap: SocketCap, port: u16) !void {
        const entry = self.lookup(cap) orelse return error.BadSocket;
        if (entry.state != .Open) return error.BadState;
        entry.local_port = port;
        switch (entry.protocol) {
            .Udp => entry.udp_socket = try self.udp_stack.bind(port, 0),
            .Tcp => entry.tcp_connection = tcp.Connection.init(self.local_ip, port, self.tcp_send_fn, self.tcp_send_ctx),
        }
        entry.state = .Bound;
    }

    pub fn connect(self: *Service, cap: SocketCap, remote_ip: eth.Ip4, remote_port: u16, now_ns: u64) !void {
        const entry = self.lookup(cap) orelse return error.BadSocket;
        if (entry.state != .Bound) return error.BadState;
        entry.remote_ip = remote_ip;
        entry.remote_port = remote_port;
        switch (entry.protocol) {
            .Udp => entry.state = .Connected,
            .Tcp => {
                if (entry.tcp_connection == null) {
                    entry.tcp_connection = tcp.Connection.init(self.local_ip, entry.local_port, self.tcp_send_fn, self.tcp_send_ctx);
                }
                const seq = self.next_tcp_seq;
                self.next_tcp_seq +%= 4096;
                if (!entry.tcp_connection.?.connect(remote_ip, remote_port, seq, now_ns)) return error.ConnectFailed;
                entry.state = .Connecting;
            },
        }
    }

    pub fn send(self: *Service, cap: SocketCap, payload: []const u8, now_ns: u64) bool {
        const entry = self.lookup(cap) orelse return false;
        switch (entry.protocol) {
            .Udp => {
                if (entry.state != .Connected) return false;
                const sock = entry.udp_socket orelse return false;
                return self.udp_stack.sendTo(sock, entry.remote_ip, entry.remote_port, payload);
            },
            .Tcp => {
                if (entry.state != .Connected) return false;
                if (entry.tcp_connection) |*conn| return conn.sendData(payload, now_ns);
                return false;
            },
        }
    }

    pub fn recv(self: *Service, cap: SocketCap) ?udp.Datagram {
        const entry = self.lookup(cap) orelse return null;
        if (entry.protocol != .Udp) return null;
        const sock = entry.udp_socket orelse return null;
        return self.udp_stack.recvFrom(sock);
    }

    pub fn onTcpSegment(self: *Service, cap: SocketCap, segment: tcp.Segment, now_ns: u64) bool {
        const entry = self.lookup(cap) orelse return false;
        if (entry.protocol != .Tcp) return false;
        if (entry.tcp_connection) |*conn| {
            const ok = conn.onSegment(segment, now_ns);
            if (ok and conn.state == .Established) entry.state = .Connected;
            return ok;
        }
        return false;
    }

    pub fn close(self: *Service, cap: SocketCap) !void {
        const idx = capIndex(cap);
        const entry = self.lookup(cap) orelse return error.BadSocket;
        entry.* = .{ .generation = entry.generation +% 1 };
        if (idx < self.sockets.len and self.sockets[idx].generation == 0) {
            self.sockets[idx].generation = 1;
        }
    }

    pub fn stateOf(self: *Service, cap: SocketCap) ?SocketState {
        const entry = self.lookup(cap) orelse return null;
        return entry.state;
    }

    fn lookup(self: *Service, cap: SocketCap) ?*SocketEntry {
        const idx = capIndex(cap);
        if (idx >= self.sockets.len) return null;
        const entry = &self.sockets[idx];
        if (!entry.valid or entry.generation != capGeneration(cap)) return null;
        return entry;
    }
};

fn makeCap(index: usize, generation: u16) SocketCap {
    return (@as(SocketCap, generation) << 48) | @as(SocketCap, index);
}

fn capIndex(cap: SocketCap) usize {
    return @intCast(cap & 0x0000ffffffffffff);
}

fn capGeneration(cap: SocketCap) u16 {
    return @intCast(cap >> 48);
}
