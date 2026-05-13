# VantaOS Phase 1: x86_64 Context Switching and Scheduler Reference

## 1. Register State to Save/Restore

### General Purpose Registers (16 total)
```
RAX, RBX, RCX, RDX, RSI, RDI, RBP (callee-saved)
R8-R15
```

### Control Flow Registers (Must Save)
- **RIP**: Instruction pointer (return address)
- **RSP**: Stack pointer (kernel or user stack)
- **RFLAGS**: Processor flags (IF, CF, ZF, TF, DF, OF, SF, etc.)

### Segment Registers
- **CS**: Code segment (x86_64 GDT selector; implicit from SYSCALL/SYSRET)
- **SS**: Stack segment (x86_64 GDT selector; implicit from SYSCALL/SYSRET)
- **DS, ES, FS, GS**: Data segments (rarely used in 64-bit, but FS/GS have bases)

### Extended State (Optional for Phase 1)
- **FS.base, GS.base**: Segment base addresses (set via WRMSR, used for TLS/kernel structures)
- **SSE/AVX state**: XMM0-XMM15 (not needed unless floating-point syscalls)

---

## 2. SYSCALL/SYSRET Mechanism (x86_64 Fast System Call)

### Hardware Automatic Actions on SYSCALL

**Instruction**: `SYSCALL` (opcode `0F 05`)

| Register | Before SYSCALL | After SYSCALL | Used By |
|----------|----------------|---------------|---------|
| **RCX** | User RIP | Next instruction RIP (user return point) | Kernel reads for return address |
| **R11** | User RFLAGS | User RFLAGS (saved) | Kernel reads for flag restoration |
| **RIP** | User code | IA32_LSTAR MSR value → kernel handler | Jump to handler |
| **CS** | User CS | bits[47:32] of IA32_STAR (kernel CS) | Kernel code segment |
| **SS** | User SS | bits[47:32] of IA32_STAR (kernel SS) | Kernel stack segment |
| **RFLAGS** | User flags | Masked by IA32_FMASK MSR | Clear IF, TF, DF if needed |
| **RSP** | User RSP | **NOT SAVED** (OS responsibility) | Must save before/after SYSCALL |
| **CPL** | 3 (user) | 0 (kernel) | Implicit privilege level change |

### Kernel Responsibility on SYSCALL Entry
1. **Save user RSP** before changing stack (either in R13/R12 or on kernel stack)
2. **Load kernel RSP** from TSS RSP0 or pre-saved value
3. **Use SWAPGS** if kernel uses separate GS base
4. Save any additional registers (RAX-RDX, RSI, RDI, R8-R10)

### Hardware Automatic Actions on SYSRET

**Instruction**: `SYSRET` (opcode `0F 07`)

| Register | Before SYSRET | After SYSRET | Restored From |
|----------|---------------|--------------|---------------|
| **RCX** | Kernel value | User RIP | Kernel must restore from saved |
| **R11** | Kernel value | User RFLAGS | Kernel must restore from saved |
| **RIP** | Kernel code | RCX value | Kernel must set RCX |
| **RFLAGS** | Kernel flags | R11 value | Kernel must set R11 |
| **CS** | Kernel CS | bits[63:48] of IA32_STAR + 16 (user CS) | IA32_STAR |
| **SS** | Kernel SS | bits[63:48] of IA32_STAR + 24 (user SS) | IA32_STAR |
| **CPL** | 0 (kernel) | 3 (user) | Implicit |

### MSR Configuration for SYSCALL/SYSRET

#### IA32_STAR (MSR 0xC0000081)
```
[63:48] = User Code Segment + 16 (user SS + 8, for SYSRET to use)
[47:32] = Kernel Code Segment (for SYSCALL entry)
[31:0]  = Unused
```
**Example**: Kernel CS=0x08, User CS=0x18
```asm
mov rax, 0x0018000800000000  ; CS kernel=0x08, SS kernel=0x10, CS user=0x18
mov rcx, 0xC0000081
mov rdx, 0
wrmsr
```

#### IA32_LSTAR (MSR 0xC0000082)
```
Points to kernel syscall handler entry point
Must be canonical address
```

#### IA32_FMASK (MSR 0xC0000084)
```
Mask applied to RFLAGS on SYSCALL entry
Typical: 0x700 (clears IF, TF, DF)
```

