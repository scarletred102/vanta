// VantaOS — input server (sys.input)
// Handles PS/2 keyboard (IRQ 1) and PS/2 mouse (IRQ 12).
// Translates scancode set 1 → Unicode codepoints.
// Delivers KeyEvent / MouseEvent notifications to registered listeners.

const lib = @import("libvanta");

// ── Cap slot constants ────────────────────────────────────────────────
// slot 1: DeviceIRQ(1)  — PS/2 keyboard
// slot 2: DeviceIRQ(12) — PS/2 mouse
// slot 3: endpoint — registry
// slot 4: endpoint — our own server port
const KBD_IRQ_CAP: lib.Handle = 0x0001000000000001;
const MOUSE_IRQ_CAP: lib.Handle = 0x0001000000000002;
const REGISTRY_CAP: lib.Handle = 0x0001000000000003;
const PORT_CAP: lib.Handle = 0x0001000000000004;

// Created dynamically in main() via vanta_notif_create()
var KBD_NOTIF_CAP: lib.Handle = 0;
var MOUSE_NOTIF_CAP: lib.Handle = 0;

// ── IPC message codes ─────────────────────────────────────────────────
// Clients can register a notification cap with SET_FOCUS_CAP (0x50).
// MSG_KEY_EVENT (0x51) and MSG_MOUSE_EVENT (0x52) are sent to listeners.
const MSG_SET_FOCUS_CAP: u32 = 0x50;
const MSG_KEY_EVENT: u32 = 0x51;
const MSG_MOUSE_EVENT: u32 = 0x52;

// ── Key event flags ───────────────────────────────────────────────────
const KEY_FLAG_KEYDOWN: u32 = 0x01;
const KEY_FLAG_KEYUP: u32 = 0x02;
const KEY_FLAG_SHIFT: u32 = 0x04;
const KEY_FLAG_CTRL: u32 = 0x08;
const KEY_FLAG_ALT: u32 = 0x10;

// ── Key event payload ─────────────────────────────────────────────────
// [0..4]  : flags (KEY_FLAG_*)
// [4..8]  : scancode (raw PS/2 scancode set 1)
// [8..12] : unicode codepoint (0 = no printable char)

// ── Mouse event payload ───────────────────────────────────────────────
// [0..4]  : buttons bitmask (bit0=left, bit1=right, bit2=middle)
// [4..8]  : dx (i32, signed relative movement)
// [8..12] : dy (i32, signed, Y is positive = down)

