// ============================================================================
// VantaOS Userspace — virtio-net Network Driver Server (P8)
//
// Startup capability slots (injected by kernel):
//   Slot 1: PCICap / BAR0 MemCap   (PCI_CAP_HANDLE)
//   Slot 2: Listener Port           (PORT_CAP_HANDLE)
//   Slot 3: Service Registry Port   (REGISTRY_CAP_HANDLE)
//   Slot 4: DeviceIRQ capability    (IRQ_CAP_HANDLE)
//
// IPC messages:
//   MSG_NET_SEND  0x0201  payload[0..4]=len,  buffer_cap=shm → send frame
//   MSG_NET_RECV  0x0202  payload[0..8]=max,  buffer_cap=shm → recv frame
//   MSG_NET_INFO  0x0203  (none)                             → MAC[6] in payload
// ============================================================================

const std = @import("std");
const libvanta = @import("../libvanta/libvanta.zig");
const net = @import("net_ethernet_arp.zig");
const ip = @import("net_ipv4_icmp.zig");

// ── Startup Capability Handles ─────────────────────────────────────────────
pub const PCI_CAP_HANDLE: u64      = 0x0001000000000001;
pub const PORT_CAP_HANDLE: u64     = 0x0001000000000002;
pub const REGISTRY_CAP_HANDLE: u64 = 0x0001000000000003;
pub const IRQ_CAP_HANDLE: u64      = 0x0001000000000004;

// ── Virtual Addresses ──────────────────────────────────────────────────────
// Carefully spaced so no region overlaps.
const VIRTIO_MMIO_VADDR: u64 = 0x20000000; // BAR0 MMIO (1 page)
const RX_QUEUE_VADDR: u64    = 0x40000000; // RX virtqueue  (2 pages: desc+avail | used)
const TX_QUEUE_VADDR: u64    = 0x40002000; // TX virtqueue  (2 pages)
const RX_BUF_VADDR: u64      = 0x41000000; // RX receive buffers (QUEUE_SIZE pages)
const TX_BUF_VADDR: u64      = 0x42000000; // TX transmit buffer (1 page)
const SHM_VADDR: u64         = 0x30000000; // Caller-provided shared memory

// ── IPC Message Codes ──────────────────────────────────────────────────────
pub const MSG_NET_SEND: u32 = 0x0201;
pub const MSG_NET_RECV: u32 = 0x0202;
pub const MSG_NET_INFO: u32 = 0x0203;
pub const MSG_ERROR: u32    = 0x0003;

// ── Virtio PCI Legacy Register Offsets (MMIO-mapped BAR0) ─────────────────
// Ref: VirtIO spec 1.0, §4.1.4.8 (legacy interface)
const REG_HOST_FEATURES: usize = 0x00; // u32 r   Device feature bits
const REG_GUEST_FEATURES: usize = 0x04; // u32 w   Driver feature bits
const REG_QUEUE_PFN: usize     = 0x08; // u32 rw  Queue PFN (phys addr >> 12)
const REG_QUEUE_SIZE: usize    = 0x0C; // u16 r   Current queue size
const REG_QUEUE_SEL: usize     = 0x0E; // u16 w   Queue select (0=RX, 1=TX)
const REG_QUEUE_NOTIFY: usize  = 0x10; // u16 w   Queue notify (kick)
const REG_STATUS: usize        = 0x12; // u8  rw  Device status register
const REG_ISR: usize           = 0x13; // u8  r   ISR status (clears on read)
const REG_CONFIG: usize        = 0x14; // Net config: MAC[0..5], Status[2]

// ── Virtio Device Status Bits ──────────────────────────────────────────────
const VIRTIO_STATUS_ACK: u8       = 1;
const VIRTIO_STATUS_DRIVER: u8    = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
const VIRTIO_STATUS_FAILED: u8    = 128;

// ── Virtio Network Feature Bits ────────────────────────────────────────────
const VIRTIO_NET_F_CSUM: u32   = 1 << 0;  // Checksum offload supported
const VIRTIO_NET_F_MAC: u32    = 1 << 5;  // Device has valid MAC address
const VIRTIO_NET_F_STATUS: u32 = 1 << 16; // Configuration status field present

// ── Virtqueue Parameters ───────────────────────────────────────────────────
const QUEUE_SIZE: usize = 16;   // Power of 2; must not exceed device's reported size
const PAGE_SIZE: usize  = 4096;
const RX_QUEUE: u16     = 0;
const TX_QUEUE: u16     = 1;

