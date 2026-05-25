const std = @import("std");
const eth = @import("net_ethernet_arp.zig");

pub const DNS_PORT: u16 = 53;
pub const SERVICE_NAME = "sys.dns";

pub fn writeAQuery(buf: []u8, id: u16, hostname: []const u8) !usize {
    if (buf.len < 17) return error.BufferTooSmall;
    std.mem.writeInt(u16, buf[0..2], id, .big);
    std.mem.writeInt(u16, buf[2..4], 0x0100, .big);
    std.mem.writeInt(u16, buf[4..6], 1, .big);
    std.mem.writeInt(u16, buf[6..8], 0, .big);
    std.mem.writeInt(u16, buf[8..10], 0, .big);
    std.mem.writeInt(u16, buf[10..12], 0, .big);

    var out: usize = 12;
    var start: usize = 0;
    while (start < hostname.len) {
        var end = start;
        while (end < hostname.len and hostname[end] != '.') : (end += 1) {}
        const label_len = end - start;
        if (label_len == 0 or label_len > 63) return error.BadHostname;
        if (out + 1 + label_len >= buf.len) return error.BufferTooSmall;
        buf[out] = @intCast(label_len);
        out += 1;
        @memcpy(buf[out .. out + label_len], hostname[start..end]);
        out += label_len;
        start = end + 1;
    }

    if (out + 5 > buf.len) return error.BufferTooSmall;
    buf[out] = 0;
    out += 1;
    std.mem.writeInt(u16, buf[out..][0..2], 1, .big);
    out += 2;
    std.mem.writeInt(u16, buf[out..][0..2], 1, .big);
    out += 2;
    return out;
}

pub fn parseAResponse(buf: []const u8, expected_id: u16) !eth.Ip4 {
    if (buf.len < 12) return error.ShortPacket;
    if (std.mem.readInt(u16, buf[0..2], .big) != expected_id) return error.IdMismatch;
    const flags = std.mem.readInt(u16, buf[2..4], .big);
    if ((flags & 0x8000) == 0) return error.NotResponse;
    if ((flags & 0x000f) != 0) return error.DnsError;
    const qdcount = std.mem.readInt(u16, buf[4..6], .big);
    const ancount = std.mem.readInt(u16, buf[6..8], .big);

    var off: usize = 12;
    var qi: usize = 0;
    while (qi < qdcount) : (qi += 1) {
        off = try skipName(buf, off);
        if (off + 4 > buf.len) return error.ShortPacket;
        off += 4;
    }

    var ai: usize = 0;
    while (ai < ancount) : (ai += 1) {
        off = try skipName(buf, off);
        if (off + 10 > buf.len) return error.ShortPacket;
        const typ = std.mem.readInt(u16, buf[off..][0..2], .big);
        const class = std.mem.readInt(u16, buf[off + 2 ..][0..2], .big);
        const rdlen = std.mem.readInt(u16, buf[off + 8 ..][0..2], .big);
        off += 10;
        if (off + rdlen > buf.len) return error.ShortPacket;
        if (typ == 1 and class == 1 and rdlen == 4) {
            var result: eth.Ip4 = undefined;
            @memcpy(&result, buf[off .. off + 4]);
            return result;
        }
        off += rdlen;
    }
    return error.NoARecord;
}

fn skipName(buf: []const u8, start: usize) !usize {
    var off = start;
    while (true) {
        if (off >= buf.len) return error.ShortPacket;
        const len = buf[off];
        if ((len & 0xc0) == 0xc0) {
            if (off + 2 > buf.len) return error.ShortPacket;
            return off + 2;
        }
        off += 1;
        if (len == 0) return off;
        if ((len & 0xc0) != 0 or off + len > buf.len) return error.BadName;
        off += len;
    }
}
