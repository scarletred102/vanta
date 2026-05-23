// Minimal Zig userspace binary — no libc
pub const std = @import("std");

pub export fn _start() callconv(.naked) noreturn {
    asm volatile (
        \\// 1. Read from COW page (0x500000)
        \\movabsq $0x500000, %%rcx
        \\movq (%%rcx), %%rax
        \\
        \\// 2. Write to COW page (0x500000) -> triggers COW fault
        \\movabsq $0x99999999, %%rax
        \\movq %%rax, (%%rcx)
        \\
        \\// 3. Write to Lazy VMA page (0x600000) -> triggers Lazy VMA fault
        \\movabsq $0x600000, %%rcx
        \\movabsq $0x77777777, %%rax
        \\movq %%rax, (%%rcx)
        \\
        \\// 4. DebugPrint (RAX = 5)
        \\mov $5, %%rax
        \\mov $msg, %%rdi
        \\mov $37, %%rsi            // msg length
        \\syscall
        \\
        \\// 5. Exit (RAX = 6)
        \\mov $6, %%rax
        \\xor %%rdi, %%rdi          // exit code 0
        \\syscall
        \\
        \\.data
        \\msg: .asciz "Hello from VantaOS Ring 3 userspace!\n"
    );
    while (true) {}
}
