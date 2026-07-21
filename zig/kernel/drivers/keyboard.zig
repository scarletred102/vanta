// ============================================================================
// VantaOS — PS/2 Keyboard Driver
// Phase 1: Basic scancode reader and printer.
// ============================================================================

const cpu = @import("../arch/x86_64/cpu.zig");
const serial = @import("../arch/x86_64/serial.zig");

// Keyboard I/O Ports
const KBD_DATA_PORT: u16 = 0x60;
const KBD_STATUS_PORT: u16 = 0x64;

// US QWERTY Scancode Set 1 Translation Table
const kbd_map = [_]u8{
    0,   27,  '1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '-', '=', '\x08',
    '\t', 'q', 'w', 'e', 'r', 't', 'y', 'u', 'i', 'o', 'p', '[', ']', '\n',
    0,   'a', 's', 'd', 'f', 'g', 'h', 'j', 'k', 'l', ';', '\'', '`',
    0,   '\\', 'z', 'x', 'c', 'v', 'b', 'n', 'm', ',', '.', '/', 0,
    '*', 0,   ' ', 0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    0,   0,   0,   0,   '-', 0,   0,   0,   '+', 0,   0,   0,   0,   0,   0,   0,
    0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
    0,   0,   0,   0,   0,   0,   0,   0,   0,
};

var shift_active: bool = false;

pub fn handleInterrupt() void {
    const scancode = cpu.inb(KBD_DATA_PORT);

    // If key release
    if (scancode & 0x80 != 0) {
        const released_scancode = scancode & 0x7F;
        if (released_scancode == 0x2A or released_scancode == 0x36) {
            shift_active = false;
        }
        return;
    }

    // Key press
    if (scancode == 0x2A or scancode == 0x36) {
        shift_active = true;
        return;
    }

    if (scancode < 128) {
        var char = kbd_map[scancode];
        if (char != 0) {
            // Apply simple shift logic
            if (shift_active) {
                if (char >= 'a' and char <= 'z') {
                    char = char - 'a' + 'A';
                } else {
                    char = switch (char) {
                        '1' => '!',
                        '2' => '@',
                        '3' => '#',
                        '4' => '$',
                        '5' => '%',
                        '6' => '^',
                        '7' => '&',
                        '8' => '*',
                        '9' => '(',
                        '0' => ')',
                        '-' => '_',
                        '=' => '+',
                        '[' => '{',
                        ']' => '}',
                        ';' => ':',
                        '\'' => '"',
                        '`' => '~',
                        ',' => '<',
                        '.' => '>',
                        '/' => '?',
                        '\\' => '|',
                        else => char,
                    };
                }
            }

            serial.puts("[KBD]   Key press: '");
            const buf = [1]u8{char};
            serial.puts(&buf);
            serial.puts("'\n");
        } else {
            serial.puts("[KBD]   Scancode: 0x");
            serial.putHex(scancode);
            serial.puts("\n");
        }
    }
}
