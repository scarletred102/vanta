// ============================================================================
// VantaOS — Interrupt Descriptor Table (IDT)
// Phase 0: Minimal exception handlers that print diagnostic info and halt.
// Phase 1 will add proper ISR stubs with register saving and recovery.
// ============================================================================

const serial = @import("serial.zig");

// ── IDT Entry (16 bytes) ────────────────────────────────────────

const IdtEntry = extern struct {
    offset_low: u16 = 0,
    selector: u16 = 0,
    ist: u8 = 0, // bits 0-2: IST index, bits 3-7: reserved (0)
    type_attr: u8 = 0, // type (4 bits) + DPL (2 bits) + present (1 bit)
    offset_mid: u16 = 0,
    offset_high: u32 = 0,
    reserved: u32 = 0,
};

comptime {
    if (@sizeOf(IdtEntry) != 16) @compileError("IDT entry must be 16 bytes");
}

fn makeGate(handler_addr: u64, selector: u16, ist: u3, gate_type: u4, dpl: u2) IdtEntry {
    return .{
        .offset_low = @truncate(handler_addr & 0xFFFF),
        .selector = selector,
        .ist = ist,
        .type_attr = (@as(u8, 1) << 7) | // Present
            (@as(u8, dpl) << 5) |
            @as(u8, gate_type),
        .offset_mid = @truncate((handler_addr >> 16) & 0xFFFF),
        .offset_high = @truncate((handler_addr >> 32) & 0xFFFFFFFF),
        .reserved = 0,
    };
}

// ── IDT Table ───────────────────────────────────────────────────

var idt: [256]IdtEntry = [_]IdtEntry{.{}} ** 256;

// ── Exception Names ─────────────────────────────────────────────

const EXCEPTION_NAMES = [32][]const u8{
    "#DE Divide Error",
    "#DB Debug",
    "NMI Non-Maskable Interrupt",
    "#BP Breakpoint",
    "#OF Overflow",
    "#BR Bound Range Exceeded",
    "#UD Invalid Opcode",
    "#NM Device Not Available",
    "#DF Double Fault",
    "Coprocessor Segment Overrun",
    "#TS Invalid TSS",
    "#NP Segment Not Present",
    "#SS Stack-Segment Fault",
    "#GP General Protection Fault",
    "#PF Page Fault",
    "Reserved (15)",
    "#MF x87 FPU Error",
    "#AC Alignment Check",
    "#MC Machine Check",
    "#XM SIMD FP Exception",
    "#VE Virtualization Exception",
    "#CP Control Protection",
    "Reserved (22)",
    "Reserved (23)",
    "Reserved (24)",
    "Reserved (25)",
    "Reserved (26)",
    "Reserved (27)",
    "#HV Hypervisor Injection",
    "#VC VMM Communication",
    "#SX Security Exception",
    "Reserved (31)",
};

// Exceptions that push an error code onto the stack
fn hasErrorCode(vector: u8) bool {
    return switch (vector) {
        8, 10, 11, 12, 13, 14, 17, 21, 29, 30 => true,
        else => false,
    };
}

// ── ISR Stubs (comptime-generated) ──────────────────────────────
// Each exception vector gets a minimal stub that:
//   1. Disables interrupts
//   2. Prints exception info via serial
//   3. Halts the CPU
//
// Phase 1 TODO: Save all registers, support recovery for non-fatal faults.

fn makeIsrStub(comptime vector: u8) *const fn () callconv(.naked) void {
    return &struct {
        fn stub() callconv(.naked) void {
            // Single asm block: disable interrupts, set vector arg, jump to handler
            asm volatile (
                \\cli
                \\mov %[vec], %%edi
                \\jmp *%[handler]
                :
                : [vec] "i" (@as(u32, vector)),
                  [handler] "r" (&handleException),
            );
        }
    }.stub;
}

fn handleException(vector: u32) callconv(.c) noreturn {
    serial.puts("\n!!! EXCEPTION: ");
    if (vector < 32) {
        serial.puts(EXCEPTION_NAMES[vector]);
    } else {
        serial.puts("Unknown (");
        serial.putDec(vector);
        serial.puts(")");
    }
    serial.puts(" !!!\n");

    // TODO Phase 1: Print registers, stack trace, faulting address (CR2 for PF)

    serial.puts("System halted.\n");
    while (true) {
        asm volatile ("hlt");
    }
}

// ── IDT Initialization ──────────────────────────────────────────

pub fn init() void {
    // Install exception handlers for vectors 0-31
    inline for (0..32) |i| {
        const handler_addr = @intFromPtr(makeIsrStub(i));
        idt[i] = makeGate(
            handler_addr,
            0x08, // Kernel code segment
            0, // No IST
            0xE, // 64-bit interrupt gate
            0, // Ring 0
        );
    }

    // Build and load IDTR (10 bytes: limit[2] + base[8])
    var idtr: [10]u8 align(4) = undefined;
    const limit: u16 = @sizeOf(@TypeOf(idt)) - 1;
    const base: u64 = @intFromPtr(&idt);

    idtr[0] = @truncate(limit);
    idtr[1] = @truncate(limit >> 8);
    inline for (0..8) |j| {
        idtr[2 + j] = @truncate(base >> (j * 8));
    }

    asm volatile ("lidt (%[idtr])"
        :
        : [idtr] "r" (&idtr),
        : .{ .memory = true }
    );

    serial.puts("[IDT]   Loaded (32 exception vectors)\n");
}
