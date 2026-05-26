// VantaOS — terminal emulator
// 80×25 VT100/ANSI terminal on a compositor surface.
// Connects to sys.compositor (display), sys.input (keyboard), sys.pty (I/O).

const lib = @import("../libvanta/libvanta.zig");
const font = @import("bitmap_font.zig");
const atlas = @import("glyph_atlas.zig");

// ── Cap slots (injected by kernel) ───────────────────────────────────
// slot 1: registry endpoint
const REGISTRY_CAP: lib.Handle = 0x0001000000000001;
// Created dynamically in main()
var NOTIF_CAP: lib.Handle = 0;

// ── Terminal dimensions ───────────────────────────────────────────────
const COLS: u32 = 80;
const ROWS: u32 = 25;
const CELL_W: u32 = font.GLYPH_W;
const CELL_H: u32 = font.GLYPH_H;
const SURF_W: u32 = COLS * CELL_W; // 640
const SURF_H: u32 = ROWS * CELL_H; // 400

// ── Colours (BGRA8) ───────────────────────────────────────────────────
const COLOR_BG: u32 = 0xFF1A1A2E;  // dark navy
const COLOR_FG: u32 = 0xFFE0E0E0;  // light grey
const COLOR_CURSOR: u32 = 0xFF00FF88; // green cursor

// ── Cell buffer ───────────────────────────────────────────────────────
const Cell = struct {
    ch: u8 = ' ',
    fg: u32 = COLOR_FG,
    bg: u32 = COLOR_BG,
    dirty: bool = true,
};

var cells: [ROWS][COLS]Cell = [_][COLS]Cell{[_]Cell{.{}} ** COLS} ** ROWS;
var cursor_row: u32 = 0;
var cursor_col: u32 = 0;
var cursor_visible: bool = true;

// ── ANSI parser state ─────────────────────────────────────────────────
const ParseState = enum { normal, esc, csi };
var parse_state: ParseState = .normal;
var csi_params: [8]u32 = [_]u32{0} ** 8;
var csi_param_count: u32 = 0;
var csi_buf: [32]u8 = [_]u8{0} ** 32;
var csi_len: u32 = 0;

// ── Surface framebuffer ───────────────────────────────────────────────
// Mapped at SURFACE_VADDR after CreateSurface
const SURFACE_VADDR: u64 = 0x60000000;
var surface_id: u64 = 0;
var surf_pixels: [*]u32 = undefined;

// ── Service discovery caps ────────────────────────────────────────────
var compositor_cap: lib.Handle = 0;
var input_cap: lib.Handle = 0;
var pty_cap: lib.Handle = 0;

// ── Registry helpers ──────────────────────────────────────────────────

fn registryLookup(name: []const u8) lib.Handle {
    var msg: lib.Message = .{};
    msg.msg_type = 0x11; // NS_LOOKUP
    for (name, 0..) |c, i| {
        if (i >= 32) break;
        msg.payload[i] = c;
    }
    // Busy-retry for up to ~2000 attempts (services may not be up yet)
    var attempts: u32 = 0;
    while (attempts < 2000) : (attempts += 1) {
        var reply: lib.Message = .{};
        _ = lib.vanta_cap_call(REGISTRY_CAP, @intFromPtr(&msg), @intFromPtr(&reply));
        // Registry inserts the found cap into our table and returns handle in caps[0]
        if (reply.msg_type == 0x11 and reply.caps[0] != 0) {
            return reply.caps[0];
        }
        var spin: u32 = 0;
        while (spin < 50000) : (spin += 1) asm volatile ("pause");
    }
    return 0;
}

// ── Compositor helpers ────────────────────────────────────────────────

fn createSurface() void {
    var msg: lib.Message = .{};
    msg.msg_type = 0x30; // MSG_CREATE_SURFACE
    @as(*align(1) u32, @ptrCast(&msg.payload[0])).* = SURF_W;
    @as(*align(1) u32, @ptrCast(&msg.payload[4])).* = SURF_H;
    var reply: lib.Message = .{};
    _ = lib.vanta_cap_call(compositor_cap, @intFromPtr(&msg), @intFromPtr(&reply));
    if (reply.msg_type == (0x30 | 0x8000)) {
        surface_id = @as(*align(1) u64, @ptrCast(&reply.payload[0])).*;
        // Map the SHM cap from the reply so we can write pixels
        const shm = reply.caps[0];
        if (shm != 0) {
            _ = lib.vanta_shm_map(shm, SURFACE_VADDR);
        }
    }
    surf_pixels = @as([*]u32, @ptrFromInt(SURFACE_VADDR));
}

