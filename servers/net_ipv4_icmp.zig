const std = @import("std");
const eth = @import("net_ethernet_arp.zig");

pub const PROTO_ICMP: u8 = 1;
pub const FRAGMENT_TTL_NS: u64 = 30 * std.time.ns_per_s;

const IPV4_HEADER_LEN: usize = 20;
const MAX_PAYLOAD_LEN: usize = 1480;
const MAX_REASSEMBLED_LEN: usize = 4096;
const FRAGMENT_TABLE_SIZE: usize = 8;
const FRAGMENTS_PER_ENTRY: usize = 8;

pub const SendIpv4Fn = *const fn (ctx: *anyopaque, dst_ip: eth.Ip4, packet: []const u8) bool;

pub const WriteOptions = struct {
    src_ip: eth.Ip4,
    dst_ip: eth.Ip4,
    protocol: u8,
    identification: u16 = 0,
    ttl: u8 = 64,
    more_fragments: bool = false,
    fragment_offset: u13 = 0,
};

pub const Packet = struct {
    src_ip: eth.Ip4,
    dst_ip: eth.Ip4,
    protocol: u8,
    identification: u16,
    payload: []const u8,
    more_fragments: bool,
    fragment_offset_bytes: usize,
};

pub const Datagram = struct {
    src_ip: eth.Ip4,
    dst_ip: eth.Ip4,
    protocol: u8,
    payload: []const u8,
};

const FragmentRange = struct {
    start: usize = 0,
    end: usize = 0,
    valid: bool = false,
};

const FragmentEntry = struct {
    src_ip: eth.Ip4 = .{ 0, 0, 0, 0 },
    identification: u16 = 0,
    protocol: u8 = 0,
    dst_ip: eth.Ip4 = .{ 0, 0, 0, 0 },
    first_seen_ns: u64 = 0,
    total_len: ?usize = null,
    ranges: [FRAGMENTS_PER_ENTRY]FragmentRange = [_]FragmentRange{.{}} ** FRAGMENTS_PER_ENTRY,
    buffer: [MAX_REASSEMBLED_LEN]u8 = [_]u8{0} ** MAX_REASSEMBLED_LEN,
    valid: bool = false,

    fn reset(self: *FragmentEntry) void {
        self.* = .{};
    }
};

