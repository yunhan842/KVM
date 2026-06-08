ENTRY(_start)

/* Invariant: .text must be the FIRST allocatable section, starting at
 * load address 0x2000 with no preceding padding. The Makefile uses
 * `objcopy -O binary` to flatten this ELF into guest.img — byte N of
 * the resulting blob corresponds to GPA 0x2000+N. gdb's "file kernel"
 * loads symbols from this ELF; "b symbol_name" places 0xCC at the
 * symbol's link address, which must equal its runtime GPA. */
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