// Virtqueue memory layout (within a 2-page region):
//   Offset 0               : Descriptor table  (QUEUE_SIZE × 16 bytes = 256 bytes)
//   Offset DESC_TABLE_SIZE : Available ring     (4 + QUEUE_SIZE×2 bytes)
//   Offset PAGE_SIZE       : Used ring          (4 + QUEUE_SIZE×8 bytes)
const DESC_TABLE_SIZE: usize  = QUEUE_SIZE * 16;   // 256 bytes
const AVAIL_RING_OFF: usize   = DESC_TABLE_SIZE;   // immediately after desc table
const USED_RING_OFF: usize    = PAGE_SIZE;          // second page (4096-byte aligned)

// ── Virtio Network Header (no GSO) ────────────────────────────────────────
// Prepended to every TX frame; filled by device for RX frames.
const VirtioNetHdr = extern struct {
    flags: u8       = 0,
    gso_type: u8    = 0,  // VIRTIO_NET_HDR_GSO_NONE
    hdr_len: u16    = 0,
    gso_size: u16   = 0,
    csum_start: u16 = 0,
    csum_offset: u16= 0,
};
const NET_HDR_SIZE: usize = @sizeOf(VirtioNetHdr); // 10 bytes

// ── Virtring Descriptor ────────────────────────────────────────────────────
const VringDesc = extern struct {
    addr:  u64 = 0,
    len:   u32 = 0,
    flags: u16 = 0,
    next:  u16 = 0,
};
const VRING_DESC_F_WRITE: u16 = 2; // Device writes into this buffer (RX)

// ── IPC Structures (matching kernel cap/port layout) ──────────────────────
const CapEntry = struct {
    type: u4 = 0,
    rights: u8 = 0,
    generation: u16 = 1,
    kernel_object_ptr: u48 = 0,
    next_derived_table: ?*anyopaque = null,
    next_derived_index: u16 = 0,
    parent_table: ?*anyopaque = null,
    parent_index: u16 = 0,
    parent_generation: u16 = 0,
    old_table: ?*anyopaque = null,
    old_index: u16 = 0,
};

const Message = struct {
    msg_type: u32 = 0,
    flags: packed struct(u32) {
        expects_reply: bool = false,
        is_reply: bool = false,
        has_buffer: bool = false,
        urgent: bool = false,
        _reserved: u28 = 0,
    } = .{},
    payload: [64]u8 = [_]u8{0} ** 64,
    caps: [4]u64 = [_]u64{0} ** 4,
    buffer_cap: u64 = 0,
    transferred_caps: [4]CapEntry = [_]CapEntry{.{}} ** 4,
    transferred_buffer_cap: CapEntry = .{},
};

// ── Global Driver State ────────────────────────────────────────────────────
var mac_addr: [6]u8 = [_]u8{0} ** 6;
var irq_notif_handle: u64 = 0;
var dry_run: bool = false;
var net_stack: net.Stack = undefined;
var ipv4_stack: ip.Stack = undefined;
var net_send_ctx: u8 = 0;

// RX queue physical base (PFN written to device) and per-slot buffer addrs
var rx_queue_phys: u64 = 0;
var rx_avail_idx: u16  = 0;   // shadow of avail.idx (next slot to offer device)
var rx_last_used: u16  = 0;   // last used.idx we consumed
var rx_buf_phys: [QUEUE_SIZE]u64 = [_]u64{0} ** QUEUE_SIZE;

// TX queue physical base and single shared TX buffer
var tx_queue_phys: u64 = 0;
var tx_avail_idx: u16  = 0;   // shadow of avail.idx
var tx_last_used: u16  = 0;   // last used.idx we consumed
var tx_buf_phys: u64   = 0;

// ── MMIO Register Helpers ──────────────────────────────────────────────────

fn readReg8(off: usize) u8 {
    const p: *volatile u8 = @ptrFromInt(VIRTIO_MMIO_VADDR + off);
    return p.*;
}
fn writeReg8(off: usize, v: u8) void {
    const p: *volatile u8 = @ptrFromInt(VIRTIO_MMIO_VADDR + off);
    p.* = v;
}
fn readReg16(off: usize) u16 {
    const p: *volatile u16 = @ptrFromInt(VIRTIO_MMIO_VADDR + off);
    return p.*;
}
fn writeReg16(off: usize, v: u16) void {
    const p: *volatile u16 = @ptrFromInt(VIRTIO_MMIO_VADDR + off);
    p.* = v;
}
fn readReg32(off: usize) u32 {
    const p: *volatile u32 = @ptrFromInt(VIRTIO_MMIO_VADDR + off);
    return p.*;
}
fn writeReg32(off: usize, v: u32) void {
    const p: *volatile u32 = @ptrFromInt(VIRTIO_MMIO_VADDR + off);
    p.* = v;
}

// ── Virtqueue Accessor Helpers ─────────────────────────────────────────────
// These return volatile pointers into the mapped virtqueue pages.
// Layout per queue (2 pages starting at base_vaddr):
//   [0..256)       VringDesc table
//   [256..294)     Avail ring: flags(u16), idx(u16), ring[QUEUE_SIZE](u16)
//   [4096..4100)   Used ring:  flags(u16), idx(u16), used[QUEUE_SIZE](id:u32, len:u32)

