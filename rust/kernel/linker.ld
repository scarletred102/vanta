ENTRY(_start)
OUTPUT_FORMAT(elf64-x86-64)

PHDRS {
    requests  PT_LOAD;
    text      PT_LOAD;
    rodata    PT_LOAD;
    data      PT_LOAD;
}

SECTIONS {
    . = 0xffffffff80000000;

    .requests : {
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))
    } :requests

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .text : { *(.text .text.*) } :text

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .rodata : { *(.rodata .rodata.*) } :rodata

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    .data : {
        *(.data .data.*)
        *(.got)
    } :data

    .bss : {
        __bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        . = ALIGN(8);
        __bss_end = .;
    } :data

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note*)
        *(.comment)
    }
}
