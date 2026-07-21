// VantaOS — terminal console (self-contained)
// 80×25 VT100/ANSI terminal rendered directly to the Limine framebuffer.
// Owns the keyboard IRQ directly, translates scancodes, runs the shell
// builtins inline, and renders to the framebuffer. No input/pty/shell IPC.

const lib = @import("libvanta");
const font = @import("bitmap_font.zig");

// ── Cap slots (injected by kernel at spawn) ─────────────────────────
// slot 1: terminal's own port (unused, kept for symmetry)
// slot 2: MemoryCap — Limine linear framebuffer (direct pixel access)
// slot 3: DeviceIRQ — IRQ 1 (PS/2 keyboard)
const TERM_PORT_CAP: lib.Handle = 0x0001000000000001;
const FB_MEM_CAP: lib.Handle = 0x0001000000000002;
const KBD_IRQ_CAP: lib.Handle = 0x0001000000000003;

// ── Framebuffer — hardcoded to match limine.conf resolution ─────────
const FB_WIDTH: u32 = 1024;
const FB_HEIGHT: u32 = 768;
const FB_STRIDE_PX: u32 = FB_WIDTH; // pixels per row
const FB_VADDR: u64 = 0x50000000;

// ── Terminal dimensions ─────────────────────────────────────────────
const COLS: u32 = 80;
const ROWS: u32 = 25;
const CELL_W: u32 = font.GLYPH_W;
const CELL_H: u32 = font.GLYPH_H;

// ── Colours (BGRA8) ─────────────────────────────────────────────────
const COLOR_BG: u32 = 0xFF1A1A2E; // dark navy
const COLOR_FG: u32 = 0xFFE0E0E0; // light grey
const COLOR_CURSOR: u32 = 0xFF00FF88; // green cursor

// ── Cell buffer ─────────────────────────────────────────────────────
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

// ── ANSI parser state ───────────────────────────────────────────────
const ParseState = enum { normal, esc, csi };
var parse_state: ParseState = .normal;
var csi_params: [8]u32 = [_]u32{0} ** 8;
var csi_param_count: u32 = 0;

// ── Framebuffer pixel pointer ───────────────────────────────────────
var fb_pixels: [*]volatile u32 = undefined;

// ── Keyboard state ──────────────────────────────────────────────────
var shift_down: bool = false;
var line_buf: [256]u8 = [_]u8{0} ** 256;
var line_len: usize = 0;

// Scancode set 1 → ASCII (unshifted). Index = scancode (0x00..0x58).
const SCANCODE_MAP: [89]u8 = .{
    0,   0,   '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '-',  '=',  0x08,
    '\t','q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p', '[', ']',  '\n',
    0,   'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';', '\'','`',
    0,   '\\','z', 'x', 'c', 'v', 'b', 'n', 'm', ',', '.', '/', 0,
    '*', 0,   ' ', 0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    0,   0,   0,   0,   0,   0,   0,   '7', '8', '9', '-', '4', '5',
    '6', '+', '1', '2', '3', '0', '.', 0,
};

const SCANCODE_MAP_SHIFT: [89]u8 = .{
    0,   0,   '!', '@', '#', '$', '%', '^', '&', '*', '(', ')', '_',  '+',  0x08,
    '\t','Q', 'W', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P', '{', '}', '\n',
    0,   'A', 'S', 'D', 'F', 'G', 'H', 'J', 'K', 'L', ':', '"', '~',
    0,   '|', 'Z', 'X', 'C', 'V', 'B', 'N', 'M', '<', '>', '?', 0,
    '*', 0,   ' ', 0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    0,   0,   0,   0,   0,   0,   0,   '7', '8', '9', '-', '4', '5',
    '6', '+', '1', '2', '3', '0', '.', 0,
};

// ── Rendering (direct to Limine FB) ─────────────────────────────────