fn descTable(base: u64) [*]volatile VringDesc {
    return @ptrFromInt(base);
}
fn availFlags(base: u64) *volatile u16 {
    return @ptrFromInt(base + AVAIL_RING_OFF);
}
fn availIdx(base: u64) *volatile u16 {
    return @ptrFromInt(base + AVAIL_RING_OFF + 2);
}
fn availRing(base: u64, i: usize) *volatile u16 {
    return @ptrFromInt(base + AVAIL_RING_OFF + 4 + i * 2);
}
fn usedFlags(base: u64) *volatile u16 {
    return @ptrFromInt(base + USED_RING_OFF);
}
fn usedIdx(base: u64) *volatile u16 {
    return @ptrFromInt(base + USED_RING_OFF + 2);
}
fn usedId(base: u64, i: usize) *volatile u32 {
    return @ptrFromInt(base + USED_RING_OFF + 4 + i * 8);
}
fn usedLen(base: u64, i: usize) *volatile u32 {
    return @ptrFromInt(base + USED_RING_OFF + 4 + i * 8 + 4);
}

// ── Virtqueue Setup ────────────────────────────────────────────────────────
// Selects the queue, zeroes its pages, writes the PFN to the device.
fn setupVirtqueue(q_idx: u16, q_vaddr: u64, q_phys: u64) bool {
    writeReg16(REG_QUEUE_SEL, q_idx);
    const dev_size = readReg16(REG_QUEUE_SIZE);
    if (dev_size == 0) return false;

    // Zero both pages (desc+avail page and used page)
    const mem: [*]u8 = @ptrFromInt(q_vaddr);
    @memset(mem[0 .. PAGE_SIZE * 2], 0);

    // Write queue PFN (physical address of desc table start, in pages)
    writeReg32(REG_QUEUE_PFN, @truncate(q_phys >> 12));
    return true;
}

// ── Frame Transmit ─────────────────────────────────────────────────────────
// Copies `frame` into the TX buffer (with virtio header), submits to TX queue,
// kicks the device, then polls the TX used ring for completion.
fn sendFrameInternal(frame: []const u8) bool {
    if (dry_run) return true;
    if (frame.len == 0 or frame.len > PAGE_SIZE - NET_HDR_SIZE) return false;

    // Write virtio-net header (all zeros = no GSO/checksum offload)
    const tx_vaddr = TX_BUF_VADDR;
    const hdr: *volatile VirtioNetHdr = @ptrFromInt(tx_vaddr);
    hdr.* = .{};

    // Copy frame data after the header
    const frame_dst: [*]u8 = @ptrFromInt(tx_vaddr + NET_HDR_SIZE);
    @memcpy(frame_dst[0..frame.len], frame);

    // Configure descriptor 0: points to tx_buf_phys, length = hdr + frame
    const descs = descTable(TX_QUEUE_VADDR);
    descs[0].addr  = tx_buf_phys;
    descs[0].len   = @truncate(NET_HDR_SIZE + frame.len);
    descs[0].flags = 0; // device reads (TX)
    descs[0].next  = 0;

    // Add descriptor 0 to the avail ring
    const slot = tx_avail_idx % QUEUE_SIZE;
    availRing(TX_QUEUE_VADDR, slot).* = 0; // desc index 0
    // Memory barrier: ensure descriptor write is visible before advancing idx.
    var _fence_dummy: u32 = 0;
    @atomicStore(u32, &_fence_dummy, 0, .seq_cst);
    tx_avail_idx +%= 1;
    availIdx(TX_QUEUE_VADDR).* = tx_avail_idx;

    // Kick the device
    writeReg16(REG_QUEUE_NOTIFY, TX_QUEUE);

    // Poll TX used ring for completion (device confirms frame consumed).
    // Fast path: typically < 1ms for local virtio.
    var timeout: usize = 0;
    while (timeout < 100_000) : (timeout += 1) {
        const used_i = @atomicLoad(u16, usedIdx(TX_QUEUE_VADDR), .acquire);
        if (used_i != tx_last_used) {
            tx_last_used = used_i;
            // Clear ISR (also done on IRQ handler path)
            _ = readReg8(REG_ISR);
            return true;
        }
        asm volatile ("pause");
    }
    // Timeout — device did not consume frame
    return false;
}

