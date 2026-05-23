ENTRY(_start)

SECTIONS {
    . = 0x2000;

    .text : {
        *(.text.boot)        /* _start goes here, first, so it lands exactly at 0x2000 */
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) }
    .data   : { *(.data .data.*) }
    .bss    : { *(.bss .bss.*) *(COMMON) }

    /* Drop unwind tables (panic=abort), build notes, and toolchain comments so no
       allocatable orphan section sorts ahead of .text and displaces our 0x2000 entry. */
    /DISCARD/ : {
        *(.eh_frame) *(.eh_frame_hdr)
        *(.note .note.*)
        *(.comment)
    }
}
