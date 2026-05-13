// VantaOS Phase 1: Context Switching and Scheduler
// x86_64 implementation in Zig (freestanding)

const std = @import("std");

/// Context structure: mirrors x86_64 register layout for task switching
/// Total size: 0xC8 (200 bytes), 16-byte aligned
pub const Context = struct {
    // General purpose registers (128 bytes)
    rax: u64 = 0,   // 0x00
    rbx: u64 = 0,   // 0x08
    rcx: u64 = 0,   // 0x10
    rdx: u64 = 0,   // 0x18
    rsi: u64 = 0,   // 0x20
    rdi: u64 = 0,   // 0x28
    rbp: u64 = 0,   // 0x30
    r8: u64 = 0,    // 0x38
    r9: u64 = 0,    // 0x40
    r10: u64 = 0,   // 0x48
    r11: u64 = 0,   // 0x50
    r12: u64 = 0,   // 0x58
    r13: u64 = 0,   // 0x60
    r14: u64 = 0,   // 0x68
    r15: u64 = 0,   // 0x70

    // Control flow (24 bytes)
    rip: u64 = 0,   // 0x78 - instruction pointer
    rsp: u64 = 0,   // 0x80 - stack pointer
    rflags: u64 = 0,// 0x88 - processor flags

    // Segment registers (12 bytes, padded to 16)
    cs: u16 = 0,    // 0x90 - code segment
    ss: u16 = 0,    // 0x92 - stack segment
    ds: u16 = 0,    // 0x94 - data segment
    es: u16 = 0,    // 0x96 - extra segment
    fs: u16 = 0,    // 0x98 - FS segment
    gs: u16 = 0,    // 0x9A - GS segment
    _pad: u16 = 0,  // 0x9C - padding

    // Extended state (16 bytes)
    fs_base: u64 = 0,  // 0xA0 - FS base address
    gs_base: u64 = 0,  // 0xA8 - GS base address

    // Task metadata (8+ bytes)
    id: u64 = 0,       // 0xB0 - task ID
    kernel_rsp: u64 = 0, // 0xB8 - kernel stack pointer for syscall entry

    pub fn default_user_flags() u64 {
        return 0x202; // IF set, others cleared
    }
};

pub const TaskState = enum(u8) {
    Running = 0,
    Blocked = 1,
    Sleeping = 2,
    Dead = 3,
};

pub const Task = struct {
    id: u64,
    context: Context,
    state: TaskState,
    priority: u8,
    kernel_stack_base: [*]u8,
    kernel_stack_size: usize,

    pub fn init(id: u64, context: Context, kernel_stack: [*]u8, kernel_stack_size: usize) Task {
        return Task{
            .id = id,
            .context = context,
            .state = .Running,
            .priority = 0,
            .kernel_stack_base = kernel_stack,
            .kernel_stack_size = kernel_stack_size,
        };
    }
};

pub const MAX_TASKS = 64;

