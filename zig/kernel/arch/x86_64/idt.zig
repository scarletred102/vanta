// ============================================================================
// VantaOS — Interrupt Descriptor Table (IDT)
// Phase 0: Minimal exception handlers via module-level asm stubs.
//
// Each stub is generated at module-level inline asm with fixed 16-byte
// alignment. The IDT is built by computing addresses from a base label.
// ============================================================================

const serial = @import("serial.zig");
const sched = @import("../../sched/scheduler.zig");
const vmm = @import("../../mm/vmm.zig");
const interrupts_mod = @import("interrupts.zig");
const cpu_local = @import("cpu_local.zig");

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

const DescriptorRegister = extern struct {
    limit: u16,
    base: u64 align(1),
};

comptime {
    if (@sizeOf(IdtEntry) != 16) @compileError("IDT entry must be 16 bytes");
    if (@sizeOf(DescriptorRegister) != 10) @compileError("descriptor register must be 10 bytes");
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

pub var irq_notification_bindings: [16]?*@import("../../ipc/notification.zig").Notification = [_]?*@import("../../ipc/notification.zig").Notification{null} ** 16;

/// Per-IRQ byte ring buffer so userspace drivers can read raw data via vanta_irq_readbyte.
pub const IRQ_RING_SIZE: usize = 64;
pub const IrqRingBuffer = struct {
    data: [IRQ_RING_SIZE]u8 = [_]u8{0} ** IRQ_RING_SIZE,
    head: u8 = 0,
    tail: u8 = 0,

    pub fn push(self: *IrqRingBuffer, b: u8) void {
        const next: u8 = @truncate((@as(usize, self.tail) + 1) % IRQ_RING_SIZE);
        if (next != self.head) {
            self.data[self.tail] = b;
            self.tail = next;
        }
        // If full, silently drop — keyboard repeat events are not critical.
    }

    pub fn pop(self: *IrqRingBuffer) ?u8 {
        if (self.head == self.tail) return null;
        const b = self.data[self.head];
        self.head = @truncate((@as(usize, self.head) + 1) % IRQ_RING_SIZE);
        return b;
    }
};
pub var irq_data_buffers: [16]IrqRingBuffer = [_]IrqRingBuffer{.{}} ** 16;

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
        \\// Vector 32 — Timer IRQ
        \\pushq $0
        \\pushq $32
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 33 — Keyboard IRQ
        \\pushq $0
        \\pushq $33
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 34 — AHCI/virtio-net IRQ
        \\pushq $0
        \\pushq $34
        \\jmp common_isr_handler
        \\.balign 16
        \\// Vector 35 — PS/2 Mouse IRQ
        \\pushq $0
        \\pushq $35
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
        \\    testb $3, 144(%rsp)
        \\    jz 1f
        \\    swapgs
        \\1:
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
        \\    testb $3, 8(%rsp)
        \\    jz 2f
        \\    swapgs
        \\2:
        \\    iretq
    );
}

extern const isr_stub_table: u8;

// Naked stub for TLB shootdown IPI (vector 0x40)
comptime {
    asm (
        \\.global tlb_ipi_stub
        \\tlb_ipi_stub:
        \\    pushq %rax
        \\    pushq %rcx
        \\    pushq %rdx
        \\    pushq %rsi
        \\    pushq %rdi
        \\    pushq %r8
        \\    pushq %r9
        \\    pushq %r10
        \\    pushq %r11
        \\    callq handleTlbShootdown
        \\    popq %r11
        \\    popq %r10
        \\    popq %r9
        \\    popq %r8
        \\    popq %rdi
        \\    popq %rsi
        \\    popq %rdx
        \\    popq %rcx
        \\    popq %rax
        \\    iretq
    );
}
extern fn tlb_ipi_stub() void;

