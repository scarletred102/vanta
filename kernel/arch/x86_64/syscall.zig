// ============================================================================
// VantaOS — SYSCALL/SYSRET MSR Setup & Ring 3 Entry (Phase 2)
//
// STAR[47:32] = kernel CS (0x08), STAR[63:48] = 0x1B (→ user SS=0x20, CS=0x28)
// LSTAR = address of syscall_entry stub
// FMASK = clears RFLAGS.IF on syscall entry
// ============================================================================

const serial = @import("serial.zig");
const gdt = @import("gdt.zig");

const MSR_STAR: u32 = 0xC0000081;
const MSR_LSTAR: u32 = 0xC0000082;
const MSR_FMASK: u32 = 0xC0000084;
const MSR_EFER: u32 = 0xC0000080;

const syscall_table = @import("../../syscall/syscall_table.zig");

inline fn rdmsr(msr: u32) u64 {
    var lo: u32 = 0;
    var hi: u32 = 0;
    asm volatile ("rdmsr"
        : [lo] "={eax}" (lo),
          [hi] "={edx}" (hi),
        : [msr] "{ecx}" (msr),
    );
    return @as(u64, hi) << 32 | lo;
}

inline fn wrmsr(msr: u32, val: u64) void {
    const lo: u32 = @truncate(val & 0xFFFFFFFF);
    const hi: u32 = @truncate(val >> 32);
    asm volatile ("wrmsr"
        :
        : [msr] "{ecx}" (msr),
          [lo] "{eax}" (lo),
          [hi] "{edx}" (hi),
    );
}

// Per-CPU storage layout at GS base:
//   offset 0  = self pointer (u64)
//   offset 8  = kernel RSP for syscall entry (u64) — kstack_top of current thread
//   offset 16 = scratch for user RSP save (u64)
// We use a simple static struct for Phase 2 (single-CPU).
pub const CpuLocal = extern struct {
    self_ptr: u64 = 0,
    kernel_rsp: u64 = 0,
    user_rsp_scratch: u64 = 0,
};

var bsp_cpu_local: CpuLocal = .{};

fn writeMsrGsBase(val: u64) void {
    const MSR_GS_BASE: u32 = 0xC0000101;
    wrmsr(MSR_GS_BASE, val);
}

// Kernel stack for syscall entry fallback
var syscall_kernel_stack: [16 * 1024]u8 align(16) = [_]u8{0} ** (16 * 1024);

// Called from the kernel entry stub; receives syscall number in rax and args.
export fn syscall_dispatch_from_asm(
    number: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64
) callconv(.c) u64 {
    _ = a6;
    const result = syscall_table.dispatch(number, a1, a2, a3, a4, a5, 0);
    return result.value;
}

pub fn init() void {
    // Enable SCE (System Call Extensions) in EFER
    const efer = rdmsr(MSR_EFER);
    wrmsr(MSR_EFER, efer | 1);

    // STAR: bits[47:32] = kernel CS=0x08, bits[63:48] = 0x1B
    const star: u64 = (@as(u64, 0x1B) << 48) | (@as(u64, gdt.KERNEL_CODE_SEL) << 32);
    wrmsr(MSR_STAR, star);

    // Set up GS base to point to CpuLocal so the syscall entry stub can swap stacks.
    bsp_cpu_local.self_ptr = @intFromPtr(&bsp_cpu_local);
    bsp_cpu_local.kernel_rsp = @intFromPtr(&syscall_kernel_stack) + syscall_kernel_stack.len;
    writeMsrGsBase(@intFromPtr(&bsp_cpu_local));

    // LSTAR: address of syscall_entry (defined in inline asm below via wrapper)
    wrmsr(MSR_LSTAR, @intFromPtr(&syscall_entry_wrapper));

    // FMASK: clear RFLAGS.IF on syscall entry
    wrmsr(MSR_FMASK, 0x200);

    serial.puts("[SYSCALL] MSRs configured (STAR/LSTAR/FMASK/SCE)\n");
    serial.puts("[SYSCALL] Entry at 0x");
    serial.putHex(@intFromPtr(&syscall_entry_wrapper));
    serial.puts("\n");
}

// Update the kernel RSP in CpuLocal when the scheduler switches threads.
// Called by scheduler after setRsp0.
pub fn setCpuKernelRsp(rsp: u64) void {
    bsp_cpu_local.kernel_rsp = rsp;
}

// The SYSCALL entry wrapper: saves registers, dispatches, restores, SYSRETQs.
// Uses extern C convention so linker can find it.
export fn syscall_entry_wrapper() callconv(.naked) void {
    // On entry: RCX = user RIP, R11 = user RFLAGS, RSP = user stack (untrusted)
    // We swap to the kernel stack stored at GS:8.
    asm volatile (
        \\ swapgs
        \\ movq %%rsp, %%gs:16
        \\ movq %%gs:8, %%rsp
        \\ subq $8, %%rsp
        \\ andq $-16, %%rsp
        \\ pushq %%rcx
        \\ pushq %%r11
        \\ pushq %%rdi
        \\ pushq %%rsi
        \\ pushq %%rdx
        \\ pushq %%r10
        \\ pushq %%r8
        \\ pushq %%r9
        \\ pushq %%rbx
        \\ pushq %%rbp
        \\ movq %%r10, %%rcx
        \\ callq syscall_dispatch_from_asm
        \\ popq %%rbp
        \\ popq %%rbx
        \\ popq %%r9
        \\ popq %%r8
        \\ popq %%r10
        \\ popq %%rdx
        \\ popq %%rsi
        \\ popq %%rdi
        \\ popq %%r11
        \\ popq %%rcx
        \\ movq %%gs:16, %%rsp
        \\ swapgs
        \\ sysretq
    );
}

// Ring 0 → Ring 3 transition via IRET
pub fn enter_userspace(entry_addr: u64, stack: u64, _arg0: u64) noreturn {
    _ = _arg0;
    // Set user data segments
    asm volatile (
        \\ movw $0x23, %%ax
        \\ movw %%ax, %%ds
        \\ movw %%ax, %%es
        \\ movw %%ax, %%fs
        \\ movw %%ax, %%gs
        :
        :
        : "rax"
    );
    // IRET frame: SS, RSP, RFLAGS, CS, RIP
    asm volatile (
        \\ pushq $0x23
        \\ pushq %[stack]
        \\ pushq $0x202
        \\ pushq $0x2B
        \\ pushq %[entry]
        \\ iretq
        :
        : [entry] "r" (entry_addr),
          [stack] "r" (stack),
    );
    unreachable;
}

pub fn verifySyscallFromRing3() void {
    const lstar = rdmsr(MSR_LSTAR);
    const expected = @intFromPtr(&syscall_entry_wrapper);
    if (lstar != expected) {
        serial.puts("[SYSCALL] WARN: LSTAR readback mismatch!\n");
    } else {
        serial.puts("[SYSCALL] LSTAR verified OK\n");
    }

    const star = rdmsr(MSR_STAR);
    const kern_cs = @as(u16, @truncate((star >> 32) & 0xFFFF));
    if (kern_cs != gdt.KERNEL_CODE_SEL) {
        serial.puts("[SYSCALL] WARN: STAR kernel CS mismatch!\n");
    } else {
        serial.puts("[SYSCALL] STAR kernel CS verified OK\n");
    }
}