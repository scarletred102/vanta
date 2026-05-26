// VantaOS — compositor server (sys.compositor)
// Self-paced at ~60 Hz using vanta_cap_poll with 16 ms timeout.
// Manages up to 16 surfaces; blits them in z-order to the framebuffer.
// Protocol: see COMPOSITOR_PROTOCOL.md

const lib = @import("../libvanta/libvanta.zig");

// ── Cap slot constants (injected by kernel) ──────────────────────────
// slot 1: MemoryCap — Limine linear framebuffer
// slot 2: endpoint — registry
// slot 3: endpoint — compositor's own server port
const FB_MEM_CAP: lib.Handle = 0x0001000000000001;
const REGISTRY_CAP: lib.Handle = 0x0001000000000002;
const PORT_CAP: lib.Handle = 0x0001000000000003;

// ── Message codes ─────────────────────────────────────────────────────
const MSG_CREATE_SURFACE: u32 = 0x30;
const MSG_SWAP_BUFFERS: u32 = 0x31;
const MSG_SET_POSITION: u32 = 0x32;
const MSG_SET_ZORDER: u32 = 0x33;
const MSG_DESTROY_SURFACE: u32 = 0x34;
const MSG_QUERY_DISPLAY: u32 = 0x35;

// ── Virtual address layout ───────────────────────────────────────────
const FB_VADDR: u64 = 0x50000000;       // Limine/virtio-gpu framebuffer
const SURFACE_BASE: u64 = 0x60000000;   // client surface backing buffers
const SURFACE_STRIDE: u64 = 0x01000000; // 16 MB per surface slot (max 4096×4096 BGRA8)

// ── Framebuffer metadata ─────────────────────────────────────────────
// Kernel writes [width: u32, height: u32, stride: u32, format: u32]
// at the very start of the FB MemoryCap before mapping it.
var fb_width: u32 = 1024;
var fb_height: u32 = 768;
var fb_stride: u32 = 0; // bytes per row; filled in from cap metadata

// ── Surface table ────────────────────────────────────────────────────
const MAX_SURFACES: usize = 16;

const Surface = struct {
    id: u64 = 0,
    width: u32 = 0,
    height: u32 = 0,
    x: i32 = 0,
    y: i32 = 0,
    z: i64 = 0,
    vaddr: u64 = 0,   // virtual address of pixel backing (BGRA8)
    active: bool = false,
};

var surfaces: [MAX_SURFACES]Surface = [_]Surface{.{}} ** MAX_SURFACES;
var next_surface_id: u64 = 1;
var fb_ptr: [*]u32 = undefined; // BGRA8 pixels, row-major

// ── Helpers ──────────────────────────────────────────────────────────

fn findSurface(id: u64) ?*Surface {
    for (&surfaces) |*s| {
        if (s.active and s.id == id) return s;
    }
    return null;
}

fn allocSurface() ?*Surface {
    for (&surfaces) |*s| {
        if (!s.active) return s;
    }
    return null;
}

// Sort surfaces by z-order in place (simple insertion sort, 16 elements).
fn sortByZ() void {
    var i: usize = 1;
    while (i < MAX_SURFACES) : (i += 1) {
        const key = surfaces[i];
        var j: usize = i;
        while (j > 0 and surfaces[j - 1].z > key.z) : (j -= 1) {
            surfaces[j] = surfaces[j - 1];
        }
        surfaces[j] = key;
    }
}

// Blit one surface onto the framebuffer, clipping to screen bounds.
fn blitSurface(s: *const Surface) void {
    if (!s.active or s.vaddr == 0) return;
    const src = @as([*]const u32, @ptrFromInt(s.vaddr));
    const sw: i32 = @intCast(s.width);
    const sh: i32 = @intCast(s.height);
    const fw: i32 = @intCast(fb_width);
    const fh: i32 = @intCast(fb_height);

    var dy: i32 = 0;
    while (dy < sh) : (dy += 1) {
        const fy = s.y + dy;
        if (fy < 0 or fy >= fh) continue;
        var dx: i32 = 0;
        while (dx < sw) : (dx += 1) {
            const fx = s.x + dx;
            if (fx < 0 or fx >= fw) continue;
            const pixel = src[@as(u32, @intCast(dy)) * s.width + @as(u32, @intCast(dx))];
            // Alpha blend: if alpha == 0xFF, overwrite; otherwise skip for speed
            if ((pixel >> 24) != 0) {
                fb_ptr[@as(u32, @intCast(fy)) * (fb_stride / 4) + @as(u32, @intCast(fx))] = pixel;
            }
        }
    }
}

