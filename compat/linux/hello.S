.section .rodata
message:
    .ascii "[linux] hello\n"
message_end:

.section .text
.global _start
_start:
    mov $1, %rax
    mov $1, %rdi
    lea message(%rip), %rsi
    mov $(message_end-message), %rdx
    syscall
    mov $60, %rax
    xor %rdi, %rdi
    syscall
