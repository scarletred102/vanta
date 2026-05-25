const std = @import("std");

pub const Mac = [6]u8;
pub const Ip4 = [4]u8;

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const ETHERTYPE_IPV6: u16 = 0x86DD;

pub const ARP_OP_REQUEST: u16 = 1;
pub const ARP_OP_REPLY: u16 = 2;
pub const ARP_CACHE_TTL_NS: u64 = 60 * std.time.ns_per_s;

const ARP_CACHE_SIZE: usize = 32;
const ETHERNET_HEADER_LEN: usize = 14;
const ARP_PACKET_LEN: usize = 28;
const MTU: usize = 1500;

pub const SendFn = *const fn (ctx: *anyopaque, frame: []const u8) bool;

pub const ArpPacket = struct {
    op: u16,
    sender_mac: Mac,
    sender_ip: Ip4,
    target_mac: Mac,
    target_ip: Ip4,
};

const ArpEntry = struct {
    ip: Ip4 = .{ 0, 0, 0, 0 },
    mac: Mac = .{ 0, 0, 0, 0, 0, 0 },
    expiry_ns: u64 = 0,
    valid: bool = false,
};

pub const ArpCache = struct {
    entries: [ARP_CACHE_SIZE]ArpEntry = [_]ArpEntry{.{}} ** ARP_CACHE_SIZE,

    pub fn get(self: *ArpCache, ip: Ip4, now_ns: u64) ?Mac {
        for (&self.entries) |*entry| {
            if (!entry.valid) continue;
            if (now_ns >= entry.expiry_ns) {
                entry.valid = false;
                continue;
            }
            if (std.mem.eql(u8, &entry.ip, &ip)) return entry.mac;
        }
        return null;
    }

    pub fn put(self: *ArpCache, ip: Ip4, mac: Mac, now_ns: u64) void {
        const expiry_ns = now_ns +| ARP_CACHE_TTL_NS;

        for (&self.entries) |*entry| {
            if (entry.valid and std.mem.eql(u8, &entry.ip, &ip)) {
                entry.mac = mac;
                entry.expiry_ns = expiry_ns;
                return;
            }
        }

        var best_idx: usize = 0;
        for (&self.entries, 0..) |*entry, idx| {
            if (!entry.valid) {
                best_idx = idx;
                break;
            }
            if (entry.expiry_ns < self.entries[best_idx].expiry_ns) {
                best_idx = idx;
            }
        }

        self.entries[best_idx] = .{
            .ip = ip,
            .mac = mac,
            .expiry_ns = expiry_ns,
            .valid = true,
        };
    }
};