// ── Frame Receive ──────────────────────────────────────────────────────────
// Waits (via IRQ notification or fallback polling) for the device to deposit
// a frame in the RX used ring, then copies it to `buf`.
// Returns the number of bytes written to `buf` (0 = no frame / error).
fn recvFrameInternal(buf: []u8) usize {
    if (dry_run) return 0;
    // Wait for IRQ notification (blocks thread yielding to scheduler)
    if (irq_notif_handle != 0) {
        _ = libvanta.vanta_cap_wait(irq_notif_handle, 1);
    } else {
        // Fallback: spin until RX used ring advances
        var spin: usize = 0;
        while (spin < 10_000_000) : (spin += 1) {
            const u = @atomicLoad(u16, usedIdx(RX_QUEUE_VADDR), .acquire);
            if (u != rx_last_used) break;
            asm volatile ("pause");
        }
    }

    // Clear ISR register (acknowledges the interrupt)
    _ = readReg8(REG_ISR);

    // Check RX used ring for a new entry
    const used_i = @atomicLoad(u16, usedIdx(RX_QUEUE_VADDR), .acquire);
    if (used_i == rx_last_used) return 0; // spurious wake

    // Consume one used entry
    const used_slot: usize = rx_last_used % QUEUE_SIZE;
    const desc_idx: usize  = usedId(RX_QUEUE_VADDR, used_slot).*;
    const written: u32     = usedLen(RX_QUEUE_VADDR, used_slot).*;
    rx_last_used +%= 1;

    // Frame data in RX buffer (after virtio-net header)
    const frame_len = if (written > NET_HDR_SIZE) written - NET_HDR_SIZE else 0;
    if (frame_len == 0 or desc_idx >= QUEUE_SIZE) {
        rearmRxDesc(desc_idx);
        return 0;
    }

    const copy_len = @min(frame_len, buf.len);
    const src: [*]const u8 = @ptrFromInt(RX_BUF_VADDR + desc_idx * PAGE_SIZE + NET_HDR_SIZE);
    @memcpy(buf[0..copy_len], src[0..copy_len]);

    // Re-arm the descriptor: return it to the avail ring
    rearmRxDesc(desc_idx);

    return copy_len;
}

fn rearmRxDesc(desc_idx: usize) void {
    // Descriptor already points to correct buffer; just re-add to avail ring
    const slot = rx_avail_idx % QUEUE_SIZE;
    availRing(RX_QUEUE_VADDR, slot).* = @truncate(desc_idx);
    var _fence_dummy2: u32 = 0;
    @atomicStore(u32, &_fence_dummy2, 0, .seq_cst);
    rx_avail_idx +%= 1;
    availIdx(RX_QUEUE_VADDR).* = rx_avail_idx;
    // Kick RX queue
    writeReg16(REG_QUEUE_NOTIFY, RX_QUEUE);
}

// ── Ethernet & ARP Implementation ──────────────────────────────────────────
const OUR_IP: net.Ip4 = .{ 10, 0, 2, 15 };

fn getTimeNs() u64 {
    var low: u32 = 0;
    var high: u32 = 0;
    asm volatile ("rdtsc" : [low] "={eax}" (low), [high] "={edx}" (high));
    // Temporary calibration: existing P8 code assumes a ~2 GHz TSC.
    return (((@as(u64, high) << 32) | low) / 2);
}

fn sendRawEthernetFrame(_: *anyopaque, frame: []const u8) bool {
    return sendFrameInternal(frame);
}

fn sendRawIpv4Packet(_: *anyopaque, dst_ip: net.Ip4, packet: []const u8) bool {
    const next_hop = routeIpv4(dst_ip);
    if (arpCacheGet(next_hop)) |dst_mac| {
        return send_ethernet_frame(dst_mac, net.ETHERTYPE_IPV4, packet);
    }
    sendArpRequest(next_hop);
    return false;
}

fn routeIpv4(dst_ip: net.Ip4) net.Ip4 {
    if (dst_ip[0] == OUR_IP[0] and dst_ip[1] == OUR_IP[1] and dst_ip[2] == OUR_IP[2]) {
        return dst_ip;
    }
    return .{ 10, 0, 2, 2 };
}

fn arpCacheGet(addr: net.Ip4) ?net.Mac {
    return net_stack.cache.get(addr, getTimeNs());
}

fn send_ethernet_frame(dst_mac: net.Mac, ethertype: u16, payload: []const u8) bool {
    return net_stack.sendEthernetFrame(dst_mac, ethertype, payload);
}

fn sendArpRequest(target_ip: net.Ip4) void {
    _ = net_stack.sendArpRequest(target_ip);
}

fn handleReceivedFrame(frame: []const u8) ?[]const u8 {
    const now_ns = getTimeNs();
    const ipv4_packet = net_stack.handleReceivedFrame(frame, now_ns) orelse return null;
    const datagram = ipv4_stack.handleIpv4Packet(ipv4_packet, now_ns) orelse return null;
    return datagram.payload;
}

