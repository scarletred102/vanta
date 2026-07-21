ENTRY(_start)
OUTPUT_FORMAT(elf64-x86-64)

PHDRS {
    text PT_LOAD FLAGS(5);
    data PT_LOAD FLAGS(6);
}

SECTIONS {
    . = 0x400000;
    .text : { *(.text .text.*) } :text
    . = ALIGN(0x1000);
    .rodata : { *(.rodata .rodata.*) } :text
    . = ALIGN(0x1000);
    .data : {
        *(.data .data.*)
        *(.got .got.*)
        *(COMMON)
        *(.bss .bss.*)
    } :data
    /DISCARD/ : { *(.eh_frame*) *(.note*) *(.comment) }
}
