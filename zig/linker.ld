/*
 * VantaOS Kernel Linker Script
 * Higher-half kernel at 0xFFFFFFFF80000000
 * Compatible with Limine boot protocol
 */

OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

PHDRS
{
    requests PT_LOAD    FLAGS((1 << 2));                    /* R   */
    text     PT_LOAD    FLAGS((1 << 0) | (1 << 2));        /* R X */
    rodata   PT_LOAD    FLAGS((1 << 2));                    /* R   */
    data     PT_LOAD    FLAGS((1 << 1) | (1 << 2));        /* RW  */
}

SECTIONS
{
    /* Limine loads us at the higher half */
    . = 0xFFFFFFFF80000000;

    /* Limine request sections — bootloader scans these for magic IDs */
    .limine_requests_start : ALIGN(8) {
        KEEP(*(.limine_requests_start))
    } :requests

    .limine_requests : ALIGN(8) {
        KEEP(*(.limine_requests))
    } :requests

    .limine_requests_end : ALIGN(8) {
        KEEP(*(.limine_requests_end))
    } :requests

    /* Executable code */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .text : {
        *(.text .text.*)
    } :text

    /* Read-only data */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    /* Read-write data */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    .data : {
        *(.data .data.*)
    } :data

    /* Uninitialized data (zeroed by loader) */
    .bss : {
        *(COMMON)
        *(.bss .bss.*)
    } :data

    /* Symbols for kernel to know its own extent */
    _kernel_end = .;

    /* Discard unneeded sections */
    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