// ── IPC Error Reply ────────────────────────────────────────────────────────
fn sendErrorReply(msg: *const Message) void {
    if (!msg.flags.expects_reply) return;
    var reply = Message{};
    reply.msg_type = MSG_ERROR;
    reply.flags.is_reply = true;
    @memcpy(reply.payload[0..4], "FAIL");
    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
}

// ── Main Entry Point ───────────────────────────────────────────────────────
pub export fn main() void {
    libvanta.vanta_debug_print("virtio-net: Starting virtio-net driver server (P8)...");

    // ── 1. Map BAR0 (virtio-net PCI legacy MMIO registers) ────────────
    libvanta.vanta_debug_print("virtio-net: Mapping BAR0 MMIO...");
    const mmio_err = libvanta.vanta_mem_map(PCI_CAP_HANDLE, VIRTIO_MMIO_VADDR, 1);
    if (mmio_err != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Failed to map BAR0!");
        libvanta.vanta_exit(1);
    }
    libvanta.vanta_debug_print("virtio-net: BAR0 mapped at 0x20000000.");

    // ── 2. Device Reset & Driver Identification ────────────────────────
    writeReg8(REG_STATUS, 0);                                     // Reset
    writeReg8(REG_STATUS, VIRTIO_STATUS_ACK);                     // Acknowledge
    writeReg8(REG_STATUS, VIRTIO_STATUS_ACK | VIRTIO_STATUS_DRIVER); // Driver loading
    libvanta.vanta_debug_print("virtio-net: Device reset, ACKNOWLEDGE + DRIVER set.");

    // ── 3. Feature Negotiation ─────────────────────────────────────────
    const dev_features = readReg32(REG_HOST_FEATURES);
    var feat_buf: [96]u8 = [_]u8{0} ** 96;
    const feat_str = std.fmt.bufPrint(&feat_buf, "virtio-net: Device features: 0x{x}", .{dev_features}) catch unreachable;
    libvanta.vanta_debug_print(feat_str);

    const wanted: u32     = VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS | VIRTIO_NET_F_CSUM;
    const negotiated: u32 = dev_features & wanted;
    writeReg32(REG_GUEST_FEATURES, negotiated);
    var neg_buf: [96]u8 = [_]u8{0} ** 96;
    const neg_str = std.fmt.bufPrint(&neg_buf, "virtio-net: Negotiated features: 0x{x}", .{negotiated}) catch unreachable;
    libvanta.vanta_debug_print(neg_str);

    // ── 4. Allocate RX Virtqueue (2 physically contiguous pages) ──────
    libvanta.vanta_debug_print("virtio-net: Allocating RX virtqueue...");
    const rx_q_mem = libvanta.vanta_mem_create(2);
    if (rx_q_mem.err != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Cannot allocate RX queue memory!");
        writeReg8(REG_STATUS, VIRTIO_STATUS_FAILED);
        libvanta.vanta_exit(2);
    }
    if (libvanta.vanta_mem_map(rx_q_mem.handle, RX_QUEUE_VADDR, 2) != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Cannot map RX queue!");
        writeReg8(REG_STATUS, VIRTIO_STATUS_FAILED);
        libvanta.vanta_exit(2);
    }
    rx_queue_phys = libvanta.vanta_mem_phys(rx_q_mem.handle).phys;

    // ── 5. Allocate TX Virtqueue ───────────────────────────────────────
    libvanta.vanta_debug_print("virtio-net: Allocating TX virtqueue...");
    const tx_q_mem = libvanta.vanta_mem_create(2);
    if (tx_q_mem.err != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Cannot allocate TX queue memory!");
        writeReg8(REG_STATUS, VIRTIO_STATUS_FAILED);
        libvanta.vanta_exit(2);
    }
    if (libvanta.vanta_mem_map(tx_q_mem.handle, TX_QUEUE_VADDR, 2) != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Cannot map TX queue!");
        writeReg8(REG_STATUS, VIRTIO_STATUS_FAILED);
        libvanta.vanta_exit(2);
    }
    tx_queue_phys = libvanta.vanta_mem_phys(tx_q_mem.handle).phys;

    // ── 6. Register Both Queues with the Device ────────────────────────
    const rx_ok = setupVirtqueue(RX_QUEUE, RX_QUEUE_VADDR, rx_queue_phys);
    const tx_ok = setupVirtqueue(TX_QUEUE, TX_QUEUE_VADDR, tx_queue_phys);

    if (!rx_ok or !tx_ok) {
        libvanta.vanta_debug_print("virtio-net: [WARN] Device refused queue initialization. Falling back to dry-run mode.");
        dry_run = true;
    } else {
        libvanta.vanta_debug_print("virtio-net: RX virtqueue registered.");
        libvanta.vanta_debug_print("virtio-net: TX virtqueue registered.");
    }

    // ── 7. Allocate RX Receive Buffers (QUEUE_SIZE pages) ─────────────
    // One 4096-byte page per descriptor slot gives plenty of room for
    // the 10-byte virtio header + 1514-byte max Ethernet frame.
    libvanta.vanta_debug_print("virtio-net: Allocating RX buffers...");
    const rx_bufs_mem = libvanta.vanta_mem_create(QUEUE_SIZE);
    if (rx_bufs_mem.err != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Cannot allocate RX buffer pages!");
        writeReg8(REG_STATUS, VIRTIO_STATUS_FAILED);
        libvanta.vanta_exit(4);
    }
    if (libvanta.vanta_mem_map(rx_bufs_mem.handle, RX_BUF_VADDR, QUEUE_SIZE) != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Cannot map RX buffers!");
        writeReg8(REG_STATUS, VIRTIO_STATUS_FAILED);
        libvanta.vanta_exit(4);
    }
    const rx_buf_base = libvanta.vanta_mem_phys(rx_bufs_mem.handle).phys;
    for (0..QUEUE_SIZE) |i| {
        rx_buf_phys[i] = rx_buf_base + @as(u64, i) * PAGE_SIZE;
    }

    // ── 8. Allocate TX Buffer ──────────────────────────────────────────
    const tx_buf_mem = libvanta.vanta_mem_create(1);
    if (tx_buf_mem.err != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Cannot allocate TX buffer!");
        writeReg8(REG_STATUS, VIRTIO_STATUS_FAILED);
        libvanta.vanta_exit(4);
    }
    if (libvanta.vanta_mem_map(tx_buf_mem.handle, TX_BUF_VADDR, 1) != 0) {
        libvanta.vanta_debug_print("virtio-net: FATAL: Cannot map TX buffer!");
        writeReg8(REG_STATUS, VIRTIO_STATUS_FAILED);
        libvanta.vanta_exit(4);
    }
    tx_buf_phys = libvanta.vanta_mem_phys(tx_buf_mem.handle).phys;
    libvanta.vanta_debug_print("virtio-net: DMA buffers ready.");

    // ── 9. Pre-populate RX Descriptor Table & Available Ring ──────────
    // Every descriptor slot is offered to the device upfront so it can
    // start depositing received frames immediately.
    libvanta.vanta_debug_print("virtio-net: Pre-populating RX descriptors...");
    const rx_descs = descTable(RX_QUEUE_VADDR);
    for (0..QUEUE_SIZE) |i| {
        rx_descs[i].addr  = rx_buf_phys[i];
        rx_descs[i].len   = PAGE_SIZE;              // Accepts up to 4096 bytes
        rx_descs[i].flags = VRING_DESC_F_WRITE;     // Device writes into buffer
        rx_descs[i].next  = 0;
        availRing(RX_QUEUE_VADDR, i).* = @truncate(i); // avail.ring[i] = i
        rx_avail_idx +%= 1;
    }
    availFlags(RX_QUEUE_VADDR).* = 0;
    availIdx(RX_QUEUE_VADDR).* = rx_avail_idx;
    availFlags(TX_QUEUE_VADDR).* = 0;
    availIdx(TX_QUEUE_VADDR).* = 0;
    usedFlags(RX_QUEUE_VADDR).* = 0;
    usedFlags(TX_QUEUE_VADDR).* = 0;

    // Kick RX queue so device knows descriptors are available
    if (!dry_run) {
        writeReg16(REG_QUEUE_NOTIFY, RX_QUEUE);
        libvanta.vanta_debug_print("virtio-net: RX descriptors pre-populated, device kicked.");
    } else {
        libvanta.vanta_debug_print("virtio-net: RX descriptors pre-populated (dry-run).");
    }

    // ── 10. Set DRIVER_OK — Handshake Complete ─────────────────────────
    if (!dry_run) {
        writeReg8(REG_STATUS, VIRTIO_STATUS_ACK | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK);
        libvanta.vanta_debug_print("virtio-net: DRIVER_OK — device handshake complete.");
    } else {
        libvanta.vanta_debug_print("virtio-net: Dry-run — skipping DRIVER_OK.");
    }

    // ── 11. Bind DeviceIRQ to Notification Capability ─────────────────
    if (!dry_run) {
        libvanta.vanta_debug_print("virtio-net: Binding DeviceIRQ...");
        const notif_res = libvanta.vanta_notif_create();
        if (notif_res.err == 0) {
            irq_notif_handle = notif_res.handle;
            const bind_err = libvanta.vanta_irq_bind(IRQ_CAP_HANDLE, irq_notif_handle);
            if (bind_err == 0) {
                libvanta.vanta_debug_print("virtio-net: DeviceIRQ bound to Notification cap.");
            } else {
                libvanta.vanta_debug_print("virtio-net: IRQ bind failed — using polling fallback.");
                irq_notif_handle = 0;
            }
        } else {
            libvanta.vanta_debug_print("virtio-net: Notification cap creation failed — using polling.");
        }
    } else {
        libvanta.vanta_debug_print("virtio-net: Dry-run — skipping DeviceIRQ binding.");
    }

    // ── 12. Read MAC Address from Device Config Space ──────────────────
    if (!dry_run and (negotiated & VIRTIO_NET_F_MAC) != 0) {
        for (0..6) |i| mac_addr[i] = readReg8(REG_CONFIG + i);
        var mac_buf: [96]u8 = [_]u8{0} ** 96;
        const mac_str = std.fmt.bufPrint(&mac_buf,
            "virtio-net: MAC = {x:0>2}:{x:0>2}:{x:0>2}:{x:0>2}:{x:0>2}:{x:0>2}",
            .{ mac_addr[0], mac_addr[1], mac_addr[2],
               mac_addr[3], mac_addr[4], mac_addr[5] }) catch unreachable;
        libvanta.vanta_debug_print(mac_str);
    } else {
        // Locally administered, unicast default
        mac_addr = .{ 0x52, 0x54, 0x00, 0x0A, 0x0A, 0x01 };
        libvanta.vanta_debug_print("virtio-net: F_MAC not negotiated or dry-run; using default MAC.");
    }
    net_stack = net.Stack.init(mac_addr, OUR_IP, sendRawEthernetFrame, &net_send_ctx);
    ipv4_stack = ip.Stack.init(OUR_IP, sendRawIpv4Packet, &net_send_ctx);

    // ── 13. Register as 'hw.net.0' in the Service Registry ────────────
    libvanta.vanta_debug_print("virtio-net: Registering as 'hw.net.0'...");
    var derived_port: u64 = 0;
    const derive_err = libvanta.vanta_cap_derive(PORT_CAP_HANDLE, 3, @intFromPtr(&derived_port));
    if (derive_err != 0) {
        libvanta.vanta_debug_print("virtio-net: Failed to derive port cap for registry.");
        libvanta.vanta_exit(5);
    }
    var reg_msg = Message{};
    reg_msg.msg_type = 0x10; // RegistryRegister
    @memcpy(reg_msg.payload[0..9], "hw.net.0\x00");
    reg_msg.caps[0] = derived_port;
    const reg_err = libvanta.vanta_cap_send(REGISTRY_CAP_HANDLE, @intFromPtr(&reg_msg));
    if (reg_err != 0) {
        libvanta.vanta_debug_print("virtio-net: Registry registration failed (registry absent). Continuing.");
    } else {
        libvanta.vanta_debug_print("virtio-net: Registered as 'hw.net.0'.");
    }

    // ── 14. IPC Service Loop ───────────────────────────────────────────
    libvanta.vanta_debug_print("virtio-net: Entering IPC service loop...");
    while (true) {
        var msg = Message{};
        const recv_err = libvanta.vanta_cap_recv(PORT_CAP_HANDLE, @intFromPtr(&msg));
        if (recv_err != 0) {
            libvanta.vanta_debug_print("virtio-net: IPC recv error, retrying.");
            continue;
        }

        switch (msg.msg_type) {

            // ── MSG_NET_SEND: send_frame(shm_cap, len, [dst_mac, ethertype]) ──
            MSG_NET_SEND => {
                const ethertype = std.mem.readInt(u16, msg.payload[6..8], .little);
                var len: u32 = 0;
                
                if (ethertype != 0) {
                    len = std.mem.readInt(u32, msg.payload[8..12], .little);
                } else {
                    len = std.mem.readInt(u32, msg.payload[0..4], .little);
                }

                const shm_cap = msg.buffer_cap;

                if (shm_cap == 0 or len == 0 or len > 1500) {
                    libvanta.vanta_debug_print("virtio-net: NetSend: invalid arguments.");
                    sendErrorReply(&msg);
                    continue;
                }

                // Map caller's shared memory to read frame data
                const pages = (len + 4095) / 4096;
                if (libvanta.vanta_mem_map(shm_cap, SHM_VADDR, pages) != 0) {
                    libvanta.vanta_debug_print("virtio-net: NetSend: failed to map SHM.");
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                const frame_src: [*]const u8 = @ptrFromInt(SHM_VADDR);
                
                var ok = false;
                if (ethertype != 0) {
                    var dst_mac: [6]u8 = undefined;
                    @memcpy(&dst_mac, msg.payload[0..6]);
                    
                    if (dry_run) {
                        dst_mac = .{ 0x52, 0x54, 0x00, 0x12, 0x34, 0x56 };
                        ok = send_ethernet_frame(dst_mac, ethertype, frame_src[0..len]);
                    } else {
                        var all_zero = true;
                        for (dst_mac) |b| {
                            if (b != 0) {
                                all_zero = false;
                                break;
                            }
                        }
                        if (ethertype == 0x0800 and all_zero) {
                            if (len >= 20) {
                                var dest_ip: [4]u8 = undefined;
                                @memcpy(&dest_ip, frame_src[16..20]);
                                
                                if (arpCacheGet(dest_ip)) |cached_mac| {
                                    dst_mac = cached_mac;
                                } else {
                                    libvanta.vanta_debug_print("virtio-net: ARP cache miss, sending broadcast ARP request...");
                                    sendArpRequest(dest_ip);
                                    
                                    // Wait/pump for the reply (up to 100ms or 50 attempts)
                                    var resolved = false;
                                    var spin: usize = 0;
                                    while (spin < 50) : (spin += 1) {
                                        var raw_frame_buf: [1518]u8 = undefined;
                                        const raw_len = recvFrameInternal(&raw_frame_buf);
                                        if (raw_len > 0) {
                                            _ = handleReceivedFrame(raw_frame_buf[0..raw_len]);
                                        }
                                        if (arpCacheGet(dest_ip)) |cached_mac| {
                                            dst_mac = cached_mac;
                                            resolved = true;
                                            break;
                                        }
                                        var delay: usize = 0;
                                        while (delay < 1000) : (delay += 1) {
                                            asm volatile ("pause");
                                        }
                                    }
                                    if (!resolved) {
                                        libvanta.vanta_debug_print("virtio-net: ARP resolution failed (timeout).");
                                        _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                                        _ = libvanta.vanta_cap_revoke(shm_cap);
                                        sendErrorReply(&msg);
                                        continue;
                                    }
                                }
                            }
                        }
                        ok = send_ethernet_frame(dst_mac, ethertype, frame_src[0..len]);
                    }
                } else {
                    ok = sendFrameInternal(frame_src[0..len]);
                }

                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    if (ok) {
                        reply.msg_type = MSG_NET_SEND;
                        reply.flags.is_reply = true;
                        @memcpy(reply.payload[0..4], "OKAY");
                    } else {
                        reply.msg_type = MSG_ERROR;
                        reply.flags.is_reply = true;
                        @memcpy(reply.payload[0..4], "FAIL");
                    }
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },

            // ── MSG_NET_RECV: recv_frame(shm_cap, max_len) → bytes ─────
            MSG_NET_RECV => {
                const max_len = std.mem.readInt(u64, msg.payload[0..8], .little);
                const shm_cap = msg.buffer_cap;

                if (shm_cap == 0 or max_len == 0) {
                    libvanta.vanta_debug_print("virtio-net: NetRecv: invalid arguments.");
                    sendErrorReply(&msg);
                    continue;
                }

                // Map caller's shared memory as the receive destination
                const pages = (max_len + 4095) / 4096;
                if (libvanta.vanta_mem_map(shm_cap, SHM_VADDR, pages) != 0) {
                    libvanta.vanta_debug_print("virtio-net: NetRecv: failed to map SHM.");
                    _ = libvanta.vanta_cap_revoke(shm_cap);
                    sendErrorReply(&msg);
                    continue;
                }

                const dst_buf: [*]u8 = @ptrFromInt(SHM_VADDR);
                
                var bytes_written: usize = 0;
                var raw_frame_buf: [1518]u8 = undefined;

                while (true) {
                    const raw_len = recvFrameInternal(&raw_frame_buf);
                    if (raw_len == 0) {
                        break;
                    }

                    if (handleReceivedFrame(raw_frame_buf[0..raw_len])) |ipv4_payload| {
                        const copy_len = @min(ipv4_payload.len, max_len);
                        @memcpy(dst_buf[0..copy_len], ipv4_payload[0..copy_len]);
                        bytes_written = copy_len;
                        break;
                    }
                }

                _ = libvanta.vanta_mem_unmap(SHM_VADDR);
                _ = libvanta.vanta_cap_revoke(shm_cap);

                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_NET_RECV;
                    reply.flags.is_reply = true;
                    std.mem.writeInt(u64, reply.payload[0..8], bytes_written, .little);
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },

            // ── MSG_NET_INFO: returns MAC address ──────────────────────
            MSG_NET_INFO => {
                if (msg.flags.expects_reply) {
                    var reply = Message{};
                    reply.msg_type = MSG_NET_INFO;
                    reply.flags.is_reply = true;
                    @memcpy(reply.payload[0..6], &mac_addr);
                    _ = libvanta.vanta_cap_send(PORT_CAP_HANDLE, @intFromPtr(&reply));
                }
            },

            else => {
                libvanta.vanta_debug_print("virtio-net: Unknown message type, ignoring.");
                sendErrorReply(&msg);
            },
        }
    }
}
