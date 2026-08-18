.section .rodata
path:
    .asciz "/etc/config"
.section .bss
.lcomm buffer, 256
.section .text
.global _start
_start:
    mov $257, %rax
    mov $-100, %rdi
    lea path(%rip), %rsi
    xor %rdx, %rdx
    xor %r10, %r10
    syscall
    test %rax, %rax
    js exit_error
    mov %rax, %r12
    mov $0, %rax
    mov %r12, %rdi
    lea buffer(%rip), %rsi
    mov $256, %rdx
    syscall
    test %rax, %rax
    js exit_error
    mov %rax, %rdx
    mov $1, %rax
    mov $1, %rdi
    lea buffer(%rip), %rsi
    syscall
    mov $3, %rax
    mov %r12, %rdi
    syscall
    xor %rdi, %rdi
    mov $60, %rax
    syscall
exit_error:
    mov $60, %rax
    mov $1, %rdi
    syscall