fn swapBuffers() void {
    var msg: lib.Message = .{};
    msg.msg_type = 0x31; // MSG_SWAP_BUFFERS
    @as(*align(1) u64, @ptrCast(&msg.payload[0])).* = surface_id;
    _ = lib.vanta_cap_send(compositor_cap, @intFromPtr(&msg));
}

fn setPosition(x: i32, y: i32) void {
    var msg: lib.Message = .{};
    msg.msg_type = 0x32;
    @as(*align(1) u64, @ptrCast(&msg.payload[0])).* = surface_id;
    @as(*align(1) i32, @ptrCast(&msg.payload[8])).* = x;
    @as(*align(1) i32, @ptrCast(&msg.payload[12])).* = y;
    _ = lib.vanta_cap_send(compositor_cap, @intFromPtr(&msg));
}

// ── Input registration ────────────────────────────────────────────────

fn registerForKeyEvents() void {
    var msg: lib.Message = .{};
    msg.msg_type = 0x50; // MSG_SET_FOCUS_CAP
    msg.payload[0] = 1;  // key events
    msg.caps[0] = NOTIF_CAP;
    _ = lib.vanta_cap_send(input_cap, @intFromPtr(&msg));
}

// ── PTY helpers ───────────────────────────────────────────────────────

fn ptyWrite(data: []const u8) void {
    if (pty_cap == 0) return;
    var msg: lib.Message = .{};
    msg.msg_type = 0x41; // MSG_PTY_WRITE
    @as(*align(1) u32, @ptrCast(&msg.payload[0])).* = 0; // FD_MASTER
    const n = @min(data.len, 56);
    @as(*align(1) u32, @ptrCast(&msg.payload[4])).* = @intCast(n);
    @memcpy(msg.payload[8..8 + n], data[0..n]);
    _ = lib.vanta_cap_send(pty_cap, @intFromPtr(&msg));
}

fn ptyRead(out: []u8) usize {
    if (pty_cap == 0) return 0;
    var msg: lib.Message = .{};
    msg.msg_type = 0x42; // MSG_PTY_READ
    msg.flags.expects_reply = true;
    @as(*align(1) u32, @ptrCast(&msg.payload[0])).* = 0; // FD_MASTER
    var reply: lib.Message = .{};
    _ = lib.vanta_cap_call(pty_cap, @intFromPtr(&msg), @intFromPtr(&reply));
    if (reply.msg_type == 0x42) {
        const n = @min(@as(*align(1) u32, @ptrCast(&reply.payload[0])).*, @as(u32, @intCast(out.len)));
        @memcpy(out[0..n], reply.payload[4..4 + n]);
        return n;
    }
    return 0;
}

// ── Rendering ─────────────────────────────────────────────────────────

fn renderCell(row: u32, col: u32) void {
    const cell = &cells[row][col];
    const bmp = font.getGlyph(cell.ch);
    const px = col * CELL_W;
    const py = row * CELL_H;
    const stride = SURF_W;

    for (0..CELL_H) |r| {
        const bits = bmp[r];
        for (0..CELL_W) |c| {
            const on = (bits >> @intCast(7 - c)) & 1 != 0;
            surf_pixels[(py + r) * stride + (px + c)] = if (on) cell.fg else cell.bg;
        }
    }
    // Draw cursor
    if (row == cursor_row and col == cursor_col and cursor_visible) {
        const cy = py + CELL_H - 2;
        for (0..CELL_W) |c| {
            surf_pixels[cy * stride + (px + c)] = COLOR_CURSOR;
        }
    }
    cell.dirty = false;
}

fn renderAll() void {
    for (0..ROWS) |r| {
        for (0..COLS) |c| {
            renderCell(@intCast(r), @intCast(c));
        }
    }
}

