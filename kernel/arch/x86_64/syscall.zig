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
const std = @import("std");
const serial = @import("serial.zig");
const dispatch_mod = @import("../../syscall/table.zig");
const pmm = @import("../../mm/pmm.zig");
const vmm = @import("../../mm/vmm.zig");
const syscall_table = @import("../../syscall/syscall_table.zig");

const EFER_MSR:  u32 = 0xC0000080;
const STAR_MSR:  u32 = 0xC0000081;
const LSTAR_MSR: u32 = 0xC0000082;
const FMASK_MSR: u32 = 0xC0000084;
const GS_BASE_MSR: u32 = 0xC0000101;
const KERNEL_GS_BASE_MSR: u32 = 0xC0000102;

pub var test_kernel_stack: [4096]u8 align(16) = undefined;
pub var test_gs_data: [1]u64 = undefined;

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
pub fn syscall_init() void {
    // 1. Enable SCE bit in EFER
    const efer = rdmsr(EFER_MSR);
    wrmsr(EFER_MSR, efer | 1);

    // 2. STAR: ring 0 CS in bits 47:32, SYSRET CS in bits 63:48
    // We set the user base selector to USER_CODE32_SEL | 3 = 0x1B so user CS = 0x2B, SS = 0x23
    const star: u64 = (@as(u64, gdt.USER_CODE32_SEL | 3) << 48) | (@as(u64, gdt.KERNEL_CODE_SEL) << 32);
    wrmsr(STAR_MSR, star);

    // 3. LSTAR: 64-bit syscall entry RIP
    wrmsr(LSTAR_MSR, @intFromPtr(&syscall_entry));

    // 4. FMASK: clear RFLAGS.IF (0x200) and DF (0x400) on entry
    wrmsr(FMASK_MSR, 0x200 | 0x400);

    // Set up GS base pointers for safe kernel stack retrieval
    test_gs_data[0] = @intFromPtr(&test_kernel_stack) + 4096;
    wrmsr(GS_BASE_MSR, @intFromPtr(&test_gs_data));
    wrmsr(KERNEL_GS_BASE_MSR, @intFromPtr(&test_gs_data));

    serial.puts("[SC]    SYSCALL/SYSRET MSRs configured via syscall_init()\n");
}

pub fn init() void {
    syscall_init();
}

pub fn verifySyscallFromRing3() void {
    serial.puts("[TEST]  Starting Ring 3 Extended Verification...\n");

    // 1. Create a fresh address space
    const space_phys = vmm.create_user_address_space() orelse {
        serial.puts("[TEST]  FAILED: Cannot create address space\n");
        return;
    };
    const space = vmm.AddressSpace{ .pml4_phys = space_phys };

    // 2. Map the flat binary code at 0x400000
    const code_paddr = pmm.allocPage() orelse {
        serial.puts("[TEST]  FAILED: Cannot allocate code page\n");
        return;
    };
    
    // Copy the embedded smoke flat binary into the code page
    const code_virt = vmm.phys2virt(code_paddr);
    const dest_ptr = @as([*]u8, @ptrFromInt(code_virt));
    const smoke_bin = @embedFile("../../smoke");
    @memcpy(dest_ptr[0..smoke_bin.len], smoke_bin);

    if (!vmm.map(space, 0x400000, code_paddr, vmm.PTE_USER | vmm.PTE_WRITE)) {
        serial.puts("[TEST]  FAILED: Cannot map code page\n");
        return;
    }
    serial.puts("[TEST]  Flat binary mapped at 0x400000 (");
    serial.putDec(smoke_bin.len);
    serial.puts(" bytes)\n");

    // 3. Set up COW test page at 0x500000
    const cow_paddr = pmm.allocPage() orelse {
        serial.puts("[TEST]  FAILED: Cannot allocate COW page\n");
        return;
    };
    
    // Write initial test value (0x88888888) to COW page
    const cow_virt = vmm.phys2virt(cow_paddr);
    @as(*volatile u64, @ptrFromInt(cow_virt)).* = 0x88888888;
    
    // Map as COW (read-only and PTE_COW) in the new address space
    if (!vmm.map(space, 0x500000, cow_paddr, vmm.PTE_USER | vmm.PTE_COW)) {
        serial.puts("[TEST]  FAILED: Cannot map COW page\n");
        return;
    }
    
    // Set reference count to 2 to simulate page sharing
    const cow_page_idx = cow_paddr / pmm.PAGE_SIZE;
    pmm.page_refcounts[cow_page_idx] = 2;
    serial.puts("[TEST]  COW page mapped at 0x500000 (Initial Val=0x");
    serial.putHex(0x88888888);
    serial.puts(", RefCount=2)\n");

    // 4. Register a Lazy VMA for the range [0x600000, 0x601000)
    const current_proc = dispatch_mod.getCurrentProcess();
    // Clear existing VMAs first to make it a clean state
    current_proc.vma_count = 0;
    // Register the VMA as lazy and writable
    if (!current_proc.addVma(0x600000, 0x601000, vmm.PTE_WRITE, true)) {
        serial.puts("[TEST]  FAILED: Cannot add VMA\n");
        return;
    }
    serial.puts("[TEST]  Lazy VMA registered for [0x600000, 0x601000)\n");

    // 5. Activate the fresh address space!
    space.activate();
    serial.puts("[TEST]  Activated fresh user address space (CR3=0x");
    serial.putHex(space.pml4_phys);
    serial.puts(")\n");

    // 6. Allocate userspace stack in the new address space
    const user_rsp = vmm.alloc_user_stack(4) orelse {
        serial.puts("[TEST]  FAILED: Cannot allocate userspace stack\n");
        return;
    };
    
    // ABI Validation: Assert 16-byte stack pointer alignment
    std.debug.assert(user_rsp % 16 == 0);
    serial.puts("[TEST]  Verified: RSP is 16-byte aligned before enter_userspace (RSP=0x");
    serial.putHex(user_rsp);
    serial.puts(")\n");

    // 7. Transition to Ring 3!
    const user_rip: u64 = 0x400000;
    asm volatile ("swapgs" ::: .{ .memory = true });
    enter_userspace(user_rip, user_rsp, 0);
}

