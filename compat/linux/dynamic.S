.section .interp,"a"
    .asciz "/lib64/ld-linux-x86-64.so.2"
.section .text
.global _start
_start:
    mov $60, %rax
    xor %rdi, %rdi
    syscall