#### IA32_EFER (MSR 0xC0000080)
```
Bit 0 (SCE) = SYSCALL Enable (must set to 1)
```

#### IA32_KERNEL_GS_BASE (MSR 0xC0000102) — Optional
```
Used with SWAPGS to switch GS between user/kernel
```

---

## 3. Context Struct Layout (TCB - Task Control Block)

```zig
pub const Context = struct {
    // === General Purpose Registers (16 × 8 bytes = 128 bytes) ===
    rax: u64,   // 0x00
    rbx: u64,   // 0x08
    rcx: u64,   // 0x10
    rdx: u64,   // 0x18
    rsi: u64,   // 0x20
    rdi: u64,   // 0x28
    rbp: u64,   // 0x30
    r8:  u64,   // 0x38
    r9:  u64,   // 0x40
    r10: u64,   // 0x48
    r11: u64,   // 0x50
    r12: u64,   // 0x58
    r13: u64,   // 0x60
    r14: u64,   // 0x68
    r15: u64,   // 0x70

    // === Control Flow (24 bytes) ===
    rip: u64,   // 0x78 (instruction pointer / return address)
    rsp: u64,   // 0x80 (stack pointer)
    rflags: u64,// 0x88 (processor flags)

    // === Segment Registers (12 bytes, padded to 16) ===
    cs: u16,    // 0x90
    ss: u16,    // 0x92
    ds: u16,    // 0x94
    es: u16,    // 0x96
    fs: u16,    // 0x98
    gs: u16,    // 0x9A

    // === Extended State (16 bytes) ===
    fs_base: u64,  // 0xA0 (FS segment base address)
    gs_base: u64,  // 0xA8 (GS segment base address)

    // === Task Metadata (32 bytes) ===
    id: u64,       // 0xB0 (task ID)
    kernel_rsp: u64, // 0xB8 (kernel stack pointer for syscall entry)
    state: u8,     // 0xC0 (Running, Blocked, Sleeping, etc.)
    priority: u8,  // 0xC1
    // padding: 6 bytes to 0xC8
    
    // Total: 0xC8 (200 bytes)
};
```

**Alignment Notes**:
- Use `align(16)` on Context struct for efficiency
- GPRs are naturally 8-byte aligned
- RSP should be 16-byte aligned before function calls (ABI requirement)

---

## 4. Save/Restore Assembly Sequences

### Save Context (on timer interrupt or manual switch)

```asm
; void save_context(Context *rdi)
; Assumes all registers except RDI are the register state to save
; RDI = &context_struct

save_context:
    ; Save general purpose registers
    mov [rdi + 0x00], rax
    mov [rdi + 0x08], rbx
    mov [rdi + 0x10], rcx
    mov [rdi + 0x18], rdx
    mov [rdi + 0x20], rsi
    ; RDI saved later, after we're done with it
    mov [rdi + 0x30], rbp
    mov [rdi + 0x38], r8
    mov [rdi + 0x40], r9
    mov [rdi + 0x48], r10
    mov [rdi + 0x50], r11
    mov [rdi + 0x58], r12
    mov [rdi + 0x60], r13
    mov [rdi + 0x68], r14
    mov [rdi + 0x70], r15

    ; Save RFLAGS
    pushfq
    pop qword [rdi + 0x88]

    ; Save control flow registers
    ; RIP must come from caller (interrupt frame or explicit)
    mov r8, [rsp]          ; or passed as argument
    mov [rdi + 0x78], r8   ; RIP

    ; RSP: if called from interrupt handler, RSP already points to user stack
    ; Adjust if needed (after interrupt, RSP has been decremented)
    lea r8, [rsp + 8]      ; or calculate based on context
    mov [rdi + 0x80], r8   ; RSP

    ; Save segments (usually kernel CS/SS, but preserve user context)
    mov ax, cs
    mov [rdi + 0x90], ax
    mov ax, ss
    mov [rdi + 0x92], ax

    ; Save RDI last
    mov [rdi + 0x28], rdi

    ret
```

### Restore Context (jump to next task)

