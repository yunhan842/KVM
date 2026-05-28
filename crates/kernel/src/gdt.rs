//! GDT, TSS, and IST setup.
//!
//! Extends the bootstrap GDT from `boot.asm` with user-mode segments (for slice
//! 4b's SYSCALL/SYSRET) and a TSS descriptor, installs a 64-bit TSS with a
//! dedicated IST stack for double faults, and points the IDT's #DF entry
//! (vector 8) at that stack.
//!
//! Layout note: the kernel CS/SS selectors (0x18 / 0x20) are kept IDENTICAL to
//! boot.asm so the already-installed IDT gates (which reference 0x18) and the
//! currently-running CS stay valid. Because no kernel selector changes, lgdt
//! does NOT require a segment-register reload -- the cached descriptors match
//! the new GDT slots byte-for-byte. Only `ltr` (new) and `set_ist` are needed.
//!
//! Design: docs/superpowers/specs/2026-05-28-minikvm-month2-gdt-tss-design.md

use core::arch::asm;
use core::mem::size_of;
use core::ptr::addr_of_mut;

// Segment selectors = byte offsets into the GDT.
// Kernel CS/SS match boot.asm. The user selectors are loaded by SYSCALL/SYSRET
// in slice 4b; unused in 4a but documented here as the layout contract.
#[allow(dead_code)]
pub const KERNEL_CS: u16 = 0x18;
#[allow(dead_code)]
pub const KERNEL_SS: u16 = 0x20;
#[allow(dead_code)]
pub const USER_CS32: u16 = 0x28;
#[allow(dead_code)]
pub const USER_SS: u16 = 0x30;
#[allow(dead_code)]
pub const USER_CS64: u16 = 0x38;
pub const TSS_SEL: u16 = 0x40;

// Long-mode segment descriptors. Base/limit are vestigial (paging is
// authoritative); only P/S/DPL/type/L bits matter. The first four match
// boot.asm exactly, so swapping GDTs leaves the running CS/SS valid.
const CODE32: u64 = 0x00CF9A000000FFFF; // 0x08 32-bit code (boot.asm, vestigial)
const DATA32: u64 = 0x00CF92000000FFFF; // 0x10 32-bit data (boot.asm, vestigial)
const KCODE: u64 = 0x00AF9A000000FFFF; // 0x18 kernel 64-bit code (boot.asm)
const KDATA: u64 = 0x00AF92000000FFFF; // 0x20 kernel 64-bit data (boot.asm)
const UCODE32: u64 = 0x00CFFA000000FFFF; // 0x28 user 32-bit code (DPL=3)
const UDATA: u64 = 0x00CFF2000000FFFF; // 0x30 user data (DPL=3)
const UCODE64: u64 = 0x00AFFA000000FFFF; // 0x38 user 64-bit code (DPL=3, L=1)

const KERNEL_STACK_TOP: u64 = 0x100000;
const IST_STACK_SIZE: usize = 4096;

#[repr(C, align(16))]
struct IstStack([u8; IST_STACK_SIZE]);
static mut IST1_STACK: IstStack = IstStack([0; IST_STACK_SIZE]);

// 64-bit TSS, exactly 104 bytes (Intel SDM Vol 3A §7.7).
#[repr(C, packed)]
struct Tss {
    reserved_0: u32,
    rsp: [u64; 3], // RSP0/1/2
    reserved_1: u64,
    ist: [u64; 7], // IST1..IST7 (ist[0] == architectural IST1)
    reserved_2: u64,
    reserved_3: u16,
    iopb: u16,
}

static mut TSS: Tss = Tss {
    reserved_0: 0,
    rsp: [0; 3],
    reserved_1: 0,
    ist: [0; 7],
    reserved_2: 0,
    reserved_3: 0,
    iopb: 0,
};

// 10 slots x 8 bytes = 80 bytes: null, c32, d32, kCS, kSS, uCS32, uSS, uCS64,
// TSS(2 slots).
#[repr(C, align(16))]
struct Gdt {
    entries: [u64; 10],
}

static mut GDT: Gdt = Gdt { entries: [0; 10] };

#[repr(C, packed)]
struct Gdtr {
    limit: u16,
    base: u64,
}

/// Build a 16-byte (two-slot) long-mode TSS descriptor from base + limit.
const fn tss_descriptor(base: u64, limit: u32) -> [u64; 2] {
    let low = (limit as u64 & 0xFFFF)
        | ((base & 0xFF_FFFF) << 16)
        | (0x89u64 << 40) // access byte: P=1, S=0, type=9 (available 64-bit TSS)
        | ((((limit as u64) >> 16) & 0xF) << 48)
        | (((base >> 24) & 0xFF) << 56);
    let high = (base >> 32) & 0xFFFF_FFFF;
    [low, high]
}

/// Build and load the new GDT + TSS, and point #DF at IST1.
///
/// # Safety
/// Call once, after the IDT is installed (we mutate IDT entry 8), interrupts off.
pub unsafe fn init() {
    // 1. Populate the TSS.
    let ist1_top = addr_of_mut!(IST1_STACK) as u64 + IST_STACK_SIZE as u64;
    let tss = addr_of_mut!(TSS);
    (*tss).rsp[0] = KERNEL_STACK_TOP;
    (*tss).ist[0] = ist1_top;
    (*tss).iopb = size_of::<Tss>() as u16; // 104 -> IO bitmap base past TSS -> none

    // 2. Populate the GDT (the TSS descriptor needs the TSS base address).
    let tss_desc = tss_descriptor(tss as u64, (size_of::<Tss>() - 1) as u32);
    let gdt = addr_of_mut!(GDT);
    (*gdt).entries[0] = 0;
    (*gdt).entries[1] = CODE32;
    (*gdt).entries[2] = DATA32;
    (*gdt).entries[3] = KCODE;
    (*gdt).entries[4] = KDATA;
    (*gdt).entries[5] = UCODE32;
    (*gdt).entries[6] = UDATA;
    (*gdt).entries[7] = UCODE64;
    (*gdt).entries[8] = tss_desc[0];
    (*gdt).entries[9] = tss_desc[1];

    // 3. lgdt. No segment reload needed: kernel CS (0x18) / SS (0x20) keep the
    //    same selectors AND descriptors as boot.asm, so the cached segment
    //    descriptors stay valid against the new table.
    let gdtr = Gdtr {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: gdt as u64,
    };
    asm!("lgdt [{}]", in(reg) &gdtr, options(readonly, nostack, preserves_flags));

    // 4. ltr: load the task register with the TSS selector (marks TSS busy).
    asm!("ltr {0:x}", in(reg) TSS_SEL, options(nostack));

    // 5. Point #DF (vector 8) at IST1.
    crate::idt::set_ist(8, 1);
}