pub const Scheduler = struct {
    tasks: [MAX_TASKS]Task = undefined,
    task_count: usize = 0,
    current_task_index: usize = 0,
    ticks: u64 = 0,

    pub fn add_task(self: *Scheduler, task: Task) !void {
        if (self.task_count >= MAX_TASKS) return error.TooManyTasks;
        self.tasks[self.task_count] = task;
        self.task_count += 1;
    }

    pub fn schedule_next(self: *Scheduler) void {
        if (self.task_count == 0) return;

        // Find next ready task in round-robin fashion
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

    pub fn current_task(self: *const Scheduler) *const Task {
        return &self.tasks[self.current_task_index];
    }

    pub fn current_task_mut(self: *Scheduler) *Task {
        return &self.tasks[self.current_task_index];
    }
};

// MSR constants for SYSCALL/SYSRET
pub const MSR_STAR = 0xC0000081;       // Segment selectors
pub const MSR_LSTAR = 0xC0000082;      // Syscall handler RIP
pub const MSR_FMASK = 0xC0000084;      // RFLAGS mask
pub const MSR_EFER = 0xC0000080;       // Extended Feature Enable Register
pub const MSR_KERNEL_GS_BASE = 0xC0000102; // Kernel GS base (for SWAPGS)

pub const EFER_SCE = 1;                // Bit 0: SYSCALL/SYSRET enable

// GDT selectors
pub const GDT_KERNEL_CS = 0x08;
pub const GDT_KERNEL_SS = 0x10;
pub const GDT_USER_CS = 0x18 | 3;      // Ring 3
pub const GDT_USER_SS = 0x20 | 3;      // Ring 3
pub const GDT_TSS_SEL = 0x28;

// RFLAGS constants
pub const RFLAGS_IF = 0x200;           // Interrupt enable
pub const RFLAGS_TF = 0x100;           // Trap flag
pub const RFLAGS_DF = 0x400;           // Direction flag
pub const RFLAGS_FMASK = 0x700;        // Typical SYSCALL mask (IF|TF|DF)
pub const RFLAGS_DEFAULT = 0x202;      // IF set

// Memory layout constants
pub const USER_SPACE_BASE = 0x400000;
pub const USER_STACK_TOP = 0x800000;
pub const KERNEL_SPACE_BASE = 0xFFFF800000000000;

// ============================================================================
// Assembly function declarations (implemented in context_switch.s or inline)
// ============================================================================

/// Save current context to Context struct
/// Called from interrupt handler with RDI = &context
extern fn save_context(context: *Context) void;

/// Restore context from struct and jump to saved RIP
/// Does not return; continues execution at saved RIP
extern fn restore_context(context: *Context) noreturn;

/// Setup SYSCALL/SYSRET MSRs for kernel
extern fn setup_syscall_msrs(lstar: u64, star: u64, fmask: u64) void;

/// Load TSS into TR (Task Register)
extern fn load_tss(tss_selector: u16) void;

// ============================================================================
// Scheduler initialization and management
// ============================================================================

var global_scheduler: Scheduler = undefined;

pub fn init_scheduler() void {
    global_scheduler = Scheduler{};
}

pub fn add_task_to_scheduler(task: Task) !void {
    try global_scheduler.add_task(task);
}

pub fn current_task() *const Task {
    return global_scheduler.current_task();
}

pub fn switch_to_next_task(current_context: *Context) void {
    // Save current task's context
    global_scheduler.current_task_mut().context = current_context.*;

    // Schedule next task
    global_scheduler.schedule_next();

    // Load next task's context and jump
    const next_context = &global_scheduler.current_task_mut().context;
    current_context.* = next_context.*;

    // Note: restore_context is marked noreturn, so this function doesn't actually return
    // In practice, this would be called from interrupt handler which will then call restore_context
}

// ============================================================================
// SYSCALL setup and handling
// ============================================================================

/// Initialize SYSCALL/SYSRET mechanism
pub fn init_syscall_mechanism(syscall_handler_rip: u64) void {
    // IA32_STAR: bits [47:32] = kernel CS, bits [63:48] = user CS + 16 (user SS)
    // kernel_cs = 0x08, user_cs = 0x18
    const star: u64 = (0x18 << 48) | (0x08 << 32);

    // IA32_LSTAR: kernel syscall handler address
    const lstar: u64 = syscall_handler_rip;

    // IA32_FMASK: mask for RFLAGS on SYSCALL (clear IF, TF, DF)
    const fmask: u64 = RFLAGS_FMASK;

    // Call assembly to set up MSRs
    setup_syscall_msrs(lstar, star, fmask);
}

/// C-callable syscall dispatcher (called from syscall handler asm)
pub export fn syscall_dispatcher(syscall_num: u64) u64 {
    // Dispatch based on syscall number
    // Return value goes to RAX for user code
    switch (syscall_num) {
        0 => return syscall_read(),
        1 => return syscall_write(),
        2 => return syscall_open(),
        3 => return syscall_close(),
        // ... more syscalls ...
        else => return @as(u64, 0) -% 1, // -1 for unknown syscall
    }
}

fn syscall_read() u64 {
    return 0; // Placeholder
}

fn syscall_write() u64 {
    return 0; // Placeholder
}

fn syscall_open() u64 {
    return 0; // Placeholder
}

fn syscall_close() u64 {
    return 0; // Placeholder
}

// ============================================================================
// Task creation helpers
// ============================================================================

pub fn create_user_task(id: u64, entry_point: u64, user_rsp: u64, kernel_stack: [*]u8, kernel_stack_size: usize) Task {
    var ctx = Context{
        .id = id,
        .rip = entry_point,
        .rsp = user_rsp,
        .rflags = RFLAGS_DEFAULT,
        .cs = GDT_USER_CS,
        .ss = GDT_USER_SS,
        .kernel_rsp = @intFromPtr(kernel_stack) + kernel_stack_size, // Top of kernel stack
    };

    return Task.init(id, ctx, kernel_stack, kernel_stack_size);
}

pub fn create_kernel_task(id: u64, entry_point: u64, kernel_rsp: u64) Task {
    var ctx = Context{
        .id = id,
        .rip = entry_point,
        .rsp = kernel_rsp,
        .rflags = RFLAGS_DEFAULT,
        .cs = GDT_KERNEL_CS,
        .ss = GDT_KERNEL_SS,
    };

    return Task.init(id, ctx, undefined, 0);
}

// ============================================================================
// Test/debug helpers
// ============================================================================

pub fn print_context(ctx: *const Context) void {
    // In real kernel, use kernel logging
    _ = ctx; // unused
    // std.debug.print("Context dump:\n", .{});
    // std.debug.print("  RAX={X:0>16} RBX={X:0>16}\n", .{ctx.rax, ctx.rbx});
    // ... etc
}

pub fn print_task(task: *const Task) void {
    _ = task; // unused
    // std.debug.print("Task {}: state={}\n", .{task.id, task.state});
    // print_context(&task.context);
}