```asm
; void restore_context(Context *rdi)
; Restore registers and jump to saved RIP
; RDI = &context_struct

restore_context:
    ; Restore all GPRs except RDI, RSP, RIP
    mov rax, [rdi + 0x00]
    mov rbx, [rdi + 0x08]
    mov rcx, [rdi + 0x10]
    mov rdx, [rdi + 0x18]
    mov rsi, [rdi + 0x20]
    mov rbp, [rdi + 0x30]
    mov r8,  [rdi + 0x38]
    mov r9,  [rdi + 0x40]
    mov r10, [rdi + 0x48]
    mov r11, [rdi + 0x50]
    mov r12, [rdi + 0x58]
    mov r13, [rdi + 0x60]
    mov r14, [rdi + 0x68]
    mov r15, [rdi + 0x70]

    ; Restore RFLAGS
    mov r8, [rdi + 0x88]
    push r8
    popfq

    ; Restore RSP
    mov rsp, [rdi + 0x80]

    ; Load RIP and RDI, jump to RIP
    mov r8, [rdi + 0x78]   ; RIP to r8 (we need rdi for the last restore)
    mov rdi, [rdi + 0x28]  ; Restore RDI last
    jmp r8                 ; Jump to RIP (implicit return/context switch)
```

---

## 5. SYSCALL Entry/Exit Assembly

### SYSCALL Entry Handler

```asm
; Syscall entry point (address loaded in IA32_LSTAR)
; On entry: RCX = user RIP, R11 = user RFLAGS, privileged mode active
; Not yet in kernel context: RSP, GS.base still user values

extern syscall_handler  ; Zig function to dispatch syscall

align 16
syscall_entry:
    ; Step 1: Atomically switch to kernel GS (if using separate GS base)
    swapgs
    
    ; Step 2: Load kernel RSP from per-CPU area or TSS
    ; Method A: Use GS base pointing to per-CPU struct
    mov rsp, [gs:KERNEL_RSP_OFFSET]
    
    ; Method B: Load from TSS via tr
    ; mov rsp, [gs:TSS_POINTER]
    ; mov rsp, [rsp + 4]  ; RSP0 offset in TSS
    
    ; Step 3: Save user state on kernel stack
    push r11                ; User RFLAGS
    push rcx                ; User RIP
    push rax                ; Syscall number
    
    ; Step 4: Save remaining user-accessible registers
    push rdi
    push rsi
    push rdx
    push r10
    push r8
    push r9
    
    ; Step 5: Call C/Zig syscall handler
    ; rax still has syscall number
    call syscall_handler
    ; Return value in RAX, or RAX contains jump address for context switch
    
    ; Fall through to sysret_exit

align 16
sysret_exit:
    ; Restore user state from kernel stack
    pop r9
    pop r8
    pop r10
    pop rdx
    pop rsi
    pop rdi
    
    pop rax                 ; Syscall number (or ignore)
    pop rcx                 ; User RIP (restore for SYSRET)
    pop r11                 ; User RFLAGS (restore for SYSRET)
    
    ; Before SYSRET, kernel handler must:
    ; - Place user RIP in RCX
    ; - Place user RFLAGS in R11
    ; - Restore user RSP (either via [rsp] pop or explicitly)
    
    ; Step 6: Switch back to user GS
    swapgs
    
    ; Step 7: Return to userspace
    sysret                  ; RIP←RCX, RFLAGS←R11, CPL←3, SS/CS from STAR
```

### User → Kernel Transition (Full Context)

If syscall needs to save full context for preemption:

```asm
syscall_entry_full_save:
    swapgs
    mov rsp, [gs:KERNEL_RSP_OFFSET]
    
    ; Save everything as if interrupt occurred
    push r11
    push rcx
    ; ... save all GPRs in Context layout order ...
    
    ; Call context-aware handler
    lea rdi, [rsp]          ; RDI = saved context
    call syscall_with_preemption_handler
    
    ; Handler may choose to context switch
    ; If so, restore via restore_context
```

---

## 6. Round-Robin Scheduler Implementation

### Scheduler Data Structure

```zig
pub const TaskState = enum(u8) {
    Running,
    Blocked,
    Sleeping,
    Dead,
};

pub const Task = struct {
    id: u64,
    context: Context,
    state: TaskState,
    priority: u8,
    kernel_stack_base: [*]u8,
    kernel_stack_size: usize,
};

pub const Scheduler = struct {
    tasks: [MAX_TASKS]Task = undefined,
    task_count: usize = 0,
    current_task_index: usize = 0,
    
    pub fn add_task(self: *Scheduler, task: Task) void {
        if (self.task_count < MAX_TASKS) {
            self.tasks[self.task_count] = task;
            self.task_count += 1;
        }
    }
    
    pub fn schedule_next(self: *Scheduler) void {
        // Find next ready task (skip blocked/sleeping)
        var attempts: usize = 0;
        var next_index = (self.current_task_index + 1) % self.task_count;
        
        while (self.tasks[next_index].state != .Running and attempts < self.task_count) {
            next_index = (next_index + 1) % self.task_count;
            attempts += 1;
        }
        
        if (attempts < self.task_count) {
            self.current_task_index = next_index;
        }
    }
};
```