export fn handleTlbShootdown() callconv(.c) void {
    const va = @atomicLoad(u64, &vmm.shootdown_va, .acquire);
    asm volatile ("invlpg (%[v])" :: [v] "r" (va) : .{ .memory = true });
    _ = @atomicRmw(u32, &vmm.shootdown_count, .Sub, 1, .release);
    interrupts_mod.eoi();
}

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

    if (vec == 32) {
        const interrupts = @import("interrupts.zig");
        const cpu = cpu_local.get_cpu_local();
        const current_ticks = @atomicLoad(u64, &cpu.timer_ticks, .monotonic);
        if (current_ticks == 0) {
            serial.puts("[WATCHDOG] CPU ");
            serial.putDec(cpu.cpu_id);
            serial.puts(" first tick. CpuLocal=0x");
            serial.putHex(@intFromPtr(cpu));
            serial.puts(" timer_ticks=0x");
            serial.putHex(@intFromPtr(&cpu.timer_ticks));
            serial.puts("\n");
        }
        @atomicStore(u64, &cpu.timer_ticks, current_ticks + 1, .monotonic);

        if (cpu.cpu_id == 0) {
            var idx: usize = 0;
            while (idx < cpu_local.cpu_count) : (idx += 1) {
                const other_cpu = &cpu_local.cpus[idx];
                if (other_cpu.cpu_id == 0) continue;

                const other_ticks = @atomicLoad(u64, &other_cpu.timer_ticks, .monotonic);
                if (other_ticks == other_cpu.watchdog_last_ticks) {
                    other_cpu.watchdog_miss_count += 1;
                    if (other_cpu.watchdog_miss_count >= 100) {
                        serial.puts("\n!!! WATCHDOG PANIC: CPU ");
                        serial.putDec(other_cpu.cpu_id);
                        serial.puts(" LAPIC timer stalled for 1 second! (ticks: ");
                        serial.putDec(other_ticks);
                        serial.puts(")\n");
                        while (true) asm volatile ("cli; hlt");
                    }
                } else {
                    other_cpu.watchdog_last_ticks = other_ticks;
                    other_cpu.watchdog_miss_count = 0;
                }
            }
        }

        sched.tick();
        // Fire timer notification for the timer_server (IRQ 0 = LAPIC timer pseudo-IRQ).
        if (irq_notification_bindings[0]) |notif| {
            notif.notify(1);
        }
        interrupts.eoi();
        return;
    }

    if (vec == 33) {
        // PS/2 keyboard: read scancode, push to ring buffer, wake terminal.
        const cpu_io = @import("cpu.zig");
        const scancode = cpu_io.inb(0x60);
        serial.puts("[K-IRQ] sc=0x");
        serial.putHex(scancode);
        if (irq_notification_bindings[1] != null) {
            serial.puts(" bound\n");
        } else {
            serial.puts(" UNBOUND\n");
        }
        irq_data_buffers[1].push(scancode);
        if (irq_notification_bindings[1]) |notif| {
            notif.notify(1);
        }
        interrupts_mod.eoi();
        return;
    }

    if (vec == 34) {
        if (irq_notification_bindings[11]) |notif| {
            notif.notify(1);
        }
        interrupts_mod.eoi();
        return;
    }

    if (vec == 35) {
        // PS/2 mouse: read byte, push to ring buffer, wake input_server.
        const cpu_io = @import("cpu.zig");
        const mouse_byte = cpu_io.inb(0x60);
        irq_data_buffers[12].push(mouse_byte);
        if (irq_notification_bindings[12]) |notif| {
            notif.notify(1);
        }
        interrupts_mod.eoi();
        return;
    }

    if (vec == 14) {
        var cr2: u64 = 0;
        asm volatile ("mov %%cr2, %[cr2]" : [cr2] "=r" (cr2));

        const is_userspace = cr2 < 0x0000800000000000;
        const is_not_present = (frame.error_code & 1) == 0;
        const is_protection_fault = (frame.error_code & 1) != 0;
        const is_write = (frame.error_code & 2) != 0;

        if (is_userspace) {
            const pmm = @import("../../mm/pmm.zig");
            const table_mod = @import("../../syscall/table.zig");
            const cur_proc = table_mod.getCurrentProcess();

            // 1. Check Copy-on-Write (COW)
            if (is_protection_fault and is_write) {
                if (vmm.getPte(vmm.AddressSpace.current(), cr2)) |pte_ptr| {
                    if ((pte_ptr.* & vmm.PTE_COW) != 0) {
                        const old_phys = pte_ptr.* & vmm.ADDR_MASK;
                        const ref = pmm.getRefCount(old_phys);

                        if (ref == 1) {
                            // Only 1 reference: mark writable and clear COW
                            pte_ptr.* = (pte_ptr.* & ~vmm.PTE_COW) | vmm.PTE_WRITE;
                            vmm.invlpg(cr2);

                            return;
                        } else if (ref > 1) {
                            // More than 1 reference: copy-on-write allocate & copy
                            if (pmm.allocPage()) |new_phys| {
                                const old_virt = vmm.phys2virt(old_phys);
                                const new_virt = vmm.phys2virt(new_phys);

                                const old_ptr = @as([*]const u8, @ptrFromInt(old_virt));
                                const new_ptr = @as([*]u8, @ptrFromInt(new_virt));
                                var idx: usize = 0;
                                while (idx < pmm.PAGE_SIZE) : (idx += 1) {
                                    new_ptr[idx] = old_ptr[idx];
                                }

                                pmm.unrefPage(old_phys);
                                const flags = (pte_ptr.* & ~vmm.ADDR_MASK & ~vmm.PTE_COW) | vmm.PTE_WRITE;
                                pte_ptr.* = new_phys | flags;
                                vmm.invlpg(cr2);

                                return;
                            }
                        }
                    }
                }
            }

            // 2. Check Lazy demand paging (VMA)
            if (is_not_present) {
                if (cur_proc.findVma(cr2)) |vma| {
                    if (vma.lazy) {
                        if (pmm.allocPage()) |phys| {
                            const virt_addr = vmm.phys2virt(phys);
                            const ptr = @as([*]volatile u8, @ptrFromInt(virt_addr));
                            var idx: usize = 0;
                            while (idx < pmm.PAGE_SIZE) : (idx += 1) ptr[idx] = 0;

                            const page_vaddr = cr2 & ~@as(u64, 0xFFF);
                            const flags = vmm.PTE_USER | (if ((vma.flags & vmm.PTE_WRITE) != 0) vmm.PTE_WRITE else 0);
                            if (vmm.map(vmm.AddressSpace.current(), page_vaddr, phys, flags)) {
                                return;
                            }
                        }
                    }
                }
            }

            // If we are here, userspace page fault is fatal
            serial.puts("[FAULT] Fatal Page Fault in user space. CR2=0x");
            serial.putHex(cr2);
            serial.puts("  RIP=0x");
            serial.putHex(frame.rip);
            serial.puts("  ERR=0x");
            serial.putHex(frame.error_code);
            serial.puts(". Terminating process.\n");

            sched.exitCurrentThread();
        }

        // Kernel space page fault remains a hard panic
        serial.puts("\n!!! EXCEPTION #14: Kernel Page Fault\n");
        serial.puts("    RIP=0x");
        serial.putHex(frame.rip);
        serial.puts("  CS=0x");
        serial.putHex(frame.cs);
        serial.puts("\n    ERR=0x");
        serial.putHex(frame.error_code);
        serial.puts("  CR2=0x");
        serial.putHex(cr2);
        serial.puts("\nSystem halted.\n");
        while (true) {
            asm volatile ("cli; hlt");
        }
    }

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
    serial.puts("\n    RFLAGS=0x");
    serial.putHex(frame.rflags);
    serial.puts("\n    RAX=0x");
    serial.putHex(frame.rax);
    serial.puts("  RBX=0x");
    serial.putHex(frame.rbx);
    serial.puts("  RCX=0x");
    serial.putHex(frame.rcx);
    serial.puts("\n    RDX=0x");
    serial.putHex(frame.rdx);
    serial.puts("  RSI=0x");
    serial.putHex(frame.rsi);
    serial.puts("  RDI=0x");
    serial.putHex(frame.rdi);
    serial.puts("\n    R8=0x");
    serial.putHex(frame.r8);
    serial.puts("  R9=0x");
    serial.putHex(frame.r9);
    serial.puts("  R10=0x");
    serial.putHex(frame.r10);
    serial.puts("\n    R11=0x");
    serial.putHex(frame.r11);
    serial.puts("  R12=0x");
    serial.putHex(frame.r12);
    serial.puts("  R13=0x");
    serial.putHex(frame.r13);
    serial.puts("\n    R14=0x");
    serial.putHex(frame.r14);
    serial.puts("  R15=0x");
    serial.putHex(frame.r15);
    serial.puts("\n    RBP=0x");
    serial.putHex(frame.rbp);
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

    // Install gates for vectors 0-35 (with IST for NMI/DF/MC)
    inline for (0..36) |i| {
        idt[i] = makeGate(stub_base + i * STUB_SIZE, selector, istForVector(i));
    }

    // Vector 0x40: TLB shootdown IPI
    idt[0x40] = makeGate(@intFromPtr(&tlb_ipi_stub), selector, 0);

    // Build IDTR (10 bytes: limit[2] + base[8])
    const idtr = DescriptorRegister{
        .limit = @as(u16, @intCast(@sizeOf(@TypeOf(idt)) - 1)),
        .base = @intFromPtr(&idt),
    };

    serial.puts("[IDT]   LIDT...\n");
    asm volatile ("lidt (%[idtr])"
        :
        : [idtr] "r" (&idtr),
        : .{ .memory = true }
    );
    serial.puts("[IDT]   LIDT done\n");

    serial.puts("[IDT]   Loaded (32 exception vectors)\n");
}
