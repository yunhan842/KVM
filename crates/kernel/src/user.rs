//! Ring-3 program + setup + first-entry (iretq) for slice 4b.
//!
//! The ring-3 program is a hand-written, position-independent asm blob embedded
//! in the kernel via a single `global_asm!`. Both code and the literal message
//! sit in the same contiguous symbol range — `user_program_start`..`user_program_end` —
//! so one `copy_nonoverlapping` shifts them together to the user region at 8 MiB,
//! preserving the RIP-relative displacement to `user_msg`.
//!
//! Design: docs/superpowers/specs/2026-05-28-minikvm-month2-syscall-ring3-design.md

// Items unused until Task 4 wires `setup()` and `enter_ring3()` into `_start`.
// Standard rustc 1.95 dead_code workaround; removed in Task 4.
#![allow(dead_code)]

use core::arch::{asm, global_asm};

use crate::gdt;
use crate::paging;

// PD[4] = 0x800000..0xA00000 (8..10 MiB). Code at 0x800000; stack top strictly
// inside the page (0x9FFFF0, NOT 0xA00000 — that address lives in PD[5] and is
// supervisor-only).
const USER_PD_INDEX:  usize = 4;
const USER_ENTRY:     u64   = 0x0080_0000;
const USER_STACK_TOP: u64   = 0x009F_FFF0;

// Ring-3 program: write(1, msg, len); exit(0); ud2.
// Syscall numbers must match SYS_WRITE=1 / SYS_EXIT=2 in syscall.rs (global_asm!
// can't consume Rust consts; the literal must mirror the const).
global_asm!(r#"
    .section .rodata
    .globl user_program_start
    .globl user_program_end
user_program_start:
    mov rax, 1                       // SYS_WRITE
    mov rdi, 1                       // fd = 1 (stdout)
    lea rsi, [rip + user_msg]
    mov rdx, msg_len                 // see .equ below
    syscall
    mov rax, 2                       // SYS_EXIT
    mov rdi, 0                       // code = 0
    syscall
    ud2                              // unreachable (exit halts the kernel)
user_msg:
    .ascii "hello from ring 3\n"
user_msg_end:
user_program_end:

// LLVM's Intel-syntax parser rejects `mov rdx, sym1 - sym2` as an immediate
// (treats it as a memory operand). `.equ` defines a real constant the assembler
// resolves to the same byte count (18 here) and accepts as an imm operand.
.equ msg_len, user_msg_end - user_msg
"#);

extern "C" {
    fn user_program_start();
    fn user_program_end();
}

/// Set up the user region and copy the ring-3 program into it.
///
/// # Safety
/// Call once, before `enter_ring3`. Must run after `paging::init_identity_map`.
pub unsafe fn setup() {
    paging::map_user_pd_entry(USER_PD_INDEX);

    // Cast functions to *const () before integers per rustc's
    // function_casts_as_integer lint.
    let src = user_program_start as *const () as *const u8;
    let len = user_program_end as *const () as usize
            - user_program_start as *const () as usize;
    core::ptr::copy_nonoverlapping(src, USER_ENTRY as *mut u8, len);
}

/// Transition to ring 3 via `iretq`. Never returns.
///
/// Builds the 5-element interrupt-return frame on the kernel stack (in
/// high-to-low push order so the lowest address — top of stack — holds RIP,
/// which `iretq` consumes first), then dispatches.
///
/// # Safety
/// Call once. `setup()` must have run.
pub unsafe fn enter_ring3() -> ! {
    asm!(
        "push {ss}",
        "push {rsp}",
        "push {rflags}",
        "push {cs}",
        "push {rip}",
        "iretq",
        // All five operands are i32 const. PUSH imm32 sign-extends to 64; every
        // value fits signed 32-bit (0x9FFFF0 < 2^31).
        ss     = const ((gdt::USER_SS  | 3) as i32),
        rsp    = const 0x009F_FFF0_i32,    // strictly inside PD[4]
        rflags = const 0x2_i32,            // IF=0, bit 1 reserved-set
        cs     = const ((gdt::USER_CS64 | 3) as i32),
        rip    = const 0x0080_0000_i32,    // user program entry
        options(noreturn),
    );
}
