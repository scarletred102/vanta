# VantaOS Phase 1: Context Switching & Scheduler Implementation Guide

## Overview

This guide ties together context switching, SYSCALL/SYSRET, and round-robin scheduling for VantaOS Phase 1 microkernel on x86_64. All source files are Zig + assembly-free for early integration.

---

## Files Provided

1. **VantaOS_Phase1_Context_Switch_Reference.md** — Comprehensive technical reference
   - Register layouts and save/restore sequences
   - SYSCALL/SYSRET MSR configuration
   - TCB structure and scheduler architecture
   - Memory layout and bootstrap procedure

2. **context_switch.zig** — Zig implementation of scheduler and context struct
   - `Context` struct (200 bytes, all registers)
   - `Task` and `Scheduler` structs
   - `init_syscall_mechanism()`
   - Round-robin scheduling logic
   - Helper functions for task creation

3. **context_switch.s** — x86_64 assembly routines
   - `save_context(Context *rdi)` — serialize CPU state to Context
   - `restore_context(Context *rdi)` — load CPU state from Context and jump
   - `setup_syscall_msrs()` — configure IA32_STAR, IA32_LSTAR, IA32_FMASK, IA32_EFER
   - `load_tss(u16 selector)` — load TSS into Task Register
   - `syscall_entry` — kernel entry point for SYSCALL
   - `sysret_exit` — return to user via SYSRET

---

## Integration Checklist

### 1. Assemble and Link

```bash
# Assemble context_switch.s
as -o context_switch.o context_switch.s

# Compile context_switch.zig with your kernel
zig build-obj context_switch.zig -target x86_64-freestanding -mcpu=x86-64
zig cc ... context_switch.o context_switch_zig.o ...
```

### 2. GDT Setup (Required Before SYSCALL)

Ensure your GDT has:

```zig
// GDT entries (order matters for IA32_STAR):
const GDT = [_]u64{
    0,                               // 0x00: Null
    0x00209A0000000000,             // 0x08: Kernel CS (64-bit)
    0x0020920000000000,             // 0x10: Kernel SS
    0x0020FA0000000000,             // 0x18: User CS (64-bit, DPL=3)
    0x0020F20000000000,             // 0x20: User SS (DPL=3)
    // TSS entries follow at 0x28...
};

// Load GDT
lgdt [gdtr]  // Assembly: load GDTR

// Set segment registers
mov ax, 0x10
mov ss, ax
mov ds, ax
mov es, ax

// Load TSS (0x28 = GDT[5])
ltr 0x28
```

### 3. Initialize Scheduler

```zig
pub fn kernel_init() void {
    // ... other kernel setup ...

    // 1. Initialize scheduler
    context_switch.init_scheduler();

    // 2. Allocate kernel stacks for tasks
    var task1_kernel_stack = try allocate_stack(4096);
    var task2_kernel_stack = try allocate_stack(4096);

    // 3. Create init task (user)
    var init_task = context_switch.create_user_task(
        1,                      // task ID
        0x400000,              // user entry point RIP
        0x700000,              // user stack pointer
        task1_kernel_stack,
        4096
    );
    try context_switch.add_task_to_scheduler(init_task);

    // 4. Create idle task (kernel)
    var idle_task = context_switch.create_kernel_task(
        0,                      // task ID
        @intFromPtr(&idle_loop),
        @intFromPtr(task2_kernel_stack) + 4096
    );
    try context_switch.add_task_to_scheduler(idle_task);

    // 5. Setup SYSCALL
    context_switch.init_syscall_mechanism(@intFromPtr(&context_switch.syscall_entry));

    // 6. Jump to first task
    var ctx = init_task.context;
    restore_context(&ctx);
    // Never returns; CPU jumps to init task RIP
}
```

### 4. Timer Interrupt Handler

```zig
pub fn handle_timer_interrupt(interrupted_context: *Context) void {
    // Save current task context (already populated by interrupt handler)
    context_switch.global_scheduler.current_task_mut().context = interrupted_context.*;

    // Schedule next task
    context_switch.global_scheduler.schedule_next();

    // Load next task context
    const next_task = context_switch.global_scheduler.current_task_mut();
    interrupted_context.* = next_task.context;

    // Return from interrupt will restore next_task's context via restore_context
}

// In IDT:
pub const IDTEntry = struct {
    .vector = TIMER_IRQ,
    .handler = timer_interrupt_wrapper,  // asm wrapper to save context
    .dpl = 0,
};
```

### 5. Interrupt Handler Assembly Wrapper