pub const Stack = struct {
    ip: eth.Ip4,
    send_fn: SendIpv4Fn,
    send_ctx: *anyopaque,
    fragments: [FRAGMENT_TABLE_SIZE]FragmentEntry = [_]FragmentEntry{.{}} ** FRAGMENT_TABLE_SIZE,
    next_identification: u16 = 1,

    pub fn init(ip: eth.Ip4, send_fn: SendIpv4Fn, send_ctx: *anyopaque) Stack {
        return .{
            .ip = ip,
            .send_fn = send_fn,
            .send_ctx = send_ctx,
        };
    }

    pub fn sendPacket(self: *Stack, dst_ip: eth.Ip4, protocol: u8, payload: []const u8) bool {
        var packet: [IPV4_HEADER_LEN + MAX_PAYLOAD_LEN]u8 = undefined;
        if (payload.len > MAX_PAYLOAD_LEN) return false;
        writeIpv4Packet(packet[0 .. IPV4_HEADER_LEN + payload.len], .{
            .src_ip = self.ip,
            .dst_ip = dst_ip,
            .protocol = protocol,
            .identification = self.next_identification,
        }, payload);
        self.next_identification +%= 1;
        return self.send_fn(self.send_ctx, dst_ip, packet[0 .. IPV4_HEADER_LEN + payload.len]);
    }

    pub fn handleIpv4Packet(self: *Stack, bytes: []const u8, now_ns: u64) ?Datagram {
        self.expireFragments(now_ns);
        const parsed = parseIpv4Packet(bytes) catch return null;
        const datagram = if (parsed.more_fragments or parsed.fragment_offset_bytes != 0)
            self.handleFragment(parsed, now_ns)
        else
            Datagram{
                .src_ip = parsed.src_ip,
                .dst_ip = parsed.dst_ip,
                .protocol = parsed.protocol,
                .payload = parsed.payload,
            };

        const assembled = datagram orelse return null;
        if (assembled.protocol == PROTO_ICMP and std.mem.eql(u8, &assembled.dst_ip, &self.ip)) {
            self.handleIcmp(assembled);
            return null;
        }
        return assembled;
    }

    fn handleIcmp(self: *Stack, datagram: Datagram) void {
        if (datagram.payload.len < 8) return;
        if (!verifyChecksum(datagram.payload)) return;
        if (datagram.payload[0] != 8) return;

        var reply_payload: [MAX_PAYLOAD_LEN]u8 = undefined;
        if (datagram.payload.len > reply_payload.len) return;
        @memcpy(reply_payload[0..datagram.payload.len], datagram.payload);
        reply_payload[0] = 0;
        reply_payload[2] = 0;
        reply_payload[3] = 0;
        const sum = checksum(reply_payload[0..datagram.payload.len]);
        std.mem.writeInt(u16, reply_payload[2..4], sum, .big);
        _ = self.sendPacket(datagram.src_ip, PROTO_ICMP, reply_payload[0..datagram.payload.len]);
    }

    fn handleFragment(self: *Stack, parsed: Packet, now_ns: u64) ?Datagram {
        const offset = parsed.fragment_offset_bytes;
        if (offset + parsed.payload.len > MAX_REASSEMBLED_LEN) return null;

        const entry = self.findOrCreateFragmentEntry(parsed, now_ns) orelse return null;
        @memcpy(entry.buffer[offset .. offset + parsed.payload.len], parsed.payload);
        addRange(entry, offset, offset + parsed.payload.len);
        if (!parsed.more_fragments) entry.total_len = offset + parsed.payload.len;

        const total = entry.total_len orelse return null;
        if (!hasContiguousCoverage(entry, total)) return null;

        const result = Datagram{
            .src_ip = entry.src_ip,
            .dst_ip = entry.dst_ip,
            .protocol = entry.protocol,
            .payload = entry.buffer[0..total],
        };
        return result;
    }

    fn findOrCreateFragmentEntry(self: *Stack, parsed: Packet, now_ns: u64) ?*FragmentEntry {
        for (&self.fragments) |*entry| {
            if (!entry.valid) continue;
            if (entry.identification == parsed.identification and
                entry.protocol == parsed.protocol and
                std.mem.eql(u8, &entry.src_ip, &parsed.src_ip))
            {
                return entry;
            }
        }

        for (&self.fragments) |*entry| {
            if (!entry.valid) {
                entry.* = .{
                    .src_ip = parsed.src_ip,
                    .dst_ip = parsed.dst_ip,
                    .protocol = parsed.protocol,
                    .identification = parsed.identification,
                    .first_seen_ns = now_ns,
                    .valid = true,
                };
                return entry;
            }
        }
        return null;
    }

    fn expireFragments(self: *Stack, now_ns: u64) void {
        for (&self.fragments) |*entry| {
            if (entry.valid and now_ns -% entry.first_seen_ns > FRAGMENT_TTL_NS) {
                entry.reset();
            }
        }
    }
};

fn addRange(self: *FragmentEntry, start: usize, end: usize) void {
    for (&self.ranges) |*range| {
        if (!range.valid) {
            range.* = .{ .start = start, .end = end, .valid = true };
            return;
        }
    }
}

fn hasContiguousCoverage(self: *FragmentEntry, total: usize) bool {
    var cursor: usize = 0;
    while (cursor < total) {
        var best_end = cursor;
        for (&self.ranges) |*range| {
            if (range.valid and range.start <= cursor and range.end > best_end) {
                best_end = range.end;
            }
        }
        if (best_end == cursor) return false;
        cursor = best_end;
    }
    return true;
}

