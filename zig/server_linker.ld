/*
 * VantaOS Server Linker Script
 * Userspace servers at 0x400000, page-aligned segments
 */

OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS((1 << 0) | (1 << 2));  /* R X */
    rodata PT_LOAD FLAGS((1 << 2));              /* R   */
    data   PT_LOAD FLAGS((1 << 1) | (1 << 2));  /* RW  */
}

SECTIONS
{
    . = 0x400000;

    .text : ALIGN(4096) {
        *(.text .text.*)
    } :text

    . = ALIGN(4096);

    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(4096);

    .data : {
        *(.data .data.*)
    } :data

    .bss : {
        *(COMMON)
        *(.bss .bss.*)
    } :data

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