pub fn enter_userspace(entry: u64, stack: u64, arg0: u64) noreturn {
    asm volatile (
        \\cli
        \\
        \\// Load user data segment selector (0x23 = 0x20 | 3) into DS, ES, FS, GS
        \\mov $0x23, %%ax
        \\mov %%ax, %%ds
        \\mov %%ax, %%es
        \\mov %%ax, %%fs
        \\mov %%ax, %%gs
        \\
        \\// Construct fake interrupt frame on the stack:
        \\// Order: SS, RSP, RFLAGS, CS, RIP (high to low address)
        \\pushq $0x23             // SS (USER_DATA_SEL | 3)
        \\pushq %%rsi             // RSP (from input rsi constraint)
        \\pushq $0x202            // RFLAGS (IF=1)
        \\pushq $0x2B             // CS (USER_CODE_SEL | 3)
        \\pushq %%rdi             // RIP (from input rdi constraint)
        \\
        \\// Load RDI with arg0 for the user process
        \\mov %%rdx, %%rdi        // (from input rdx constraint)
        \\
        \\// Zero out all other general-purpose registers to prevent kernel information leaks
        \\xor %%rax, %%rax
        \\xor %%rbx, %%rbx
        \\xor %%rcx, %%rcx
        \\xor %%rdx, %%rdx
        \\xor %%rsi, %%rsi
        \\xor %%rbp, %%rbp
        \\xor %%r8, %%r8
        \\xor %%r9, %%r9
        \\xor %%r10, %%r10
        \\xor %%r11, %%r11
        \\xor %%r12, %%r12
        \\xor %%r13, %%r13
        \\xor %%r14, %%r14
        \\xor %%r15, %%r15
        \\
        \\// Execute IRETQ to transition to Ring 3 at entry
        \\iretq
        :
        : [entry] "{rdi}" (entry),
          [stack] "{rsi}" (stack),
          [arg0] "{rdx}" (arg0)
        : .{ .memory = true }
    );
    unreachable;
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
        \\.data
        \\.align 8
        \\user_rsp_temp: .quad 0
        \\user_rcx_temp: .quad 0
        \\user_r11_temp: .quad 0
        \\
        \\.text
        \\.global syscall_entry
        \\.type syscall_entry, @function
        \\syscall_entry:
        \\    swapgs                 // swap GS to kernel mode
        \\
        \\    // Save user RSP, RCX, R11 using RIP-relative addressing without touching any GP registers
        \\    mov %rsp, user_rsp_temp(%rip)
        \\    mov %rcx, user_rcx_temp(%rip)
        \\    mov %r11, user_r11_temp(%rip)
        \\
        \\    // Swap RSP to the per-thread kernel stack stored in TSS.RSP0
        \\    mov tss+4(%rip), %rsp
        \\
        \\    // Save all caller-saved registers & user context to the kernel stack
        \\    pushq user_rsp_temp(%rip)
        \\    pushq user_r11_temp(%rip)
        \\    pushq user_rcx_temp(%rip)
        \\    push %rax
        \\    push %rdx
        \\    push %rsi
        \\    push %rdi
        \\    push %r8
        \\    push %r9
        \\    push %r10
        \\
        \\    // Route arguments to C calling convention for syscallDispatchC
        \\    mov %rax, %rdi         // C arg 0 (syscall number)
        \\    mov 24(%rsp), %rsi     // C arg 1 (user rdi)
        \\    mov 32(%rsp), %rdx     // C arg 2 (user rsi)
        \\    mov 40(%rsp), %rcx     // C arg 3 (user rdx)
        \\    mov 0(%rsp), %r8       // C arg 4 (user r10)
        \\    mov 16(%rsp), %r9      // C arg 5 (user r8)
        \\    sub $8, %rsp           // 8-byte padding to align RSP to 16 bytes
        \\    pushq 16(%rsp)         // C arg 6 (user r9) - pushed onto stack (adjusted offset)
        \\
        \\    // Call the Zig dispatcher
        \\    call syscallDispatchC
        \\    add $16, %rsp          // clean up the 7th argument AND the 8-byte padding!
        \\
        \\    // Restore registers
        \\    pop %r10
        \\    pop %r9
        \\    pop %r8
        \\    pop %rdi
        \\    pop %rsi
        \\    pop %rdx
        \\    add $8, %rsp           // skip saved rax (return value in rax)
        \\    pop %rcx               // user rip
        \\    pop %r11               // user rflags
        \\    pop %rsp               // user rsp
        \\
        \\    swapgs                 // swap GS back to user mode
        \\    sysretq
    );
}

/// C-ABI dispatch shim called from syscall_entry asm.
/// rdi=number, rsi=a1, rdx=a2, rcx=a3, r8=a4, r9=a5
export fn syscallDispatchC(
    number: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64,
) callconv(.c) u64 {
    if (number > 10) {
        // ENOSYS is 38, -ENOSYS = -38
        return @bitCast(@as(i64, -38));
    }
    const r = syscall_table.dispatch(number, a1, a2, a3, a4, a5, 0);
    if (r.err != .success) {
        return 0xFFFFFFFF00000000 | @intFromEnum(r.err);
    }
    return r.value;
}
