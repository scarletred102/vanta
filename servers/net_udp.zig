const std = @import("std");
const eth = @import("net_ethernet_arp.zig");

pub const PROTO_UDP: u8 = 17;

const UDP_HEADER_LEN: usize = 8;
const MAX_PAYLOAD_LEN: usize = 1472;
const MAX_SOCKETS: usize = 32;
const QUEUE_LEN: usize = 4;

pub const SendUdpFn = *const fn (ctx: *anyopaque, dst_ip: eth.Ip4, protocol: u8, payload: []const u8) bool;

pub const Socket = struct {
    index: usize,
    local_port: u16,
    notification_cap: u64,
};

pub const Endpoint = struct {
    src_ip: eth.Ip4,
    dst_ip: eth.Ip4,
};

pub const WriteOptions = struct {
    src_ip: eth.Ip4,
    dst_ip: eth.Ip4,
    src_port: u16,
    dst_port: u16,
};

pub const Datagram = struct {
    src_ip: eth.Ip4,
    src_port: u16,
    dst_port: u16,
    payload: []const u8,
};

const QueuedDatagram = struct {
    src_ip: eth.Ip4 = .{ 0, 0, 0, 0 },
    src_port: u16 = 0,
    len: usize = 0,
    payload: [MAX_PAYLOAD_LEN]u8 = [_]u8{0} ** MAX_PAYLOAD_LEN,
    valid: bool = false,
};

const SocketEntry = struct {
    local_port: u16 = 0,
    notification_cap: u64 = 0,
    queue: [QUEUE_LEN]QueuedDatagram = [_]QueuedDatagram{.{}} ** QUEUE_LEN,
    read_idx: usize = 0,
    write_idx: usize = 0,
    queued: usize = 0,
    valid: bool = false,
};

pub const Stack = struct {
    local_ip: eth.Ip4,
    send_fn: SendUdpFn,
    send_ctx: *anyopaque,
    sockets: [MAX_SOCKETS]SocketEntry = [_]SocketEntry{.{}} ** MAX_SOCKETS,
    next_ephemeral: u16 = 49152,
    last_wake_notification: ?u64 = null,

    pub fn init(local_ip: eth.Ip4, send_fn: SendUdpFn, send_ctx: *anyopaque) Stack {
        return .{
            .local_ip = local_ip,
            .send_fn = send_fn,
            .send_ctx = send_ctx,
        };
    }

    pub fn bind(self: *Stack, requested_port: u16, notification_cap: u64) !Socket {
        const port = if (requested_port == 0) self.allocateEphemeral() else requested_port;
        for (&self.sockets) |entry| {
            if (entry.valid and entry.local_port == port) return error.PortInUse;
        }
        for (&self.sockets, 0..) |*entry, idx| {
            if (!entry.valid) {
                entry.* = .{
                    .local_port = port,
                    .notification_cap = notification_cap,
                    .valid = true,
                };
                return .{ .index = idx, .local_port = port, .notification_cap = notification_cap };
            }
        }
        return error.NoSockets;
    }

    pub fn sendTo(self: *Stack, socket: Socket, dst_ip: eth.Ip4, dst_port: u16, payload: []const u8) bool {
        if (payload.len > MAX_PAYLOAD_LEN) return false;
        const entry = self.socketEntry(socket) orelse return false;
        var datagram: [UDP_HEADER_LEN + MAX_PAYLOAD_LEN]u8 = undefined;
        writeDatagram(datagram[0 .. UDP_HEADER_LEN + payload.len], .{
            .src_ip = self.local_ip,
            .dst_ip = dst_ip,
            .src_port = entry.local_port,
            .dst_port = dst_port,
        }, payload);
        return self.send_fn(self.send_ctx, dst_ip, PROTO_UDP, datagram[0 .. UDP_HEADER_LEN + payload.len]);
    }

    pub fn handleDatagram(self: *Stack, src_ip: eth.Ip4, dst_ip: eth.Ip4, bytes: []const u8) bool {
        const parsed = parseDatagram(bytes, .{ .src_ip = src_ip, .dst_ip = dst_ip }) catch return false;
        const entry = self.findSocket(parsed.dst_port) orelse return false;
        if (entry.queued == QUEUE_LEN or parsed.payload.len > MAX_PAYLOAD_LEN) return false;

        const slot = entry.write_idx % QUEUE_LEN;
        entry.queue[slot] = .{
            .src_ip = parsed.src_ip,
            .src_port = parsed.src_port,
            .len = parsed.payload.len,
            .valid = true,
        };
        @memcpy(entry.queue[slot].payload[0..parsed.payload.len], parsed.payload);
        entry.write_idx = (entry.write_idx + 1) % QUEUE_LEN;
        entry.queued += 1;
        self.last_wake_notification = entry.notification_cap;
        return true;
    }

    pub fn recvFrom(self: *Stack, socket: Socket) ?Datagram {
        const entry = self.socketEntry(socket) orelse return null;
        if (entry.queued == 0) return null;
        const slot = entry.read_idx % QUEUE_LEN;
        const queued = &entry.queue[slot];
        if (!queued.valid) return null;
        entry.read_idx = (entry.read_idx + 1) % QUEUE_LEN;
        entry.queued -= 1;
        queued.valid = false;
        return .{
            .src_ip = queued.src_ip,
            .src_port = queued.src_port,
            .dst_port = entry.local_port,
            .payload = queued.payload[0..queued.len],
        };
    }

    fn socketEntry(self: *Stack, socket: Socket) ?*SocketEntry {
        if (socket.index >= self.sockets.len) return null;
        const entry = &self.sockets[socket.index];
        if (!entry.valid or entry.local_port != socket.local_port) return null;
        return entry;
    }

    fn findSocket(self: *Stack, port: u16) ?*SocketEntry {
        for (&self.sockets) |*entry| {
            if (entry.valid and entry.local_port == port) return entry;
        }
        return null;
    }

    fn allocateEphemeral(self: *Stack) u16 {
        const port = self.next_ephemeral;
        self.next_ephemeral +%= 1;
        if (self.next_ephemeral < 49152) self.next_ephemeral = 49152;
        return port;
    }
};