fn renderDirty() void {
    // Always re-render cursor's old and new positions
    for (0..ROWS) |r| {
        for (0..COLS) |c| {
            if (cells[r][c].dirty) {
                renderCell(@intCast(r), @intCast(c));
            }
        }
    }
}

// ── Terminal output primitives ────────────────────────────────────────

fn scrollUp() void {
    for (1..ROWS) |r| {
        cells[r - 1] = cells[r];
        for (0..COLS) |c| cells[r - 1][c].dirty = true;
    }
    for (0..COLS) |c| {
        cells[ROWS - 1][c] = .{};
    }
}

fn putChar(ch: u8) void {
    const old_r = cursor_row;
    const old_c = cursor_col;

    switch (ch) {
        '\n' => {
            cells[cursor_row][cursor_col].dirty = true;
            cursor_col = 0;
            if (cursor_row + 1 >= ROWS) {
                scrollUp();
            } else {
                cursor_row += 1;
            }
        },
        '\r' => {
            cells[cursor_row][cursor_col].dirty = true;
            cursor_col = 0;
        },
        0x08 => { // backspace
            if (cursor_col > 0) {
                cells[cursor_row][cursor_col].dirty = true;
                cursor_col -= 1;
                cells[cursor_row][cursor_col] = .{};
                cells[cursor_row][cursor_col].dirty = true;
            }
        },
        ' '...'~' => {
            cells[cursor_row][cursor_col].ch = ch;
            cells[cursor_row][cursor_col].fg = COLOR_FG;
            cells[cursor_row][cursor_col].bg = COLOR_BG;
            cells[cursor_row][cursor_col].dirty = true;
            cursor_col += 1;
            if (cursor_col >= COLS) {
                cursor_col = 0;
                if (cursor_row + 1 >= ROWS) {
                    scrollUp();
                } else {
                    cursor_row += 1;
                }
            }
        },
        else => {},
    }

    // Mark old cursor position dirty
    if (old_r != cursor_row or old_c != cursor_col) {
        cells[old_r][old_c].dirty = true;
        cells[cursor_row][cursor_col].dirty = true;
    }
}

// ── ANSI/VT100 parser ─────────────────────────────────────────────────

fn processAnsiCmd(cmd: u8) void {
    const p0 = if (csi_param_count > 0) csi_params[0] else 0;
    const p1 = if (csi_param_count > 1) csi_params[1] else 0;
    _ = p1;
    switch (cmd) {
        'A' => { // cursor up
            const n = if (p0 == 0) 1 else p0;
            if (cursor_row >= n) {
                cells[cursor_row][cursor_col].dirty = true;
                cursor_row -= n;
                cells[cursor_row][cursor_col].dirty = true;
            }
        },
        'B' => { // cursor down
            const n = if (p0 == 0) 1 else p0;
            cells[cursor_row][cursor_col].dirty = true;
            cursor_row = @min(cursor_row + n, ROWS - 1);
            cells[cursor_row][cursor_col].dirty = true;
        },
        'C' => { // cursor right
            const n = if (p0 == 0) 1 else p0;
            cells[cursor_row][cursor_col].dirty = true;
            cursor_col = @min(cursor_col + n, COLS - 1);
            cells[cursor_row][cursor_col].dirty = true;
        },
        'D' => { // cursor left
            const n = if (p0 == 0) 1 else p0;
            cells[cursor_row][cursor_col].dirty = true;
            if (cursor_col >= n) cursor_col -= n;
            cells[cursor_row][cursor_col].dirty = true;
        },
        'H', 'f' => { // cursor position
            cells[cursor_row][cursor_col].dirty = true;
            cursor_row = if (p0 > 0) @min(p0 - 1, ROWS - 1) else 0;
            cursor_col = if (csi_param_count > 1 and csi_params[1] > 0) @min(csi_params[1] - 1, COLS - 1) else 0;
            cells[cursor_row][cursor_col].dirty = true;
        },
        'J' => { // erase display
            if (p0 == 2) {
                for (0..ROWS) |r| {
                    for (0..COLS) |c| {
                        cells[r][c] = .{};
                    }
                }
                cursor_row = 0;
                cursor_col = 0;
            }
        },
        'K' => { // erase line
            const start: u32 = if (p0 == 1) 0 else cursor_col;
            const end: u32 = if (p0 == 0) COLS else cursor_col + 1;
            for (start..end) |c| cells[cursor_row][c] = .{};
        },
        'm' => {}, // SGR — ignore colours for now
        else => {},
    }
}