```asm
# This is called before handle_timer_interrupt
.globl timer_interrupt_wrapper
timer_interrupt_wrapper:
    # Hardware has already pushed SS, RSP, RFLAGS, CS, RIP
    # Push remaining registers to match interrupt frame

    push rax
    push rcx
    push rdx
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11

    # Call save_context
    lea rdi, [rsp - 128]       # Pointer to pre-allocated context on kernel stack
    call save_context

    # Call Zig timer handler
    mov rdi, rdi               # Pass context pointer
    call handle_timer_interrupt

    # Context is updated by timer handler, now restore
    call restore_context
    # restore_context is noreturn, jumps to next task
```

---

## Context Switch Flow (Step-by-Step)

### Timer Interrupt → Task Switch

1. **Timer fires** → CPU saves SS, RSP, RFLAGS, CS, RIP on current task's kernel stack
2. **CPU jumps to IDT handler** → interrupt_handler_wrapper
3. **Wrapper saves remaining GPRs** → push rax, rcx, rdx, ... r11
4. **Wrapper calls save_context()** → serializes all CPU state to Context struct
5. **Wrapper calls Zig timer handler** → calls `handle_timer_interrupt(context)`
6. **Timer handler:**
   - Saves current task's context: `tasks[current].context = context`
   - Schedules next task: `schedule_next()`
   - Loads next task's context: `context = tasks[next].context`
7. **Wrapper calls restore_context()** → pops GPRs from struct, loads RSP, jumps to RIP
8. **Next task resumes** at saved RIP, with all registers restored

### Syscall Entry → Handler → Sysret Exit

1. **User calls `syscall`** → RCX=user_rip, R11=user_rflags (hardware auto-saves)
2. **CPU jumps to IA32_LSTAR** → syscall_entry assembly
3. **syscall_entry:**
   - `swapgs` → switch to kernel GS
   - Load kernel RSP from per-CPU area
   - Push R11, RCX, RAX (user state)
   - Push RDI, RSI, RDX, R10, R8, R9
   - Call syscall_dispatcher()
4. **syscall_dispatcher** (Zig) → switch on RAX (syscall number), call handler
5. **Handler executes** (e.g., read, write, fork)
   - If preemption needed: call context switch, update context
   - Otherwise: set return value in RAX
6. **Return from handler** → fall through to sysret_exit
7. **sysret_exit:**
   - Pop R9, R8, R10, RDX, RSI, RDI
   - Pop RAX (result), RCX (user_rip), R11 (user_rflags)
   - `swapgs` → switch back to user GS
   - `sysret` → jump to RCX, restore RFLAGS from R11, CPL←3

---

## Key Data Structures

### Context (200 bytes, offset reference)

| Offset | Field | Size | Notes |
|--------|-------|------|-------|
| 0x00-0x70 | RAX-R15 | 15×8 | General purpose registers |
| 0x78 | RIP | 8 | Instruction pointer |
| 0x80 | RSP | 8 | Stack pointer |
| 0x88 | RFLAGS | 8 | Processor flags |
| 0x90-0x9A | CS-GS | 6×2 | Segment selectors |
| 0xA0 | FS.base | 8 | FS segment base |
| 0xA8 | GS.base | 8 | GS segment base |
| 0xB0 | id | 8 | Task ID |
| 0xB8 | kernel_rsp | 8 | Kernel stack for syscall |

### Task

| Field | Type | Notes |
|-------|------|-------|
| id | u64 | Unique task identifier |
| context | Context | CPU state (200 bytes) |
| state | TaskState | Running, Blocked, Sleeping, Dead |
| priority | u8 | Currently unused (for future prioritization) |
| kernel_stack_base | [*]u8 | Base of per-task kernel stack |
| kernel_stack_size | usize | Size of kernel stack |

### Scheduler

| Field | Type | Notes |
|-------|------|-------|
| tasks | [MAX_TASKS]Task | Task array (64 max) |
| task_count | usize | Current number of tasks |
| current_task_index | usize | Index into tasks array |
| ticks | u64 | Timer ticks (for scheduling) |

---

## SYSCALL MSR Configuration Example

```zig
// In kernel init
pub fn setup_syscall() void {
    const kernel_cs: u64 = 0x08;
    const user_cs: u64 = 0x18;

    // IA32_STAR:
    //   bits [47:32] = kernel CS (for SYSCALL entry)
    //   bits [63:48] = user CS (for SYSRET, SS = user CS + 8)
    const star: u64 = (user_cs << 48) | (kernel_cs << 32);

    // IA32_LSTAR: kernel syscall handler
    const lstar: u64 = @intFromPtr(&syscall_entry);

    // IA32_FMASK: clear IF, TF, DF on SYSCALL
    const fmask: u64 = 0x700;

    context_switch.init_syscall_mechanism(lstar);
    // OR call setup_syscall_msrs(lstar, star, fmask) directly if you
    // want to override the default STAR value
}
```