// Clear framebuffer to black
fn clearFb() void {
    const total = (fb_stride / 4) * fb_height;
    for (0..total) |i| fb_ptr[i] = 0xFF000000;
}

// Composite all active surfaces in z-order
fn composite() void {
    sortByZ();
    clearFb();
    for (&surfaces) |*s| {
        if (s.active) blitSurface(s);
    }
}

// ── Service registration ─────────────────────────────────────────────
fn registerService() void {
    var msg: lib.Message = .{};
    msg.msg_type = 0x10; // NS_REGISTER
    const name = "sys.compositor";
    for (name, 0..) |c, i| msg.payload[i] = c;
    // Derive a send cap so we keep PORT_CAP for our own service loop
    var send_cap: lib.Handle = 0;
    _ = lib.vanta_cap_derive(PORT_CAP, 7, @intFromPtr(&send_cap));
    msg.caps[0] = send_cap;
    _ = lib.vanta_cap_send(REGISTRY_CAP, @intFromPtr(&msg));
}

// ── Message handlers ──────────────────────────────────────────────────
fn handleCreateSurface(msg: *const lib.Message) lib.Message {
    const width = @as(*align(1) const u32, @ptrCast(&msg.payload[0])).*;
    const height = @as(*align(1) const u32, @ptrCast(&msg.payload[4])).*;

    var reply: lib.Message = .{};
    reply.msg_type = MSG_CREATE_SURFACE | 0x8000;

    const s = allocSurface() orelse {
        reply.payload[0] = 0xFF; // error: no free slots
        return reply;
    };

    const slot_idx: u64 = (@intFromPtr(s) - @intFromPtr(&surfaces[0])) / @sizeOf(Surface);

    const backing_vaddr = SURFACE_BASE + slot_idx * SURFACE_STRIDE;
    // Allocate physical pages for this surface's pixel backing
    const pages_needed = (width * height * 4 + 4095) / 4096;
    const shm = lib.vanta_shm_create(pages_needed);
    if (shm.err != 0) {
        reply.payload[0] = 0xFE;
        return reply;
    }
    const map_err = lib.vanta_shm_map(shm.handle, backing_vaddr);
    if (map_err != 0) {
        reply.payload[0] = 0xFD;
        return reply;
    }

    s.* = .{
        .id = next_surface_id,
        .width = width,
        .height = height,
        .x = 0,
        .y = 0,
        .z = 0,
        .vaddr = backing_vaddr,
        .active = true,
    };
    next_surface_id += 1;

    // Return the SHM cap handle as a cap so the client can map it
    reply.caps[0] = shm.handle;
    @as(*align(1) u64, @ptrCast(&reply.payload[0])).* = s.id;
    return reply;
}

fn handleSwapBuffers(msg: *const lib.Message) void {
    const surface_id = @as(*align(1) const u64, @ptrCast(&msg.payload[0])).*;
    const s = findSurface(surface_id) orelse return;
    // buffer_cap contains the new SHM cap; map it at the surface's vaddr
    const shm = msg.buffer_cap;
    if (shm != 0) {
        _ = lib.vanta_shm_map(shm, s.vaddr);
    }
    // Compositor will use s.vaddr on next vsync
}

fn handleSetPosition(msg: *const lib.Message) void {
    const surface_id = @as(*align(1) const u64, @ptrCast(&msg.payload[0])).*;
    const x = @as(*align(1) const i32, @ptrCast(&msg.payload[8])).*;
    const y = @as(*align(1) const i32, @ptrCast(&msg.payload[12])).*;
    if (findSurface(surface_id)) |s| {
        s.x = x;
        s.y = y;
    }
}

fn handleSetZOrder(msg: *const lib.Message) void {
    const surface_id = @as(*align(1) const u64, @ptrCast(&msg.payload[0])).*;
    const z = @as(*align(1) const i64, @ptrCast(&msg.payload[8])).*;
    if (findSurface(surface_id)) |s| {
        s.z = z;
    }
}

