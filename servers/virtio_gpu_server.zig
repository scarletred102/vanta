// VantaOS — virtio-gpu / display info server
// Registers as "hw.display.0" and answers MSG_GET_FB_INFO queries.
// The compositor writes directly to the Limine linear framebuffer;
// this server is the authoritative source of display dimensions.

const lib = @import("libvanta");

// ── Cap slot constants ────────────────────────────────────────────────
// slot 1: MemoryCap — virtio-gpu BAR0 MMIO (or dummy page if no GPU)
// slot 2: MemoryCap — Limine framebuffer
// slot 3: endpoint — registry
// slot 4: endpoint — our own server port
const MMIO_MEM_CAP: lib.Handle = 0x0001000000000001;
const FB_MEM_CAP: lib.Handle = 0x0001000000000002;
const REGISTRY_CAP: lib.Handle = 0x0001000000000003;
const PORT_CAP: lib.Handle = 0x0001000000000004;

// ── Message codes ─────────────────────────────────────────────────────
const MSG_GET_FB_INFO: u32 = 0x40; // reply: [0..4]=w [4..8]=h [8..12]=stride [12..16]=fmt
const MSG_FLUSH: u32 = 0x41;       // no-op: compositor owns the framebuffer

// ── Virtual address layout ────────────────────────────────────────────
const MMIO_VADDR: u64 = 0x50000000;

// ── virtio-gpu MMIO magic check ───────────────────────────────────────
const VIRTIO_MMIO_MAGIC: u64 = 0x000;
const VIRTIO_MMIO_DEVICE_ID: u64 = 0x008;

var display_w: u32 = 1024;
var display_h: u32 = 768;
var display_stride: u32 = 1024 * 4;

fn mmioRead32(off: u64) u32 {
    const ptr = @as(*const volatile u32, @ptrFromInt(MMIO_VADDR + off));
    return ptr.*;
}

fn detectVirtioGpu() void {
    const map_err = lib.vanta_mem_map(MMIO_MEM_CAP, MMIO_VADDR, 1);
    if (map_err != 0) return;
    const magic = mmioRead32(VIRTIO_MMIO_MAGIC);
    if (magic == 0x74726976) { // "virt" LE
        const dev_id = mmioRead32(VIRTIO_MMIO_DEVICE_ID);
        if (dev_id == 16) {
            lib.vanta_debug_print("[GPU] virtio-gpu MMIO detected\n");
            return;
        }
    }
    // No real virtio-gpu — check if kernel wrote Limine FB metadata
    // into the dummy page.  Layout: [0]=0xFB01, [4]=w, [8]=h, [12]=stride
    if (magic == 0xFB01) {
        const w = mmioRead32(4);
        const h = mmioRead32(8);
        const s = mmioRead32(12);
        if (w > 0 and w < 8192 and h > 0 and s > 0) {
            display_w = w;
            display_h = h;
            display_stride = s;
            lib.vanta_debug_print("[GPU] using Limine FB dimensions\n");
        }
    }
}

fn registerService() void {
    var msg: lib.Message = .{};
    msg.msg_type = 0x10; // NS_REGISTER
    const name = "hw.display.0";
    for (name, 0..) |c, i| msg.payload[i] = c;
    // Derive a send cap so we keep PORT_CAP for our own service loop
    var send_cap: lib.Handle = 0;
    _ = lib.vanta_cap_derive(PORT_CAP, 7, @intFromPtr(&send_cap));
    msg.caps[0] = send_cap;
    _ = lib.vanta_cap_send(REGISTRY_CAP, @intFromPtr(&msg));
}

pub export fn main() void {
    lib.vanta_debug_print("[GPU] display server starting\n");
    detectVirtioGpu();
    registerService();
    lib.vanta_debug_print("[GPU] registered as hw.display.0\n");

    while (true) {
        var msg: lib.Message = .{};
        const err = lib.vanta_cap_recv(PORT_CAP, @intFromPtr(&msg));
        if (err != 0) continue;

        switch (msg.msg_type) {
            MSG_GET_FB_INFO => {
                var reply: lib.Message = .{};
                reply.msg_type = MSG_GET_FB_INFO | 0x8000;
                reply.flags.is_reply = true;
                const pw = @as(*align(1) u32, @ptrCast(&reply.payload[0]));
                const ph = @as(*align(1) u32, @ptrCast(&reply.payload[4]));
                const ps = @as(*align(1) u32, @ptrCast(&reply.payload[8]));
                const pf = @as(*align(1) u32, @ptrCast(&reply.payload[12]));
                pw.* = display_w;
                ph.* = display_h;
                ps.* = display_stride;
                pf.* = 1; // BGRA8
                _ = lib.vanta_cap_send(PORT_CAP, @intFromPtr(&reply));
            },
            MSG_FLUSH => {}, // compositor handles its own blitting
            else => {},
        }
    }
}