### Timer Interrupt Handler (triggers context switch)

```zig
pub fn timer_interrupt_handler(context: *Context) void {
    // Called from interrupt handler with current task context
    
    // Save current task context (context already populated by interrupt handler)
    scheduler.tasks[scheduler.current_task_index].context = context.*;
    
    // Select next task
    scheduler.schedule_next();
    
    // Load next task context
    context.* = scheduler.tasks[scheduler.current_task_index].context;
    
    // Return to restore_context, which jumps to next task
}
```

---

## 7. First Task Bootstrap (Jump to Userspace)

### Kernel Setup

```zig
pub fn kernel_init() noreturn {
    // ... initialize kernel structures ...
    
    // Create init task
    var init_task: Task = undefined;
    init_task.id = 1;
    init_task.state = .Running;
    
    // Allocate kernel stack for init task (used by syscall entry)
    init_task.kernel_stack_base = kernel_heap.alloc(u8, 4096) catch unreachable;
    init_task.kernel_stack_size = 4096;
    
    // Set up context for init task
    init_task.context.rip = @intFromPtr(&userspace_entry);
    init_task.context.rsp = USER_STACK_TOP;  // User stack in userspace
    init_task.context.rflags = 0x202;        // IF set, others cleared
    init_task.context.cs = 0x18 | 3;         // User code segment
    init_task.context.ss = 0x20 | 3;         // User stack segment
    init_task.context.id = 1;
    
    // Add to scheduler
    scheduler.add_task(init_task);
    
    // Jump to first task
    restore_context(&init_task.context);
    // Never returns; next interrupt will context switch
}
```

### Memory Layout

```
User Space (ring 3):
  [0x400000] ← user code entry point
  [0x500000] ← heap start
  [0x600000] ← stack top (grows down)
  [0x800000] ← end of user space

Kernel Space (ring 0):
  [0xFFFF800000000000] ← canonical kernel base
  [...] kernel code/data
  [0xFFFF800001000000] ← kernel heap
  [0xFFFF800002000000] ← per-CPU area / GS base
  [0xFFFF800003000000] ← TSS area
  [0xFFFF800004000000] ← stacks for each task (kernel stacks)
```

---

## 8. SWAPGS and GS.base Setup

### Kernel Initialization

```asm
; Initialize GS.base for kernel mode
init_gs_base:
    mov rax, KERNEL_GS_BASE_ADDR  ; Points to per-CPU struct / kernel data
    mov rcx, 0xC0000102           ; IA32_KERNEL_GS_BASE MSR
    xor rdx, rdx
    wrmsr                         ; Write to KERNEL_GS_BASE
    
    ; User GS base is typically 0 (or set by user)
    ; We keep user GS.base in actual GS, kernel in IA32_KERNEL_GS_BASE
```

### Per-CPU Structure (pointed to by kernel GS.base)

```zig
pub const PerCpu = struct {
    kernel_rsp: u64,           // For syscall entry
    current_task: *Task,       // Pointer to current task
    tss_pointer: *TSS,         // Task State Segment
    // ... other per-CPU data ...
};
```

### SWAPGS Usage

```asm
; In userspace:
; GS.base points to user TLS or 0

; On SYSCALL entry:
syscall_entry:
    swapgs              ; Exchange user GS with IA32_KERNEL_GS_BASE
    ; Now GS.base = KERNEL_GS_BASE_ADDR = &per_cpu_data
    ; Original user GS saved in IA32_KERNEL_GS_BASE

; On SYSRET exit:
sysret_exit:
    swapgs              ; Exchange back
    sysret
    ; Now GS.base = user value again
```

---

## 9. TSS and RSP0 Setup

### Task State Segment Structure (64-bit)

