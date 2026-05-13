// ============================================================================
// VantaOS — Interrupt Descriptor Table (IDT)
// Phase 0: Minimal exception handlers via module-level asm stubs.
//
// Each stub is generated at module-level inline asm with fixed 16-byte
// alignment. The IDT is built by computing addresses from a base label.
// ============================================================================

const serial = @import("serial.zig");

// ── IDT Entry (16 bytes) ────────────────────────────────────────

const IdtEntry = extern struct {
    offset_low: u16 = 0,
    selector: u16 = 0,
    ist: u8 = 0,
    type_attr: u8 = 0,
    offset_mid: u16 = 0,
    offset_high: u32 = 0,
    reserved: u32 = 0,
};

comptime {
    if (@sizeOf(IdtEntry) != 16) @compileError("IDT entry must be 16 bytes");
}

fn makeGate(handler_addr: u64, selector: u16, ist: u3) IdtEntry {
    return .{
        .offset_low = @truncate(handler_addr & 0xFFFF),
        .selector = selector,
        .ist = ist,
        .type_attr = 0x8E, // Present, DPL=0, 64-bit interrupt gate
        .offset_mid = @truncate((handler_addr >> 16) & 0xFFFF),
        .offset_high = @truncate((handler_addr >> 32) & 0xFFFFFFFF),
        .reserved = 0,
    };
}

// IST assignment per vector (matches tss.zig)
fn istForVector(vec: usize) u3 {
    return switch (vec) {
        2 => 2,   // NMI       → IST2
        8 => 1,   // #DF       → IST1
        18 => 3,  // #MC       → IST3
        else => 0,
    };
}

fn currentCs() u16 {
    var cs: u16 = 0;
    asm volatile ("mov %%cs, %[out]" : [out] "=r" (cs));
    return cs;
}

// ── IDT Table ───────────────────────────────────────────────────

var idt: [256]IdtEntry = [_]IdtEntry{.{}} ** 256;

const EXCEPTION_NAMES = [32][]const u8{
    "Divide Error",  "Debug",         "NMI",                 "Breakpoint",
    "Overflow",      "Bound Range",   "Invalid Opcode",      "Device NA",
    "Double Fault",  "Coproc Seg",    "Invalid TSS",         "Seg Not Present",
    "Stack Fault",   "GP Fault",      "Page Fault",          "Reserved 15",
    "x87 FPU",       "Align Check",   "Machine Check",       "SIMD",
    "Virtualization","Control Prot",  "Reserved 22",         "Reserved 23",
    "Reserved 24",   "Reserved 25",   "Reserved 26",         "Reserved 27",
    "Hypervisor",    "VMM Comm",      "Security",            "Reserved 31",
};

// ── ISR Stubs via module-level inline asm ───────────────────────
// Each stub is exactly 16 bytes (padded). The table starts at `isr_stub_table`.
// Stub layout per vector:
//   [optional pushq $0]   (only for vectors WITHOUT a CPU-pushed error code)
//   pushq $vector
//   jmp common_isr_handler
//   .balign 16            (pad to 16 bytes)
//
// Error-code vectors: 8, 10, 11, 12, 13, 14, 17, 21, 29, 30

