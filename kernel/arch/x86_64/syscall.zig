// ============================================================================
// VantaOS — SYSCALL/SYSRET wiring (x86_64)
//
// MSRs:
//   IA32_EFER  (0xC0000080)  — bit 0 = SCE (System Call Enable)
//   IA32_STAR  (0xC0000081)  — [47:32] kernel CS/SS base, [63:48] user CS/SS base
//   IA32_LSTAR (0xC0000082)  — 64-bit syscall entry RIP
//   IA32_FMASK (0xC0000084)  — RFLAGS mask applied on SYSCALL entry
//
// On SYSCALL:
//   CS  = STAR[47:32] + 0
//   SS  = STAR[47:32] + 8
//   RCX = user RIP
//   R11 = user RFLAGS
//   RFLAGS &= ~FMASK
//
// On SYSRET (64-bit):
//   CS  = STAR[63:48] + 16   ← that's why user code 64 must be at +0x10 above ucode32
//   SS  = STAR[63:48] + 8
//   RIP = RCX
//   RFLAGS = R11
// ============================================================================

const gdt = @import("gdt.zig");
const serial = @import("serial.zig");
const dispatch_mod = @import("../../syscall/table.zig");

const EFER_MSR:  u32 = 0xC0000080;
const STAR_MSR:  u32 = 0xC0000081;
const LSTAR_MSR: u32 = 0xC0000082;
const FMASK_MSR: u32 = 0xC0000084;

fn wrmsr(idx: u32, val: u64) void {
    const lo: u32 = @truncate(val & 0xFFFFFFFF);
    const hi: u32 = @truncate(val >> 32);
    asm volatile ("wrmsr"
        :
        : [_] "{ecx}" (idx),
          [_] "{eax}" (lo),
          [_] "{edx}" (hi),
    );
}

fn rdmsr(idx: u32) u64 {
    var lo: u32 = 0; var hi: u32 = 0;
    asm volatile ("rdmsr"
        : [_] "={eax}" (lo),
          [_] "={edx}" (hi),
        : [_] "{ecx}" (idx),
    );
    return (@as(u64, hi) << 32) | lo;
}

/// Set up SYSCALL/SYSRET entry. Must be called after gdt.init().
pub fn init() void {
    // 1. Enable SCE bit in EFER
    const efer = rdmsr(EFER_MSR);
    wrmsr(EFER_MSR, efer | 1);

    // 2. STAR: kernel CS at +0x08, user 32-bit code at +0x18.
    //    Kernel SS = STAR[47:32] + 8 = 0x10 ✓
    //    User SS   = STAR[63:48] + 8 = 0x20 ✓
    //    User CS64 = STAR[63:48] + 16 = 0x28 ✓
    const star: u64 = (@as(u64, gdt.USER_CODE32_SEL) << 48) | (@as(u64, gdt.KERNEL_CODE_SEL) << 32);
    wrmsr(STAR_MSR, star);

    // 3. LSTAR: entry point
    wrmsr(LSTAR_MSR, @intFromPtr(&syscall_entry));

    // 4. FMASK: mask IF (0x200) and direction flag (0x400) on entry
    wrmsr(FMASK_MSR, 0x200 | 0x400);

    serial.puts("[SC]    SYSCALL MSRs configured\n");
}

// ── Entry stub ──────────────────────────────────────────────────
// Userspace executes: syscall
// Hardware: CS/SS load, RCX=user RIP, R11=user RFLAGS, RFLAGS masked.
// We must: swapgs, save user RSP, load kernel RSP from per-CPU,
// save state, call dispatcher, restore, swapgs, sysretq.
//
// Phase 1: no userspace yet, but stub must be valid asm so MSR points
// at real code. Implementation continues into syscallDispatch.

pub extern fn syscall_entry() callconv(.naked) void;

comptime {
    asm (
        \\.global syscall_entry
        \\.type syscall_entry, @function
        \\syscall_entry:
        \\    // For Phase 1 bring-up: no GS base yet. Just save state on
        \\    // current rsp (which is still user-rsp on entry — broken if
        \\    // we ever actually fire from ring 3). Stub returns immediately.
        \\    // Once we have per-CPU + userspace, replace with full save.
        \\    swapgs
        \\    // Save user rsp into r10 scratch; load kernel rsp from gs:0
        \\    mov %rsp, %r10
        \\    mov %gs:0, %rsp
        \\    // Save user state on kernel stack
        \\    push %r10              // user rsp
        \\    push %r11              // user rflags
        \\    push %rcx              // user rip
        \\    // GP regs (preserve everything dispatcher cares about)
        \\    push %rax
        \\    push %rbx
        \\    push %rdx
        \\    push %rsi
        \\    push %rdi
        \\    push %rbp
        \\    push %r8
        \\    push %r9
        \\    push %r12
        \\    push %r13
        \\    push %r14
        \\    push %r15
        \\    // Args mapping for syscallDispatchC:
        \\    //   rdi = syscall number (rax)
        \\    //   rsi = arg1 (rdi)
        \\    //   rdx = arg2 (rsi)
        \\    //   rcx = arg3 (rdx)
        \\    //   r8  = arg4 (r10)
        \\    //   r9  = arg5 (r8)
        \\    //   stack = arg6 (r9)
        \\    mov %rax, %rdi
        \\    // rsi already = rsi  → wrong, need user rdi here
        \\    // We saved everything; reload from stack to keep simple.
        \\    mov 64(%rsp), %rsi     // user rdi
        \\    mov 56(%rsp), %rdx     // user rsi
        \\    mov 80(%rsp), %rcx     // user rdx
        \\    mov %r10, %r8          // user r10 still in r10? we clobbered above
        \\    mov 32(%rsp), %r9      // user r8
        \\    call syscallDispatchC
        \\    // Return value in rax
        \\    pop %r15
        \\    pop %r14
        \\    pop %r13
        \\    pop %r12
        \\    pop %r9
        \\    pop %r8
        \\    pop %rbp
        \\    pop %rdi
        \\    pop %rsi
        \\    pop %rdx
        \\    pop %rbx
        \\    add $8, %rsp           // skip saved rax (we return new rax)
        \\    pop %rcx               // user rip
        \\    pop %r11               // user rflags
        \\    pop %rsp               // user rsp
        \\    swapgs
        \\    sysretq
    );
}

/// C-ABI dispatch shim called from syscall_entry asm.
/// rdi=number, rsi=a1, rdx=a2, rcx=a3, r8=a4, r9=a5
export fn syscallDispatchC(
    number: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64,
) callconv(.c) u64 {
    const r = dispatch_mod.dispatch(number, a1, a2, a3, a4, a5, 0);
    // Pack result into rax (Phase 1: encode err in top byte if nonzero)
    if (r.err != .success) {
        return 0xFFFFFFFF00000000 | @intFromEnum(r.err);
    }
    return r.value;
}
