#include <stdint.h>

#define AT_NULL   0
#define AT_PHDR   3
#define AT_PHENT  4
#define AT_PHNUM  5
#define AT_PAGESZ 6
#define AT_BASE   7
#define AT_FLAGS  8
#define AT_ENTRY  9
#define AT_RANDOM 25

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "mov %rsp, %rdi\n"
        "call _dl_entry\n"
        "mov %rax, %r11\n"
        "xor %rdx, %rdx\n"
        "jmp *%r11\n"
    );
}

uintptr_t _dl_entry(uintptr_t *sp) {
    uintptr_t argc = *sp++;
    sp += argc + 1; // skip argv and NULL
    while (*sp) sp++; // skip envp
    sp++; // skip NULL terminating envp

    uintptr_t entry = 0;
    while (*sp != AT_NULL) {
        uintptr_t key = *sp++;
        uintptr_t val = *sp++;
        if (key == AT_ENTRY) {
            entry = val;
        }
    }
    return entry;
}