// ── Scancode set 1 → ASCII table (unshifted) ─────────────────────────
// Index = scancode byte (0x00..0x58); value = ASCII char (0 = no char)
const SCANCODE_MAP: [89]u8 = .{
    0,   0,   '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '-',  '=',  0x08, // 0x00-0x0E (backspace)
    '\t','q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p', '[', ']',  '\n', // 0x0F-0x1C
    0,   'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';', '\'','`',  // 0x1D-0x29
    0,   '\\','z', 'x', 'c', 'v', 'b', 'n', 'm', ',', '.', '/', 0,    // 0x2A-0x36
    '*', 0,   ' ', 0,   0,   0,   0,   0,   0,   0,   0,   0,   0,    // 0x37-0x43
    0,   0,   0,   0,   0,   0,   0,   '7', '8', '9', '-', '4', '5',  // 0x44-0x50
    '6', '+', '1', '2', '3', '0', '.', 0, // 0x51-0x58 (numpad + F12)
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

// ── State ─────────────────────────────────────────────────────────────
var shift_down: bool = false;
var ctrl_down: bool = false;
var alt_down: bool = false;
var e0_prefix: bool = false;

// Mouse packet assembly (PS/2 standard 3-byte packets)
var mouse_buf: [3]u8 = .{ 0, 0, 0 };
var mouse_idx: u8 = 0;

// Registered focus notification handles (up to 8 listeners)
const MAX_LISTENERS: usize = 8;
var key_listeners: [MAX_LISTENERS]lib.Handle = [_]lib.Handle{0} ** MAX_LISTENERS;
var mouse_listeners: [MAX_LISTENERS]lib.Handle = [_]lib.Handle{0} ** MAX_LISTENERS;
var n_key_listeners: usize = 0;
var n_mouse_listeners: usize = 0;

// ── Helpers ───────────────────────────────────────────────────────────

fn broadcastKey(flags: u32, scancode: u8, codepoint: u32) void {
    var msg: lib.Message = .{};
    msg.msg_type = MSG_KEY_EVENT;
    @as(*align(1) u32, @ptrCast(&msg.payload[0])).* = flags;
    @as(*align(1) u32, @ptrCast(&msg.payload[4])).* = scancode;
    @as(*align(1) u32, @ptrCast(&msg.payload[8])).* = codepoint;
    for (key_listeners[0..n_key_listeners]) |cap| {
        if (cap != 0) _ = lib.vanta_cap_send_nb(cap, @intFromPtr(&msg));
    }
}

fn broadcastMouse(buttons: u32, dx: i32, dy: i32) void {
    var msg: lib.Message = .{};
    msg.msg_type = MSG_MOUSE_EVENT;
    @as(*align(1) u32, @ptrCast(&msg.payload[0])).* = buttons;
    @as(*align(1) i32, @ptrCast(&msg.payload[4])).* = dx;
    @as(*align(1) i32, @ptrCast(&msg.payload[8])).* = dy;
    for (mouse_listeners[0..n_mouse_listeners]) |cap| {
        if (cap != 0) _ = lib.vanta_cap_send_nb(cap, @intFromPtr(&msg));
    }
}

fn processKeyByte(byte: u8) void {
    if (byte == 0xE0) {
        e0_prefix = true;
        return;
    }

    const is_release = (byte & 0x80) != 0;
    const code: u8 = byte & 0x7F;
    const ext = e0_prefix;
    e0_prefix = false;

    // Track modifier state
    if (!ext) {
        switch (code) {
            0x2A, 0x36 => shift_down = !is_release, // L/R Shift
            0x1D => ctrl_down = !is_release,          // Ctrl
            0x38 => alt_down = !is_release,            // Alt
            else => {},
        }
    }

    var flags: u32 = if (is_release) KEY_FLAG_KEYUP else KEY_FLAG_KEYDOWN;
    if (shift_down) flags |= KEY_FLAG_SHIFT;
    if (ctrl_down) flags |= KEY_FLAG_CTRL;
    if (alt_down) flags |= KEY_FLAG_ALT;

    var codepoint: u32 = 0;
    if (!is_release and !ext and code < SCANCODE_MAP.len) {
        const ch = if (shift_down) SCANCODE_MAP_SHIFT[code] else SCANCODE_MAP[code];
        codepoint = ch;
    }

    broadcastKey(flags, byte, codepoint);
}

fn processMouseByte(byte: u8) void {
    mouse_buf[mouse_idx] = byte;
    mouse_idx += 1;
    if (mouse_idx < 3) return;
    mouse_idx = 0;

    const flags = mouse_buf[0];
    // Check overflow bits — discard packet if set
    if ((flags & 0xC0) != 0) return;

    // Sign-extend X and Y from 9-bit two's complement
    var dx: i32 = mouse_buf[1];
    if ((flags & 0x10) != 0) dx |= @as(i32, -256);
    var dy: i32 = mouse_buf[2];
    if ((flags & 0x20) != 0) dy |= @as(i32, -256);
    // PS/2 Y is inverted relative to screen coordinates
    dy = -dy;

    const buttons: u32 = flags & 0x07;
    broadcastMouse(buttons, dx, dy);
}

// Drain all bytes from an IRQ ring buffer and process them
fn drainKbd() void {
    while (true) {
        const res = lib.vanta_irq_readbyte(KBD_IRQ_CAP);
        if (res.err != 0) break;
        processKeyByte(res.byte);
    }
}

fn drainMouse() void {
    while (true) {
        const res = lib.vanta_irq_readbyte(MOUSE_IRQ_CAP);
        if (res.err != 0) break;
        processMouseByte(res.byte);
    }
}

fn registerService() void {
    var msg: lib.Message = .{};
    msg.msg_type = 0x10; // NS_REGISTER
    const name = "sys.input";
    for (name, 0..) |c, i| msg.payload[i] = c;
    // Derive a send cap so we keep PORT_CAP for our own service loop
    var send_cap: lib.Handle = 0;
    _ = lib.vanta_cap_derive(PORT_CAP, 7, @intFromPtr(&send_cap));
    msg.caps[0] = send_cap;
    _ = lib.vanta_cap_send(REGISTRY_CAP, @intFromPtr(&msg));
}

pub export fn main() void {
    lib.vanta_debug_print("[INPUT] input server starting\n");

    // Create notification caps for IRQ delivery
    const kbd_notif = lib.vanta_notif_create();
    if (kbd_notif.err == 0) KBD_NOTIF_CAP = kbd_notif.handle;

    const mouse_notif = lib.vanta_notif_create();
    if (mouse_notif.err == 0) MOUSE_NOTIF_CAP = mouse_notif.handle;

    // Bind IRQ notification caps
    _ = lib.vanta_irq_bind(KBD_IRQ_CAP, KBD_NOTIF_CAP);
    _ = lib.vanta_irq_bind(MOUSE_IRQ_CAP, MOUSE_NOTIF_CAP);

    registerService();

    lib.vanta_debug_print("[INPUT] ready\n");

    // Poll: kbd notif (idx 0), mouse notif (idx 1), port (idx 2)
    var handles: [3]u64 = .{ KBD_NOTIF_CAP, MOUSE_NOTIF_CAP, PORT_CAP };

    while (true) {
        const res = lib.vanta_cap_poll(@intFromPtr(&handles), 3, -1);

        switch (res.idx) {
            0 => drainKbd(),
            1 => drainMouse(),
            2 => {
                var msg: lib.Message = .{};
                const err = lib.vanta_cap_recv(PORT_CAP, @intFromPtr(&msg));
                if (err != 0) continue;
                if (msg.msg_type == MSG_SET_FOCUS_CAP) {
                    // payload[0] = 1 for key events, 2 for mouse events
                    const kind = msg.payload[0];
                    const cap = msg.caps[0];
                    if (kind == 1 and n_key_listeners < MAX_LISTENERS) {
                        key_listeners[n_key_listeners] = cap;
                        n_key_listeners += 1;
                    } else if (kind == 2 and n_mouse_listeners < MAX_LISTENERS) {
                        mouse_listeners[n_mouse_listeners] = cap;
                        n_mouse_listeners += 1;
                    }
                }
            },
            else => {},
        }
    }
}
