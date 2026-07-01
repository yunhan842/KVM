//! Ring-3 program loading + iretq entry.
//!
//! The ring-3 program is now a freestanding C program (user/hello.c +
//! user/crt0.s linked via user/link.ld) compiled to an ELF and embedded in
//! the kernel via `include_bytes!`. `setup()` maps the user region, parses
//! the embedded ELF, copies its PT_LOAD segments into place, and returns the
//! entry point. `enter_ring3` takes that entry point as a runtime argument
//! and dispatches to ring 3 via `iretq`.
//!
//! Design: docs/superpowers/specs/2026-05-31-minikvm-month3-c-userspace-design.md
//! (slice-4b's asm-blob predecessor:
//!  docs/superpowers/specs/2026-05-28-minikvm-month2-syscall-ring3-design.md)

use core::arch::asm;

use crate::{elf, gdt, paging};

// PD[4] = 0x800000..0xA00000 (8..10 MiB). The ELF links at 0x800000; stack top
// strictly inside the page (0x9FFFF0, NOT 0xA00000 — that lives in PD[5]).
const USER_PD_INDEX:  usize = 4;
const USER_STACK_TOP: u64   = 0x009F_FFF0;

// The embedded user ELF. include_bytes! tracks this file in cargo's dep-info,
// so changes to user/hello.c (rebuilt as build/hello.elf by the Makefile)
// automatically force a kernel rebuild.
//
// Path: CARGO_MANIFEST_DIR = crates/kernel/ ; "../.." = workspace root which
// contains build/. Two ../, not three.
static USER_ELF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../build/hello.elf"
));

/// Map the user region and load the embedded ELF into it. Returns the ELF
/// entry point — which `enter_ring3` consumes.
///
/// # Safety
/// Call once, after `paging::init_identity_map`, before `enter_ring3`. The
/// embedded ELF must satisfy `elf::load`'s preconditions (well-formed ELF64,
/// PT_LOADs targeting the mapped user region).
pub unsafe fn setup() -> u64 {
    paging::map_user_pd_entry(USER_PD_INDEX);
    elf::load(USER_ELF).expect("loading /bin/hello failed")
}

/// Transition to ring 3 via `iretq`, starting execution at `entry`. Never returns.
///
/// Builds the 5-element interrupt-return frame on the kernel stack (in
/// high-to-low push order so the lowest address — top of stack — holds RIP,
/// which `iretq` consumes first).
///
/// # Safety
/// Call once. `setup()` must have run; `entry` must be a valid ring-3
/// instruction address inside the mapped user region.
pub unsafe fn enter_ring3(entry: u64) -> ! {
    asm!(
        "push {ss}",
        "push {rsp}",
        "push {rflags}",
        "push {cs}",
        "push {entry}",
        "iretq",
        // ss/rsp/rflags/cs stay as i32 consts (every value fits signed 32-bit).
        // `entry` is a runtime u64 — passed in a register; `push <reg>` pushes
        // the full 64-bit value.
        ss     = const ((gdt::USER_SS  | 3) as i32),
        rsp    = const (USER_STACK_TOP as i32),     // strictly inside PD[4]
        rflags = const 0x202_i32,                   // IF=1 (bit 9) + reserved bit 1: ring 3 is preemptible
        cs     = const ((gdt::USER_CS64 | 3) as i32),
        entry  = in(reg) entry,
        options(noreturn),
    );
}