fn renderCell(row: u32, col: u32) void {
    const cell = &cells[row][col];
    const bmp = font.getGlyph(cell.ch);
    const px = col * CELL_W;
    const py = row * CELL_H;

    for (0..CELL_H) |r| {
        const bits = bmp[r];
        for (0..CELL_W) |c| {
            const on = (bits >> @intCast(7 - c)) & 1 != 0;
            fb_pixels[(py + r) * FB_STRIDE_PX + (px + c)] = if (on) cell.fg else cell.bg;
        }
    }
    if (row == cursor_row and col == cursor_col and cursor_visible) {
        const cy = py + CELL_H - 2;
        for (0..CELL_W) |c| {
            fb_pixels[cy * FB_STRIDE_PX + (px + c)] = COLOR_CURSOR;
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
    for (0..ROWS) |r| {
        for (0..COLS) |c| {
            if (cells[r][c].dirty) {
                renderCell(@intCast(r), @intCast(c));
            }
        }
    }
}

fn clearScreen() void {
    const total = FB_STRIDE_PX * FB_HEIGHT;
    for (0..total) |i| fb_pixels[i] = COLOR_BG;
}

// ── Terminal output primitives ──────────────────────────────────────

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
        0x08 => {
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

    if (old_r != cursor_row or old_c != cursor_col) {
        cells[old_r][old_c].dirty = true;
        cells[cursor_row][cursor_col].dirty = true;
    }
}

// ── ANSI/VT100 parser ───────────────────────────────────────────────

fn processAnsiCmd(cmd: u8) void {
    const p0 = if (csi_param_count > 0) csi_params[0] else 0;
    switch (cmd) {
        'H', 'f' => {
            cells[cursor_row][cursor_col].dirty = true;
            cursor_row = if (p0 > 0) @min(p0 - 1, ROWS - 1) else 0;
            cursor_col = if (csi_param_count > 1 and csi_params[1] > 0) @min(csi_params[1] - 1, COLS - 1) else 0;
            cells[cursor_row][cursor_col].dirty = true;
        },
        'J' => {
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
        'K' => {
            const start: u32 = if (p0 == 1) 0 else cursor_col;
            const end: u32 = if (p0 == 0) COLS else cursor_col + 1;
            for (start..end) |c| cells[cursor_row][c] = .{};
        },
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
                processAnsiCmd(b);
                parse_state = .normal;
            }
        },
    }
}

fn print(s: []const u8) void {
    for (s) |b| feedByte(b);
}

// ── Shell builtins (run inline) ─────────────────────────────────────

fn strEql(a: []const u8, b: []const u8) bool {
    if (a.len != b.len) return false;
    for (a, b) |x, y| if (x != y) return false;
    return true;
}

fn startsWith(s: []const u8, prefix: []const u8) bool {
    if (s.len < prefix.len) return false;
    return strEql(s[0..prefix.len], prefix);
}

fn executeCommand(cmd: []const u8) void {
    if (cmd.len == 0) {
        // empty line — just reprint prompt
    } else if (strEql(cmd, "help")) {
        print("Commands:\r\n");
        print("  help         print this message\r\n");
        print("  echo <text>  print text\r\n");
        print("  clear        clear screen\r\n");
        print("  uname        OS information\r\n");
        print("  ls           list files\r\n");
    } else if (startsWith(cmd, "echo ")) {
        print(cmd[5..]);
        print("\r\n");
    } else if (strEql(cmd, "echo")) {
        print("\r\n");
    } else if (strEql(cmd, "clear")) {
        print("\x1B[2J\x1B[H");
    } else if (strEql(cmd, "uname")) {
        print("VantaOS 0.1 x86_64\r\n");
    } else if (strEql(cmd, "ls")) {
        print("No filesystem mounted.\r\n");
    } else {
        print(cmd);
        print(": command not found\r\n");
    }
}

fn printPrompt() void {
    print("vanta$ ");
}

// ── Keyboard input ──────────────────────────────────────────────────

fn handleChar(ch: u8) void {
    if (ch == '\n' or ch == '\r') {
        print("\r\n");
        executeCommand(line_buf[0..line_len]);
        line_len = 0;
        printPrompt();
    } else if (ch == 0x08) {
        if (line_len > 0) {
            line_len -= 1;
            feedByte(0x08);
        }
    } else if (ch >= 0x20 and ch < 0x7F) {
        if (line_len < line_buf.len) {
            line_buf[line_len] = ch;
            line_len += 1;
            feedByte(ch);
        }
    }
}

fn processScancode(sc: u8) void {
    const release = (sc & 0x80) != 0;
    const code: u8 = sc & 0x7F;

    // Shift modifiers (0x2A = left, 0x36 = right).
    if (code == 0x2A or code == 0x36) {
        shift_down = !release;
        return;
    }
    if (release) return;
    if (code >= SCANCODE_MAP.len) return;

    const ch = if (shift_down) SCANCODE_MAP_SHIFT[code] else SCANCODE_MAP[code];
    if (ch == 0) return;
    handleChar(ch);
}

// ── Main ────────────────────────────────────────────────────────────

pub export fn main() void {
    lib.vanta_debug_print("[TERM] console starting\n");

    // 1. Map the Limine framebuffer directly.
    const map_err = lib.vanta_mem_map(FB_MEM_CAP, FB_VADDR, 2048);
    if (map_err != 0) {
        lib.vanta_debug_print("[TERM] FB mem_map FAILED\n");
        lib.vanta_exit(1);
    }
    fb_pixels = @as([*]volatile u32, @ptrFromInt(FB_VADDR));
    clearScreen();

    // 2. Bind the keyboard IRQ to a notification we can wait on.
    const notif = lib.vanta_notif_create();
    if (notif.err != 0) {
        lib.vanta_debug_print("[TERM] notif_create FAILED\n");
        lib.vanta_exit(1);
    }
    const kbd_notif = notif.handle;
    _ = lib.vanta_irq_bind(KBD_IRQ_CAP, kbd_notif);

    // 3. Welcome banner + prompt.
    print("VantaOS v0.1\r\n");
    print("Type 'help' for commands.\r\n");
    printPrompt();
    renderAll();
    lib.vanta_debug_print("[TERM] console ready\n");

    // 4. Main loop: block until a keyboard IRQ, drain scancodes, render.
    while (true) {
        _ = lib.vanta_cap_wait(kbd_notif, 0xFFFFFFFFFFFFFFFF);
        while (true) {
            const res = lib.vanta_irq_readbyte(KBD_IRQ_CAP);
            if (res.err != 0) break;
            processScancode(res.byte);
        }
        renderDirty();
    }
}
