// ============================================================================
// VantaOS — COM1 Serial Port Driver (Debug Output)
// Standard 8250/16550 UART at I/O port 0x3F8
// ============================================================================

const COM1: u16 = 0x3F8;

// ── Initialization ──────────────────────────────────────────────

pub fn init() void {
    outb(COM1 + 1, 0x00); // Disable all interrupts
    outb(COM1 + 3, 0x80); // Enable DLAB (set baud rate divisor)
    outb(COM1 + 0, 0x03); // Divisor low byte: 38400 baud
    outb(COM1 + 1, 0x00); // Divisor high byte
    outb(COM1 + 3, 0x03); // 8 bits, no parity, one stop bit
    outb(COM1 + 2, 0xC7); // Enable FIFO, clear, 14-byte threshold
    outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR set

    // Optional loopback test — only as a diagnostic; we always end in normal
    // mode below. VirtualBox's UART can fail this test even when COM1 works
    // fine, so we MUST NOT leave the UART in loopback on failure or output
    // would be silently dropped.
    outb(COM1 + 4, 0x1E);
    outb(COM1 + 0, 0xAE);
    _ = inb(COM1 + 0);

    // Normal operation mode — always.
    outb(COM1 + 4, 0x0F);
}

// ── Output Functions ────────────────────────────────────────────

pub fn putc(c: u8) void {
    // Wait for transmit holding register to be empty
    while ((inb(COM1 + 5) & 0x20) == 0) {
        asm volatile ("pause");
    }
    outb(COM1, c);
}

pub fn puts(s: []const u8) void {
    for (s) |c| {
        if (c == '\n') putc('\r');
        putc(c);
    }
}

/// Print a u64 as hexadecimal (no leading zeros except for value 0)
pub fn putHex(value: u64) void {
    const hex = "0123456789abcdef";
    var started = false;
    var shift: i8 = 60;
    while (shift >= 0) : (shift -= 4) {
        const s: u6 = @intCast(shift);
        const nibble: u4 = @truncate(value >> s);
        if (nibble != 0 or started or shift == 0) {
            putc(hex[nibble]);
            started = true;
        }
    }
}

/// Print a u64 as decimal
pub fn putDec(value: u64) void {
    if (value == 0) {
        putc('0');
        return;
    }
    var buf: [20]u8 = undefined; // u64 max is 20 digits
    var len: usize = 0;
    var v = value;
    while (v > 0) {
        buf[len] = @intCast(v % 10 + '0');
        len += 1;
        v /= 10;
    }
    // Print digits in reverse order
    var i = len;
    while (i > 0) {
        i -= 1;
        putc(buf[i]);
    }
}

// ── x86 Port I/O ───────────────────────────────────────────────

fn outb(port: u16, val: u8) void {
    asm volatile ("outb %[val], %[port]"
        :
        : [val] "{al}" (val),
          [port] "{dx}" (port),
    );
}

fn inb(port: u16) u8 {
    return asm volatile ("inb %[port], %[ret]"
        : [ret] "={al}" (-> u8),
        : [port] "{dx}" (port),
    );
}
