// VantaOS — shell server
// Accumulates keystrokes into lines, executes built-in commands.
// Writes output to PTY slave — terminal reads PTY master and displays it.
// All caps injected by kernel — NO registry lookups.

const lib = @import("libvanta");

// ── Cap slots (ALL injected by kernel at spawn) ─────────────────────
// slot 1: own Endpoint port — input server sends MSG_KEY_EVENT here
// slot 2: Endpoint — input server port (send-only for registration)
// slot 3: Endpoint — PTY server port (send+recv for write)
const SHELL_PORT_CAP: lib.Handle = 0x0001000000000001;
const INPUT_CAP: lib.Handle = 0x0001000000000002;
const PTY_CAP: lib.Handle = 0x0001000000000003;

// ── Message codes ─────────────────────────────────────────────────────
const MSG_KEY_EVENT: u32 = 0x51;     // from input server
const MSG_SET_FOCUS_CAP: u32 = 0x50; // register as key listener
const MSG_PTY_WRITE: u32 = 0x41;
const FD_SLAVE: u32 = 1;

// Key event flags (payload[0..4])
const KEY_FLAG_KEYDOWN: u32 = 0x01;

// ── State ─────────────────────────────────────────────────────────────
var line_buf: [256]u8 = [_]u8{0} ** 256;
var line_len: usize = 0;

// ── PTY slave write ───────────────────────────────────────────────────
fn ptyWrite(data: []const u8) void {
    var offset: usize = 0;
    while (offset < data.len) {
        const chunk = @min(data.len - offset, 56);
        var msg: lib.Message = .{};
        msg.msg_type = MSG_PTY_WRITE;
        @as(*align(1) u32, @ptrCast(&msg.payload[0])).* = FD_SLAVE;
        @as(*align(1) u32, @ptrCast(&msg.payload[4])).* = @intCast(chunk);
        @memcpy(msg.payload[8 .. 8 + chunk], data[offset .. offset + chunk]);
        _ = lib.vanta_cap_send_nb(PTY_CAP, @intFromPtr(&msg));
        offset += chunk;
    }
}

// ── String helpers ────────────────────────────────────────────────────
fn strEql(a: []const u8, b: []const u8) bool {
    if (a.len != b.len) return false;
    for (a, b) |x, y| if (x != y) return false;
    return true;
}

fn startsWith(s: []const u8, prefix: []const u8) bool {
    if (s.len < prefix.len) return false;
    return strEql(s[0..prefix.len], prefix);
}

// ── Built-in commands ─────────────────────────────────────────────────
fn executeCommand(cmd: []const u8) void {
    if (cmd.len == 0) {
        // empty line — just reprint prompt
    } else if (strEql(cmd, "help")) {
        ptyWrite("Commands:\r\n");
        ptyWrite("  help         print this message\r\n");
        ptyWrite("  echo <text>  print text\r\n");
        ptyWrite("  clear        clear screen\r\n");
        ptyWrite("  uname        OS information\r\n");
        ptyWrite("  ls           list files\r\n");
    } else if (startsWith(cmd, "echo ")) {
        ptyWrite(cmd[5..]);
        ptyWrite("\r\n");
    } else if (strEql(cmd, "echo")) {
        ptyWrite("\r\n");
    } else if (strEql(cmd, "clear")) {
        ptyWrite("\x1B[2J\x1B[H");
    } else if (strEql(cmd, "uname")) {
        ptyWrite("VantaOS 0.1 x86_64\r\n");
    } else if (strEql(cmd, "ls")) {
        ptyWrite("No filesystem mounted.\r\n");
    } else {
        ptyWrite(cmd);
        ptyWrite(": command not found\r\n");
    }
    ptyWrite("vanta$ ");
}

// ── Key event handler ─────────────────────────────────────────────────
fn handleKeyEvent(msg: *const lib.Message) void {
    const flags = @as(*align(1) const u32, @ptrCast(&msg.payload[0])).*;
    const codepoint = @as(*align(1) const u32, @ptrCast(&msg.payload[8])).*;
    if ((flags & KEY_FLAG_KEYDOWN) == 0) return; // only keydown
    if (codepoint == 0) return;
    const ch: u8 = if (codepoint < 128) @truncate(codepoint) else 0;
    if (ch == 0) return;

    if (ch == '\r' or ch == '\n') {
        ptyWrite("\r\n");
        executeCommand(line_buf[0..line_len]);
        line_len = 0;
    } else if ((ch == 0x08 or ch == 0x7F) and line_len > 0) {
        line_len -= 1;
        ptyWrite("\x08 \x08"); // erase character on screen
    } else if (ch >= 0x20 and line_len < line_buf.len - 1) {
        line_buf[line_len] = ch;
        line_len += 1;
        ptyWrite(&[_]u8{ch}); // echo character to terminal
    }
}

// ── Main ──────────────────────────────────────────────────────────────
pub export fn main() void {
    lib.vanta_debug_print("[SHELL] starting\n");

    // Register with input server as a key-event listener
    {
        var msg: lib.Message = .{};
        msg.msg_type = MSG_SET_FOCUS_CAP;
        msg.payload[0] = 1; // key events
        var send_cap: lib.Handle = 0;
        _ = lib.vanta_cap_derive(SHELL_PORT_CAP, 1, @intFromPtr(&send_cap));
        msg.caps[0] = send_cap;
        _ = lib.vanta_cap_send(INPUT_CAP, @intFromPtr(&msg));
    }

    lib.vanta_debug_print("[SHELL] ready\n");

    // Print banner via PTY
    ptyWrite("VantaOS shell. Type 'help' for commands.\r\nvanta$ ");

    var poll_handles: [1]u64 = .{SHELL_PORT_CAP};

    while (true) {
        const res = lib.vanta_cap_poll(@intFromPtr(&poll_handles), 1, -1);
        if (res.idx != 0) continue;

        var iters: u32 = 0;
        while (iters < 32) : (iters += 1) {
            var msg: lib.Message = .{};
            const err = lib.vanta_cap_recv(SHELL_PORT_CAP, @intFromPtr(&msg));
            if (err != 0) break;
            if (msg.msg_type == MSG_KEY_EVENT) handleKeyEvent(&msg);
        }
    }
}