```asm
align 16
tss:
    dd 0                        ; Reserved (4 bytes)
    dq KERNEL_STACK_TOP         ; RSP0 (8 bytes) - kernel stack for ring 0
    dq 0                        ; RSP1 (8 bytes)
    dq 0                        ; RSP2 (8 bytes)
    dq 0                        ; Reserved (8 bytes)
    dq 0                        ; IST1 (Interrupt Stack Table entry)
    dq 0                        ; IST2
    dq 0                        ; IST3
    dq 0                        ; IST4
    dq 0                        ; IST5
    dq 0                        ; IST6
    dq 0                        ; IST7
    dq 0                        ; Reserved (8 bytes)
    dw 0                        ; Reserved (2 bytes)
    dw tss_end - tss - 1        ; I/O Map Base Offset (2 bytes)
tss_end:

tss_size equ tss_end - tss
```

### GDT Entry for TSS

```asm
; GDT[TSS_INDEX]:
; Descriptor: { base: &tss, limit: tss_size - 1, type: 0x9 (64-bit available TSS) }
;
; Type 0x9 = 1001b = Available TSS (64-bit)
; P=1, DPL=0, type=0x9 → descriptor format for TSS

align 8
gdt:
    ; Selector 0x00: Null
    dq 0
    
    ; Selector 0x08: Kernel code (64-bit)
    dq 0x00209A0000000000
    
    ; Selector 0x10: Kernel data
    dq 0x0020920000000000
    
    ; Selector 0x18: User code (64-bit, DPL=3)
    dq 0x0020FA0000000000
    
    ; Selector 0x20: User data (DPL=3)
    dq 0x0020F20000000000
    
    ; Selector 0x28: TSS (64-bit, two entries required for 16-byte descriptor)
    ; This is filled in by code: set base=&tss, limit=tss_size-1, type=0x9, P=1, DPL=0

gdt_end:
```

### Loading TSS

```asm
; After setting up GDT with TSS descriptor:
ltr [0x28]          ; Load TR with TSS selector (0x28)
```

---

## Summary: Context Switch Flow

### Timer Interrupt → Next Task

1. **Hardware (timer)** → interrupt fires
2. **CPU hardware** → saves SS, RSP, RFLAGS, CS, RIP on stack (implicitly)
3. **Interrupt handler (asm)** → pushes remaining regs, calls `save_context()`
4. **save_context()** → serializes all regs to Task.context struct
5. **Kernel scheduler** → selects next ready task
6. **restore_context(next_context)** → pops regs from struct, jumps to RIP
7. **Next task resumes** from where it left off

### SYSCALL → System Call

1. **User code** → `syscall` instruction in user mode
2. **Hardware (CPU)** → saves RCX=user_rip, R11=user_rflags, jumps to IA32_LSTAR
3. **Syscall handler (asm)** → SWAPGS, loads kernel RSP, saves context
4. **Syscall dispatcher (Zig)** → handles syscall, may preempt
5. **If no preemption** → return via sysret, restore RCX/R11, SWAPGS back
6. **Hardware (CPU)** → sysret restores RIP←RCX, RFLAGS←R11, returns to user

---

## Key Constants for VantaOS Phase 1

```zig
pub const GDT_KERNEL_CS = 0x08;
pub const GDT_KERNEL_SS = 0x10;
pub const GDT_USER_CS = 0x18 | 3;      // +3 for ring 3
pub const GDT_USER_SS = 0x20 | 3;
pub const GDT_TSS_SEL = 0x28;

pub const MSR_STAR = 0xC0000081;
pub const MSR_LSTAR = 0xC0000082;
pub const MSR_FMASK = 0xC0000084;
pub const MSR_EFER = 0xC0000080;
pub const MSR_KERNEL_GS_BASE = 0xC0000102;

pub const EFER_SCE = 1;    // Bit 0: SYSCALL enable

pub const RFLAGS_IF = 0x200;   // Interrupt enable
pub const RFLAGS_TF = 0x100;   // Trap flag
pub const RFLAGS_DF = 0x400;   // Direction flag
pub const RFLAGS_FMASK = 0x700; // Typical mask for SYSCALL

pub const USER_SPACE_BASE = 0x400000;
pub const USER_STACK_TOP = 0x800000;
pub const KERNEL_SPACE_BASE = 0xFFFF800000000000;
```

---

## References

- Intel 64 and IA-32 Architectures Software Developer's Manual, Vol. 2B: SYSCALL, SYSRET
- x86-64 Application Binary Interface (System V AMD64 ABI)
- OSDev Wiki: Context Switching, SYSENTER and SYSCALL
