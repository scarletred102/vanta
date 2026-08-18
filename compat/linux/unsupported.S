.section .text
.global _start
_start:
    mov $9999, %rax
    syscall
    mov $60, %rax
    mov $42, %rdi
    syscall