pub fn parseIpv4Packet(bytes: []const u8) !Packet {
    if (bytes.len < IPV4_HEADER_LEN) return error.ShortPacket;
    if (bytes[0] >> 4 != 4) return error.NotIpv4;
    const ihl = @as(usize, bytes[0] & 0x0f) * 4;
    if (ihl < IPV4_HEADER_LEN or bytes.len < ihl) return error.BadHeaderLength;
    const total_len = std.mem.readInt(u16, bytes[2..4], .big);
    if (total_len < ihl or total_len > bytes.len) return error.BadTotalLength;
    if (!verifyChecksum(bytes[0..ihl])) return error.BadChecksum;

    const frag = std.mem.readInt(u16, bytes[6..8], .big);
    var src_ip: eth.Ip4 = undefined;
    var dst_ip: eth.Ip4 = undefined;
    @memcpy(&src_ip, bytes[12..16]);
    @memcpy(&dst_ip, bytes[16..20]);

    return .{
        .src_ip = src_ip,
        .dst_ip = dst_ip,
        .protocol = bytes[9],
        .identification = std.mem.readInt(u16, bytes[4..6], .big),
        .payload = bytes[ihl..total_len],
        .more_fragments = (frag & 0x2000) != 0,
        .fragment_offset_bytes = @as(usize, frag & 0x1fff) * 8,
    };
}

pub fn writeIpv4Packet(buf: []u8, options: WriteOptions, payload: []const u8) void {
    std.debug.assert(buf.len >= IPV4_HEADER_LEN + payload.len);
    buf[0] = 0x45;
    buf[1] = 0;
    std.mem.writeInt(u16, buf[2..4], @intCast(IPV4_HEADER_LEN + payload.len), .big);
    std.mem.writeInt(u16, buf[4..6], options.identification, .big);
    const flags_offset: u16 = (if (options.more_fragments) @as(u16, 0x2000) else 0) |
        @as(u16, options.fragment_offset);
    std.mem.writeInt(u16, buf[6..8], flags_offset, .big);
    buf[8] = options.ttl;
    buf[9] = options.protocol;
    buf[10] = 0;
    buf[11] = 0;
    @memcpy(buf[12..16], &options.src_ip);
    @memcpy(buf[16..20], &options.dst_ip);
    const header_sum = checksum(buf[0..IPV4_HEADER_LEN]);
    std.mem.writeInt(u16, buf[10..12], header_sum, .big);
    @memcpy(buf[IPV4_HEADER_LEN .. IPV4_HEADER_LEN + payload.len], payload);
}

pub const IcmpKind = enum { request, reply };

pub fn writeIcmpEcho(buf: []u8, kind: IcmpKind, identifier: u16, sequence: u16, data: []const u8) void {
    std.debug.assert(buf.len >= 8 + data.len);
    buf[0] = if (kind == .request) 8 else 0;
    buf[1] = 0;
    buf[2] = 0;
    buf[3] = 0;
    std.mem.writeInt(u16, buf[4..6], identifier, .big);
    std.mem.writeInt(u16, buf[6..8], sequence, .big);
    @memcpy(buf[8 .. 8 + data.len], data);
    const sum = checksum(buf[0 .. 8 + data.len]);
    std.mem.writeInt(u16, buf[2..4], sum, .big);
}

pub fn verifyChecksum(bytes: []const u8) bool {
    return onesComplementSum(bytes) == 0xffff;
}

pub fn checksum(bytes: []const u8) u16 {
    return ~onesComplementSum(bytes);
}

fn onesComplementSum(bytes: []const u8) u16 {
    var sum: u32 = 0;
    var i: usize = 0;
    while (i + 1 < bytes.len) : (i += 2) {
        sum += (@as(u32, bytes[i]) << 8) | bytes[i + 1];
        sum = (sum & 0xffff) + (sum >> 16);
    }
    if (i < bytes.len) {
        sum += @as(u32, bytes[i]) << 8;
        sum = (sum & 0xffff) + (sum >> 16);
    }
    while (sum >> 16 != 0) {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    return @truncate(sum);
}
