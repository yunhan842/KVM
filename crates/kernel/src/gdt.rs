//! GDT, TSS, and IST setup.
//!
//! Replaces the bootstrap GDT from `boot.asm` with a kernel-owned GDT that adds
//! user-mode segments (for slice 4b's SYSCALL/SYSRET) and a TSS descriptor.
//! Installs a 64-bit TSS with a dedicated IST stack for double faults, and
//! points the IDT's #DF entry (vector 8) at that stack.
//!
//! Design: docs/superpowers/specs/2026-05-28-minikvm-month2-gdt-tss-design.md

// Every item is unused until Task 2 calls `init()` from `_start`. Same rustc
// 1.95 dead_code ICE workaround as the earlier slices; removed in Task 2.
#![allow(dead_code)]

use core::arch::asm;
use core::mem::size_of;
use core::ptr::addr_of_mut;

// Segment selectors = byte offsets into the GDT.
pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_SS: u16 = 0x10;
pub const USER_CS32: u16 = 0x18;
pub const USER_SS: u16 = 0x20;
pub const USER_CS64: u16 = 0x28;
pub const TSS_SEL: u16 = 0x30;

// Long-mode segment descriptors. Base/limit are vestigial (paging is
// authoritative); only P/S/DPL/type/L bits matter.
const KCODE: u64 = 0x00AF9A000000FFFF; // ring0 64-bit code (L=1, DPL=0)
const KDATA: u64 = 0x00CF92000000FFFF; // ring0 data (DPL=0)
const UCODE32: u64 = 0x00CFFA000000FFFF; // ring3 32-bit code (DPL=3)
const UDATA: u64 = 0x00CFF2000000FFFF; // ring3 data (DPL=3)
const UCODE64: u64 = 0x00AFFA000000FFFF; // ring3 64-bit code (L=1, DPL=3)

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

// 8 slots x 8 bytes = 64 bytes: null, kCS, kSS, uCS32, uSS, uCS64, TSS(2 slots).
#[repr(C, align(16))]
struct Gdt {
    entries: [u64; 8],
}

static mut GDT: Gdt = Gdt { entries: [0; 8] };

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

/// Build and load the new GDT + TSS + IST, and point #DF at IST1.
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
    (*gdt).entries[1] = KCODE;
    (*gdt).entries[2] = KDATA;
    (*gdt).entries[3] = UCODE32;
    (*gdt).entries[4] = UDATA;
    (*gdt).entries[5] = UCODE64;
    (*gdt).entries[6] = tss_desc[0];
    (*gdt).entries[7] = tss_desc[1];

    // 3. lgdt.
    let gdtr = Gdtr {
        limit: (size_of::<Gdt>() - 1) as u16,
        base: gdt as u64,
    };
    asm!("lgdt [{}]", in(reg) &gdtr, options(readonly, nostack, preserves_flags));

    // 4. Reload all segment registers from the new GDT.
    reload_segments();

    // 5. ltr: load the task register with the TSS selector (marks TSS busy).
    asm!("ltr {0:x}", in(reg) TSS_SEL, options(nostack));

    // 6. Point #DF (vector 8) at IST1.
    crate::idt::set_ist(8, 1);
}

/// Reload CS via an iretq synthetic frame (the only way to load CS in long
/// mode), then reload SS/DS/ES/FS/GS to the kernel data selector.
#[inline(always)]
unsafe fn reload_segments() {
    asm!(
        "mov rax, rsp",
        "push {kss}",          // SS  (push reg = 8 bytes; low 16 = selector)
        "push rax",            // RSP (round-trip the current value)
        "pushfq",              // RFLAGS
        "push {kcs}",          // CS
        "lea rax, [rip + 2f]", // RIP after iretq
        "push rax",
        "iretq",
        "2:",
        "mov ds, {kss:x}",
        "mov es, {kss:x}",
        "mov fs, {kss:x}",
        "mov gs, {kss:x}",
        kss = in(reg) KERNEL_SS as u64,
        kcs = const KERNEL_CS as i32,
        out("rax") _,
    );
}
