//! SYSCALL/SYSRET machinery: MSR setup, the asm entry stub (via global_asm!),
//! the dispatch table, and the `write`/`exit` syscall handlers.
//!
//! Design: docs/superpowers/specs/2026-05-28-minikvm-month2-syscall-ring3-design.md

use core::arch::{asm, global_asm};
use core::fmt::Write;
use core::ptr::addr_of_mut;

use crate::io::{rdmsr, wrmsr};
use crate::serial::{self, Serial};

global_asm!(include_str!("syscall_entry.s"));

extern "C" {
    fn syscall_entry();
}

// MSR numbers (IA32_CSTAR=0xC000_0083 intentionally omitted — unused in 64-bit-only).
pub const IA32_EFER:  u32 = 0xC000_0080;
pub const IA32_STAR:  u32 = 0xC000_0081;
pub const IA32_LSTAR: u32 = 0xC000_0082;
pub const IA32_FMASK: u32 = 0xC000_0084;

// Syscall numbers. The asm ring-3 program in user.rs inlines the same literal
// values (global_asm! can't consume Rust consts), so DON'T change these without
// updating user.rs in lockstep.
pub const SYS_WRITE: u64 = 1;
pub const SYS_EXIT:  u64 = 2;

/// Saved syscall state pushed by `syscall_entry.s`. Layout MUST mirror the push
/// order: num at the lowest address.
#[repr(C)]
pub struct SyscallFrame {
    pub num:         u64,
    pub arg1:        u64,
    pub arg2:        u64,
    pub arg3:        u64,
    pub user_rflags: u64,
    pub user_rip:    u64,
}

const SYSCALL_STACK_SIZE: usize = 16 * 1024;

// 16-byte aligned so the `call` site inside the asm stub is 16-aligned per SysV.
// The size is a multiple of 16, so base+SYSCALL_STACK_SIZE stays aligned too.
#[repr(C, align(16))]
struct SyscallStack([u8; SYSCALL_STACK_SIZE]);
static mut SYSCALL_STACK: SyscallStack = SyscallStack([0; SYSCALL_STACK_SIZE]);

// Referenced by syscall_entry.s as `[rip + KERNEL_RSP]`. #[no_mangle] keeps the
// symbol name so the link resolves.
#[no_mangle]
pub static mut KERNEL_RSP: u64 = 0;

/// Program the MSRs and prime KERNEL_RSP. Call once, after gdt::init, before
/// any ring-3 entry.
///
/// # Safety
/// Privileged MSR writes. Must run with interrupts off in CPL 0.
pub unsafe fn init() {
    // 1. Prime KERNEL_RSP FIRST. If a stray SYSCALL happened after we set
    //    EFER.SCE but before KERNEL_RSP was valid, the first xchg would load 0
    //    and the first push would fault.
    KERNEL_RSP = addr_of_mut!(SYSCALL_STACK) as u64 + SYSCALL_STACK_SIZE as u64;

    // 2. EFER: OR in SCE (bit 0). Read-modify-write to preserve LME (bit 8)
    //    and LMA (bit 10) already set by the boot stub.
    let efer = rdmsr(IA32_EFER) | 1;
    wrmsr(IA32_EFER, efer);

    // 3. STAR: SYSCALL base 0x18 in [47:32], SYSRET base 0x28 in [63:48].
    //    Hardcoded values (not gdt::USER_CS32) to make the +16/+8 SYSRETQ
    //    arithmetic explicit: SYSRETQ loads CS = 0x28+16 = 0x38 (USER_CS64)
    //    and SS = 0x28+8 = 0x30 (USER_SS), each OR'd with RPL=3.
    //    DO NOT change 0x28 to 0x38 thinking "the user 64-bit CS is at 0x38".
    //    SYSRET would then compute CS = 0x48 (past the user slots, into the TSS
    //    descriptor) and #GP. The GDT user-segment ordering
    //    (USER_CS32@0x28, USER_SS@0x30, USER_CS64@0x38) is the invariant.
    wrmsr(IA32_STAR, (0x18u64 << 32) | (0x28u64 << 48));

    // 4. LSTAR = kernel SYSCALL entry RIP. Cast via *const () per rustc's
    //    function_casts_as_integer lint (direct fn-to-u64 is deprecated style).
    wrmsr(IA32_LSTAR, syscall_entry as *const () as u64);

    // 5. FMASK = 0x700 — clears TF(8), IF(9), DF(10) on SYSCALL entry via
    //    RFLAGS &= ~FMASK. IF is the load-bearing one.
    wrmsr(IA32_FMASK, 0x700);
}

/// C-ABI entry from the asm stub. Returns the syscall's u64 in RAX, which the
/// stub places in user RAX before sysretq. sys_exit is `-> !`; its return type
/// coerces to u64 (unreached at runtime).
///
/// Takes a raw pointer (not `&SyscallFrame`) to sidestep Rust's reference
/// aliasing rules: the kernel stack continues to grow during the call, and the
/// frame lives at a fixed offset above RSP. The frame's bytes are valid (we
/// pushed them moments before) and unaliased (single vCPU, non-reentrant), so
/// the `unsafe { &*f }` inside is sound.
#[no_mangle]
pub unsafe extern "C" fn rust_syscall_dispatch(f: *const SyscallFrame) -> u64 {
    let f = &*f;
    match f.num {
        SYS_WRITE => sys_write(f.arg1, f.arg2, f.arg3),
        SYS_EXIT  => sys_exit(f.arg1),
        _         => u64::MAX,
    }
}

fn sys_write(fd: u64, buf: u64, len: u64) -> u64 {
    if fd != 1 { return u64::MAX; }
    // SMAP is off; ring 0 freely reads USER pages. User buffers are arbitrary
    // bytes (not necessarily UTF-8), so we use write_bytes, not write_str.
    let bytes = unsafe { core::slice::from_raw_parts(buf as *const u8, len as usize) };
    serial::write_bytes(bytes);
    len
}

fn sys_exit(code: u64) -> ! {
    let mut com = Serial;
    let _ = writeln!(com, "[guest] user exited (code {})", code);
    loop {
        unsafe { asm!("hlt", options(nomem, nostack)); }
    }
}