pub const Stack = struct {
    mac: Mac,
    ip: Ip4,
    cache: ArpCache,
    send_fn: SendFn,
    send_ctx: *anyopaque,

    pub fn init(mac: Mac, ip: Ip4, send_fn: SendFn, send_ctx: *anyopaque) Stack {
        return .{
            .mac = mac,
            .ip = ip,
            .cache = .{},
            .send_fn = send_fn,
            .send_ctx = send_ctx,
        };
    }

    pub fn sendEthernetFrame(self: *Stack, dst_mac: Mac, ethertype: u16, payload: []const u8) bool {
        var frame: [ETHERNET_HEADER_LEN + MTU]u8 = undefined;
        if (payload.len > MTU) return false;

        writeEthernetHeader(frame[0..ETHERNET_HEADER_LEN], dst_mac, self.mac, ethertype);
        @memcpy(frame[ETHERNET_HEADER_LEN .. ETHERNET_HEADER_LEN + payload.len], payload);
        return self.send_fn(self.send_ctx, frame[0 .. ETHERNET_HEADER_LEN + payload.len]);
    }

    pub fn sendArpRequest(self: *Stack, target_ip: Ip4) bool {
        var payload: [ARP_PACKET_LEN]u8 = undefined;
        writeArpPacket(&payload, .{
            .op = ARP_OP_REQUEST,
            .sender_mac = self.mac,
            .sender_ip = self.ip,
            .target_mac = .{ 0, 0, 0, 0, 0, 0 },
            .target_ip = target_ip,
        });
        return self.sendEthernetFrame(.{ 0xff, 0xff, 0xff, 0xff, 0xff, 0xff }, ETHERTYPE_ARP, &payload);
    }

    pub fn sendArpReply(self: *Stack, target_mac: Mac, target_ip: Ip4) bool {
        var payload: [ARP_PACKET_LEN]u8 = undefined;
        writeArpPacket(&payload, .{
            .op = ARP_OP_REPLY,
            .sender_mac = self.mac,
            .sender_ip = self.ip,
            .target_mac = target_mac,
            .target_ip = target_ip,
        });
        return self.sendEthernetFrame(target_mac, ETHERTYPE_ARP, &payload);
    }

    pub fn resolveIpv4Destination(self: *Stack, target_ip: Ip4, now_ns: u64) ?Mac {
        if (self.cache.get(target_ip, now_ns)) |mac| return mac;
        _ = self.sendArpRequest(target_ip);
        return null;
    }

    pub fn handleReceivedFrame(self: *Stack, frame: []const u8, now_ns: u64) ?[]const u8 {
        if (frame.len < ETHERNET_HEADER_LEN) return null;

        const ethertype = std.mem.readInt(u16, frame[12..14], .big);
        switch (ethertype) {
            ETHERTYPE_ARP => {
                if (frame.len < ETHERNET_HEADER_LEN + ARP_PACKET_LEN) return null;
                const arp = readArpPacket(frame[ETHERNET_HEADER_LEN .. ETHERNET_HEADER_LEN + ARP_PACKET_LEN]) orelse return null;
                self.cache.put(arp.sender_ip, arp.sender_mac, now_ns);
                if (arp.op == ARP_OP_REQUEST and std.mem.eql(u8, &arp.target_ip, &self.ip)) {
                    _ = self.sendArpReply(arp.sender_mac, arp.sender_ip);
                }
                return null;
            },
            ETHERTYPE_IPV4 => return frame[ETHERNET_HEADER_LEN..],
            ETHERTYPE_IPV6 => return null,
            else => return null,
        }
    }
};

pub fn writeEthernetHeader(buf: []u8, dst_mac: Mac, src_mac: Mac, ethertype: u16) void {
    std.debug.assert(buf.len >= ETHERNET_HEADER_LEN);
    @memcpy(buf[0..6], &dst_mac);
    @memcpy(buf[6..12], &src_mac);
    std.mem.writeInt(u16, buf[12..14], ethertype, .big);
}

pub fn writeArpPacket(buf: []u8, packet: ArpPacket) void {
    std.debug.assert(buf.len >= ARP_PACKET_LEN);
    std.mem.writeInt(u16, buf[0..2], 1, .big);
    std.mem.writeInt(u16, buf[2..4], ETHERTYPE_IPV4, .big);
    buf[4] = 6;
    buf[5] = 4;
    std.mem.writeInt(u16, buf[6..8], packet.op, .big);
    @memcpy(buf[8..14], &packet.sender_mac);
    @memcpy(buf[14..18], &packet.sender_ip);
    @memcpy(buf[18..24], &packet.target_mac);
    @memcpy(buf[24..28], &packet.target_ip);
}

fn readArpPacket(buf: []const u8) ?ArpPacket {
    if (buf.len < ARP_PACKET_LEN) return null;
    if (std.mem.readInt(u16, buf[0..2], .big) != 1) return null;
    if (std.mem.readInt(u16, buf[2..4], .big) != ETHERTYPE_IPV4) return null;
    if (buf[4] != 6 or buf[5] != 4) return null;

    var packet = ArpPacket{
        .op = std.mem.readInt(u16, buf[6..8], .big),
        .sender_mac = undefined,
        .sender_ip = undefined,
        .target_mac = undefined,
        .target_ip = undefined,
    };
    @memcpy(&packet.sender_mac, buf[8..14]);
    @memcpy(&packet.sender_ip, buf[14..18]);
    @memcpy(&packet.target_mac, buf[18..24]);
    @memcpy(&packet.target_ip, buf[24..28]);
    return packet;
}