pub fn parseDatagram(bytes: []const u8, endpoint: Endpoint) !Datagram {
    if (bytes.len < UDP_HEADER_LEN) return error.ShortPacket;
    const len = std.mem.readInt(u16, bytes[4..6], .big);
    if (len < UDP_HEADER_LEN or len > bytes.len) return error.BadLength;
    const checksum_value = std.mem.readInt(u16, bytes[6..8], .big);
    if (checksum_value != 0 and udpChecksum(endpoint.src_ip, endpoint.dst_ip, bytes[0..len]) != 0) {
        return error.BadChecksum;
    }
    return .{
        .src_ip = endpoint.src_ip,
        .src_port = std.mem.readInt(u16, bytes[0..2], .big),
        .dst_port = std.mem.readInt(u16, bytes[2..4], .big),
        .payload = bytes[UDP_HEADER_LEN..len],
    };
}

pub fn writeDatagram(buf: []u8, options: WriteOptions, payload: []const u8) void {
    std.debug.assert(buf.len >= UDP_HEADER_LEN + payload.len);
    std.mem.writeInt(u16, buf[0..2], options.src_port, .big);
    std.mem.writeInt(u16, buf[2..4], options.dst_port, .big);
    std.mem.writeInt(u16, buf[4..6], @intCast(UDP_HEADER_LEN + payload.len), .big);
    buf[6] = 0;
    buf[7] = 0;
    @memcpy(buf[UDP_HEADER_LEN .. UDP_HEADER_LEN + payload.len], payload);
    const sum = udpChecksum(options.src_ip, options.dst_ip, buf[0 .. UDP_HEADER_LEN + payload.len]);
    std.mem.writeInt(u16, buf[6..8], if (sum == 0) 0xffff else sum, .big);
}

fn udpChecksum(src_ip: eth.Ip4, dst_ip: eth.Ip4, udp_bytes: []const u8) u16 {
    var sum: u32 = 0;
    sum = addBytes(sum, &src_ip);
    sum = addBytes(sum, &dst_ip);
    sum += PROTO_UDP;
    sum += @intCast(udp_bytes.len);
    sum = fold(sum);
    sum = addBytes(sum, udp_bytes);
    return ~@as(u16, @truncate(fold(sum)));
}

fn addBytes(initial: u32, bytes: []const u8) u32 {
    var sum = initial;
    var i: usize = 0;
    while (i + 1 < bytes.len) : (i += 2) {
        sum += (@as(u32, bytes[i]) << 8) | bytes[i + 1];
        sum = fold(sum);
    }
    if (i < bytes.len) {
        sum += @as(u32, bytes[i]) << 8;
        sum = fold(sum);
    }
    return sum;
}

fn fold(initial: u32) u32 {
    var sum = initial;
    while (sum >> 16 != 0) {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    return sum;
}