fn feedByte(b: u8) void {
    switch (parse_state) {
        .normal => {
            if (b == 0x1B) {
                parse_state = .esc;
            } else {
                putChar(b);
            }
        },
        .esc => {
            if (b == '[') {
                parse_state = .csi;
                csi_len = 0;
                csi_param_count = 0;
                for (&csi_params) |*p| p.* = 0;
            } else {
                parse_state = .normal;
            }
        },
        .csi => {
            if (b >= '0' and b <= '9') {
                if (csi_param_count == 0) csi_param_count = 1;
                csi_params[csi_param_count - 1] =
                    csi_params[csi_param_count - 1] * 10 + (b - '0');
            } else if (b == ';') {
                if (csi_param_count < csi_params.len) csi_param_count += 1;
            } else {
                // Final byte
                processAnsiCmd(b);
                parse_state = .normal;
            }
        },
    }
}

// ── Key event → ASCII ─────────────────────────────────────────────────

fn handleKeyMsg(msg: *const lib.Message) void {
    const flags = @as(*align(1) const u32, @ptrCast(&msg.payload[0])).*;
    const codepoint = @as(*align(1) const u32, @ptrCast(&msg.payload[8])).*;
    // Only handle keydown with a printable codepoint or control keys
    if (flags & 0x01 == 0) return; // only keydown
    if (codepoint == 0) return;
    const byte: u8 = if (codepoint < 128) @truncate(codepoint) else '?';
    // Echo to display
    feedByte(byte);
    // Send to PTY
    const data: [1]u8 = .{byte};
    ptyWrite(&data);
}

// ── Initial prompt ────────────────────────────────────────────────────

fn printWelcome() void {
    const msg = "VantaOS v0.1\r\nvanta> ";
    for (msg) |c| feedByte(c);
}

// ── Main ──────────────────────────────────────────────────────────────

pub export fn main() void {
    lib.vanta_debug_print("[TERM] terminal emulator starting\n");

    // Create our notification cap for receiving key events from input server
    const notif_res = lib.vanta_notif_create();
    if (notif_res.err == 0) NOTIF_CAP = notif_res.handle;

    // Service discovery — retry until services are up
    compositor_cap = registryLookup("sys.compositor");
    input_cap = registryLookup("sys.input");
    pty_cap = registryLookup("sys.pty");

    if (compositor_cap == 0) {
        lib.vanta_debug_print("[TERM] could not find sys.compositor\n");
        lib.vanta_exit(1);
    }

    // Create surface and get pixel buffer
    createSurface();
    // Centre the terminal on screen
    setPosition(0, 0);
    registerForKeyEvents();

    // Initialize atlas and render initial screen
    atlas.reset();
    printWelcome();
    renderAll();
    swapBuffers();

    lib.vanta_debug_print("[TERM] ready\n");

    var poll_handles: [1]u64 = .{NOTIF_CAP};

    while (true) {
        // Wait for key event notification (input server fires NOTIF_CAP)
        const res = lib.vanta_cap_poll(@intFromPtr(&poll_handles), 1, 100);

        if (res.idx == 0) {
            // Drain key event messages
            var iters: u32 = 0;
            while (iters < 32) : (iters += 1) {
                var msg: lib.Message = .{};
                const err = lib.vanta_cap_recv(NOTIF_CAP, @intFromPtr(&msg));
                if (err != 0) break;
                if (msg.msg_type == 0x51) { // MSG_KEY_EVENT
                    handleKeyMsg(&msg);
                }
            }
        }

        // Poll PTY for output from the shell
        var out_buf: [64]u8 = [_]u8{0} ** 64;
        const n = ptyRead(&out_buf);
        if (n > 0) {
            for (out_buf[0..n]) |b| feedByte(b);
        }

        // Re-render changed cells and push frame
        renderDirty();
        swapBuffers();
    }
}