---

## First Boot / Bootstrap

```zig
pub fn main() noreturn {
    // Initialize GDT, IDT, paging, etc.
    init_gdt();
    init_idt();
    init_paging();

    // Initialize kernel memory allocator
    kernel_heap.init();

    // Initialize scheduler
    context_switch.init_scheduler();

    // Create and register first task (init)
    const init_entry = @intFromPtr(&user_init_entry);
    const init_stack_top = KERNEL_HEAP + 0x100000; // User stack grows down
    var init_task = context_switch.create_user_task(
        1,
        init_entry,
        init_stack_top,
        user_stack_kernel_part,
        KERNEL_STACK_SIZE
    );
    try context_switch.add_task_to_scheduler(init_task);

    // Setup SYSCALL/SYSRET
    context_switch.init_syscall_mechanism(@intFromPtr(&context_switch.syscall_entry));

    // Enable interrupts (if not already)
    asm volatile ("sti");

    // Jump to first task (never returns)
    kernel_init_final();
}

fn kernel_init_final() noreturn {
    const task = context_switch.current_task();
    var ctx = task.context;
    context_switch.restore_context(&ctx);
    // Unreachable
    while (true) {}
}
```

---

## Testing Checklist

- [ ] Compile Zig + assembly without errors
- [ ] GDT loaded correctly (verify via debugger)
- [ ] SYSCALL MSRs configured (rdmsr IA32_LSTAR in debugger)
- [ ] Timer interrupt fires and context switches
- [ ] First user task enters correctly
- [ ] User task can make syscall (SYSCALL instruction executes)
- [ ] Syscall handler returns via SYSRET
- [ ] Multiple tasks round-robin correctly
- [ ] Context is preserved across switches (all registers match)

---

## Common Pitfalls

### 1. RSP Not Switched on SYSCALL
- **Problem**: Kernel tries to use user RSP, crashes
- **Fix**: Load kernel RSP from per-CPU area or TSS before pushing anything
- **Code**: `mov rsp, [gs:KERNEL_RSP_OFFSET]` or similar

### 2. RIP/RSP Offsets Wrong in Context Struct
- **Problem**: Context loads wrong RIP/RSP, jumps to garbage or wrong stack
- **Fix**: Verify offsets match assembly save/restore (0x78 for RIP, 0x80 for RSP)
- **Debug**: Print context fields and compare to CPU state in debugger

### 3. RFLAGS IF Bit Not Set for User Tasks
- **Problem**: User task never receives interrupts (timer doesn't preempt)
- **Fix**: Set RFLAGS = 0x202 when creating user task context
- **Code**: `context.rflags = 0x202` (bit 9 = IF)

### 4. GDT Selectors Wrong in STAR
- **Problem**: SYSRET loads wrong CS/SS, user code in kernel mode or vice versa
- **Fix**: Double-check IA32_STAR layout: bits [47:32] = kernel, [63:48] = user
- **Code**: `star = (user_cs << 48) | (kernel_cs << 32)`

### 5. SWAPGS Not Called
- **Problem**: Kernel tries to access user GS.base or vice versa, corrupts memory
- **Fix**: Call `swapgs` at SYSCALL entry and exit
- **Code**: `swapgs` before loading kernel RSP, `swapgs` before SYSRET

### 6. Kernel Stack Per Task Not Allocated
- **Problem**: Syscall entry tries to push to unallocated memory
- **Fix**: Allocate per-task kernel stacks before adding task to scheduler
- **Code**: `let ks = allocate_stack(4096); task.kernel_rsp = ks + 4096;`

---

## Performance Notes

- **Context switch time**: ~50-100 cycles (save/restore all regs + scheduler logic)
- **SYSCALL entry**: ~10-20 cycles (hardware auto-saves RCX, R11)
- **Round-robin latency**: ~100 cycles per task (timer + context switch)
- Optimize by:
  - Reducing number of registers saved (e.g., skip FS/GS.base if unused)
  - Inlining scheduler logic
  - Using faster memory access patterns (cache-aligned stacks)

---

## Next Steps

1. **Integrate files** into your VantaOS build system
2. **Implement GDT, IDT, timer interrupt** (use provided Context struct)
3. **Compile and test** with single task first (no scheduling)
4. **Add second task** and verify round-robin switching
5. **Implement syscall handler** (dispatcher in context_switch.zig)
6. **Add signal handling / preemption** (if needed)

---

## References

- Intel 64 and IA-32 Architectures SDM Vol. 2B (SYSCALL, SYSRET)
- x86-64 System V ABI (register usage, calling conventions)
- OSDev Wiki (Context Switching, SYSCALL/SYSENTER)