fn handleDestroySurface(msg: *const lib.Message) void {
    const surface_id = @as(*align(1) const u64, @ptrCast(&msg.payload[0])).*;
    if (findSurface(surface_id)) |s| {
        s.active = false;
        s.vaddr = 0;
    }
}

fn handleQueryDisplay() lib.Message {
    var reply: lib.Message = .{};
    reply.msg_type = MSG_QUERY_DISPLAY | 0x8000;
    @as(*align(1) u32, @ptrCast(&reply.payload[0])).* = fb_width;
    @as(*align(1) u32, @ptrCast(&reply.payload[4])).* = fb_height;
    return reply;
}

pub export fn main() void {
    lib.vanta_debug_print("[COMP] compositor starting\n");

    // Map Limine framebuffer (up to 8 MB = 2048 pages covers 4K×4K BGRA8)
    _ = lib.vanta_mem_map(FB_MEM_CAP, FB_VADDR, 512);
    fb_ptr = @as([*]u32, @ptrFromInt(FB_VADDR));

    // Discover display dimensions from hw.display.0 (retry loop)
    {
        var lookup_msg: lib.Message = .{};
        lookup_msg.msg_type = 0x11; // NS_LOOKUP
        const hw_name = "hw.display.0";
        for (hw_name, 0..) |c, i| lookup_msg.payload[i] = c;
        var attempts: u32 = 0;
        while (attempts < 500) : (attempts += 1) {
            var lookup_reply: lib.Message = .{};
            _ = lib.vanta_cap_call(REGISTRY_CAP, @intFromPtr(&lookup_msg), @intFromPtr(&lookup_reply));
            const display_cap = lookup_reply.caps[0];
            if (display_cap != 0) {
                var info_msg: lib.Message = .{};
                info_msg.msg_type = 0x40; // MSG_GET_FB_INFO
                var info_reply: lib.Message = .{};
                _ = lib.vanta_cap_call(display_cap, @intFromPtr(&info_msg), @intFromPtr(&info_reply));
                const w = @as(*align(1) u32, @ptrCast(&info_reply.payload[0])).*;
                const h = @as(*align(1) u32, @ptrCast(&info_reply.payload[4])).*;
                const s = @as(*align(1) u32, @ptrCast(&info_reply.payload[8])).*;
                if (w > 0 and w < 8192 and h > 0) {
                    fb_width = w;
                    fb_height = h;
                    fb_stride = if (s > 0) s else w * 4;
                }
                break;
            }
            var spin: u32 = 0;
            while (spin < 100000) : (spin += 1) asm volatile ("pause");
        }
    }
    if (fb_stride == 0) fb_stride = fb_width * 4;

    registerService();

    lib.vanta_debug_print("[COMP] ready, entering vsync loop\n");

    var poll_handles: [1]u64 = .{PORT_CAP};
    const VSYNC_MS: i64 = 16;

    while (true) {
        // Wait up to 16 ms for an incoming message
        const poll_res = lib.vanta_cap_poll(@intFromPtr(&poll_handles), 1, VSYNC_MS);

        if (poll_res.idx == 0) {
            // Message arrived — drain all pending messages without blocking
            var drain_iters: u32 = 0;
            while (drain_iters < 64) : (drain_iters += 1) {
                var msg: lib.Message = .{};
                const recv_err = lib.vanta_cap_recv(PORT_CAP, @intFromPtr(&msg));
                if (recv_err != 0) break;

                switch (msg.msg_type) {
                    MSG_CREATE_SURFACE => {
                        const reply = handleCreateSurface(&msg);
                        _ = lib.vanta_cap_send(PORT_CAP, @intFromPtr(&reply));
                    },
                    MSG_SWAP_BUFFERS => handleSwapBuffers(&msg),
                    MSG_SET_POSITION => handleSetPosition(&msg),
                    MSG_SET_ZORDER => handleSetZOrder(&msg),
                    MSG_DESTROY_SURFACE => handleDestroySurface(&msg),
                    MSG_QUERY_DISPLAY => {
                        const reply = handleQueryDisplay();
                        _ = lib.vanta_cap_send(PORT_CAP, @intFromPtr(&reply));
                    },
                    else => {},
                }
            }
        }
        // Vsync boundary: composite all surfaces to framebuffer
        composite();
    }
}