comptime {
    asm (
        \\.text
        \\.balign 16
        \\.global isr_stub_table
        \\isr_stub_table:
        \\
        \\// Vector 0 — Divide Error (no error code)
        \\pushq $0
        \\pushq $0
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 1 — Debug
        \\pushq $0
        \\pushq $1
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 2 — NMI
        \\pushq $0
        \\pushq $2
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 3 — Breakpoint
        \\pushq $0
        \\pushq $3
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 4 — Overflow
        \\pushq $0
        \\pushq $4
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 5 — Bound Range
        \\pushq $0
        \\pushq $5
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 6 — Invalid Opcode
        \\pushq $0
        \\pushq $6
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 7 — Device NA
        \\pushq $0
        \\pushq $7
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 8 — Double Fault (HAS error code)
        \\pushq $8
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 9 — Coprocessor (no error code, reserved)
        \\pushq $0
        \\pushq $9
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 10 — Invalid TSS (HAS error code)
        \\pushq $10
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 11 — Segment Not Present (HAS error code)
        \\pushq $11
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 12 — Stack Fault (HAS error code)
        \\pushq $12
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 13 — GP Fault (HAS error code)
        \\pushq $13
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 14 — Page Fault (HAS error code)
        \\pushq $14
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 15 — Reserved
        \\pushq $0
        \\pushq $15
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 16 — x87 FPU
        \\pushq $0
        \\pushq $16
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 17 — Alignment Check (HAS error code)
        \\pushq $17
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 18 — Machine Check
        \\pushq $0
        \\pushq $18
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 19 — SIMD
        \\pushq $0
        \\pushq $19
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 20 — Virtualization
        \\pushq $0
        \\pushq $20
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 21 — Control Protection (HAS error code)
        \\pushq $21
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vectors 22-28 reserved (no error code)
        \\pushq $0
        \\pushq $22
        \\jmp common_isr_handler
        \\.balign 16
        \\pushq $0
        \\pushq $23
        \\jmp common_isr_handler
        \\.balign 16
        \\pushq $0
        \\pushq $24
        \\jmp common_isr_handler
        \\.balign 16
        \\pushq $0
        \\pushq $25
        \\jmp common_isr_handler
        \\.balign 16
        \\pushq $0
        \\pushq $26
        \\jmp common_isr_handler
        \\.balign 16
        \\pushq $0
        \\pushq $27
        \\jmp common_isr_handler
        \\.balign 16
        \\pushq $0
        \\pushq $28
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 29 — Hypervisor Injection (HAS error code)
        \\pushq $29
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 30 — VMM Communication (HAS error code)
        \\pushq $30
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 31 — Security
        \\pushq $0
        \\pushq $31
        \\jmp common_isr_handler
        \\.balign 16
        \\
        \\// ── Common ISR Handler ──
        \\.global common_isr_handler
        \\common_isr_handler:
        \\    push %rax
        \\    push %rbx
        \\    push %rcx
        \\    push %rdx
        \\    push %rsi
        \\    push %rdi
        \\    push %rbp
        \\    push %r8
        \\    push %r9
        \\    push %r10
        \\    push %r11
        \\    push %r12
        \\    push %r13
        \\    push %r14
        \\    push %r15
        \\    mov %rsp, %rdi
        \\    call handleException
        \\    pop %r15
        \\    pop %r14
        \\    pop %r13
        \\    pop %r12
        \\    pop %r11
        \\    pop %r10
        \\    pop %r9
        \\    pop %r8
        \\    pop %rbp
        \\    pop %rdi
        \\    pop %rsi
        \\    pop %rdx
        \\    pop %rcx
        \\    pop %rbx
        \\    pop %rax
        \\    add $16, %rsp
        \\    iretq
    );
}

extern const isr_stub_table: u8;

const STUB_SIZE: usize = 16;

// ── Interrupt Frame & Exception Handler ─────────────────────────

const InterruptFrame = extern struct {
    r15: u64, r14: u64, r13: u64, r12: u64,
    r11: u64, r10: u64, r9:  u64, r8:  u64,
    rbp: u64, rdi: u64, rsi: u64, rdx: u64,
    rcx: u64, rbx: u64, rax: u64,
    vector: u64,
    error_code: u64,
    rip: u64, cs: u64, rflags: u64,
};

export fn handleException(frame: *InterruptFrame) callconv(.c) void {
    const vec: u8 = @truncate(frame.vector);
    serial.puts("\n!!! EXCEPTION #");
    serial.putDec(vec);
    serial.puts(": ");
    if (frame.vector < 32) {
        serial.puts(EXCEPTION_NAMES[frame.vector]);
    }
    serial.puts("\n    RIP=0x");
    serial.putHex(frame.rip);
    serial.puts("  CS=0x");
    serial.putHex(frame.cs);
    serial.puts("\n    ERR=0x");
    serial.putHex(frame.error_code);
    serial.puts("\nSystem halted.\n");
    while (true) {
        asm volatile ("cli; hlt");
    }
}

// ── IDT Initialization ──────────────────────────────────────────

pub fn init() void {
    const stub_base: u64 = @intFromPtr(&isr_stub_table);
    const selector = currentCs();
    serial.puts("[IDT]   Using CS selector 0x");
    serial.putHex(selector);
    serial.puts("\n");
    serial.puts("[IDT]   Building table\n");

    // Install gates for vectors 0-31 (with IST for NMI/DF/MC)
    inline for (0..32) |i| {
        idt[i] = makeGate(stub_base + i * STUB_SIZE, selector, istForVector(i));
    }

    // Build IDTR (10 bytes: limit[2] + base[8])
    var idtr: [10]u8 align(1) = undefined;
    const limit: u16 = @as(u16, @intCast(@sizeOf(@TypeOf(idt)) - 1));
    const base: u64 = @intFromPtr(&idt);
    idtr[0] = @truncate(limit);
    idtr[1] = @truncate(limit >> 8);
    inline for (0..8) |i| {
        idtr[2 + i] = @truncate(base >> (i * 8));
    }

    serial.puts("[IDT]   LIDT...\n");
    asm volatile ("lidt %[idtr]"
        :
        : [idtr] "m" (idtr),
        : .{ .memory = true }
    );
    serial.puts("[IDT]   LIDT done\n");

    serial.puts("[IDT]   Loaded (32 exception vectors)\n");
}
