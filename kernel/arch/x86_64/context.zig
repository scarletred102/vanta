// ============================================================================
// VantaOS — Kernel-mode Context Switch (x86_64 System V ABI)
//
// Saves only callee-saved regs (rbx, rbp, r12-r15) + RSP + RFLAGS.
// Caller-saved regs are spilled by Zig at the call site.
//
// Thread stack layout (top → bottom on a fresh thread):
//   [unused return address]
//   [rip = entry_fn]
//   [r15 = 0]
//   [r14 = 0]
//   [r13 = 0]
//   [r12 = 0]
//   [rbp = 0]
//   [rbx = 0]
//   [rflags = 0x2]     ← rsp points here after thread_init_stack
// ============================================================================

/// Save current thread's regs to *old_rsp, switch to new_rsp.
///   rdi = pointer to old thread's rsp slot (write u64)
///   rsi = new thread's saved rsp (u64)
pub extern fn switch_context(old_rsp: *u64, new_rsp: u64) callconv(.c) void;

comptime {
    asm (
        \\.global switch_context
        \\.type switch_context, @function
        \\switch_context:
        \\    pushfq
        \\    push %rbx
        \\    push %rbp
        \\    push %r12
        \\    push %r13
        \\    push %r14
        \\    push %r15
        \\    mov %rsp, (%rdi)       // *old_rsp = rsp
        \\    mov %rsi, %rsp         // rsp = new_rsp
        \\    pop %r15
        \\    pop %r14
        \\    pop %r13
        \\    pop %r12
        \\    pop %rbp
        \\    pop %rbx
        \\    popfq
        \\    ret
    );
}

/// Prepare a fresh thread's kernel stack so first switch_context() returns
/// into `entry`. `stack_top` is the highest address of the kstack.
/// Returns the rsp value to seed in the Thread struct.
pub fn initStack(stack_top: u64, entry: u64) u64 {
    var rsp = stack_top;
    // Leave one dummy return slot so entry sees the normal SysV stack alignment.
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = 0;

    // Push frame: rip, r15, r14, r13, r12, rbp, rbx, rflags
    // (in order high→low on stack)
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = entry;       // RIP
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = 0;           // r15
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = 0;           // r14
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = 0;           // r13
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = 0;           // r12
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = 0;           // rbp
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = 0;           // rbx
    rsp -= 8; @as(*u64, @ptrFromInt(rsp)).* = 0x2;         // rflags (IF=0 until IRQs exist)
    return rsp;
}
