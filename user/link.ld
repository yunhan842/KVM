/* user/link.ld — linker script for the ring-3 hello program.
 *
 * Layout: load at 0x800000 (PD[4] base of the kernel's user region). Two
 * explicit PT_LOAD segments — text+rodata (R+X) and data+bss (R+W). The
 * kernel-side loader (crates/kernel/src/elf.rs) walks both.
 *
 * FLAGS(5) = R + X (4 + 1); FLAGS(6) = R + W (4 + 2). These bits are real
 * ELF metadata; slice 5 doesn't enforce them at the page-table level (the
 * whole user region is RWX), but they're pedagogically honest and a future
 * slice adding per-segment perms would consume p_flags.
 *
 * The DISCARD block drops sections that default-gcc/ld emit but that we
 * don't need: unwind tables (we don't use C++ exceptions), GNU notes,
 * compiler comments, GNU-specific cruft.
 */

ENTRY(_start)

PHDRS {
    text PT_LOAD FLAGS(5);   /* R + X */
    data PT_LOAD FLAGS(6);   /* R + W */
}

SECTIONS {
    . = 0x800000;

    .text   : { *(.text*) }   :text
    .rodata : { *(.rodata*) } :text

    .data   : { *(.data*) }   :data
    .bss    : { *(.bss*) *(COMMON) } :data

    /DISCARD/ : {
        *(.eh_frame*) *(.note*) *(.comment) *(.gnu*)
    }
}
