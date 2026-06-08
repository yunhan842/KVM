//! IDT and CPU exception handling for the guest kernel.
//!
//! This module owns three things:
//!   1. The asm trampolines in `idt_stubs.s`, pulled in via `global_asm!`.
//!   2. The 256-entry IDT structure and the gate-descriptor encoding.
//!   3. The Rust dispatcher that asm calls into (placeholder in Task 1,
//!      filled in in Task 3).
//!
//! Design rationale: see docs/superpowers/specs/2026-05-24-minikvm-month2-idt-design.md

use core::arch::{asm, global_asm};
use core::fmt::Write;
use core::ptr::addr_of_mut;

use crate::serial::Serial;

global_asm!(include_str!("idt_stubs.s"));

// Symbols emitted by idt_stubs.s. One per CPU-exception vector 0..32.
// We take their addresses below and place them in IDT gate descriptors.
extern "C" {
    fn isr_0();  fn isr_1();  fn isr_2();  fn isr_3();
    fn isr_4();  fn isr_5();  fn isr_6();  fn isr_7();
    fn isr_8();  fn isr_9();  fn isr_10(); fn isr_11();
    fn isr_12(); fn isr_13(); fn isr_14(); fn isr_15();
    fn isr_16(); fn isr_17(); fn isr_18(); fn isr_19();
    fn isr_20(); fn isr_21(); fn isr_22(); fn isr_23();
    fn isr_24(); fn isr_25(); fn isr_26(); fn isr_27();
    fn isr_28(); fn isr_29(); fn isr_30(); fn isr_31();
}

// 16-byte long-mode gate descriptor. Intel SDM Vol 3A Sec 6.14.1.
//
// `selector` = 0x18 (the 64-bit code segment from guest/boot.asm).
// `type_attr` = 0x8E : P=1, DPL=0, type=0xE (64-bit *interrupt* gate -- IF is
// cleared on entry, which is what we want since we run with interrupts off).
// `ist` = 0 (no IST stack switch this slice; deferred with TSS).
#[repr(C, packed)]
#[derive(Copy, Clone)]
struct GateDescriptor {
    offset_low:  u16,
    selector:    u16,
    ist:         u8,
    type_attr:   u8,
    offset_mid:  u16,
    offset_high: u32,
    reserved:    u32,
}

impl GateDescriptor {
    const fn empty() -> Self {
        Self {
            offset_low: 0, selector: 0, ist: 0, type_attr: 0,
            offset_mid: 0, offset_high: 0, reserved: 0,
        }
    }

    /// Build a present, ring-0 interrupt gate pointing at `handler`.
    const fn new(handler: u64) -> Self {
        Self {
            offset_low:  handler as u16,
            selector:    0x18,
            ist:         0,
            type_attr:   0x8E,
            offset_mid:  (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved:    0,
        }
    }

    /// Build a present, ring-3-callable interrupt gate pointing at `handler`.
    /// Used for vector 3 (#BP) so a CPL=3 `int3` reaches the host gdb stub
    /// after detach (under KVM_GUESTDBG_USE_SW_BP, KVM intercepts before the
    /// guest IDT anyway; this is belt + suspenders for the post-detach path).
    const fn new_dpl3(handler: u64) -> Self {
        Self {
            offset_low:  handler as u16,
            selector:    0x18,
            ist:         0,
            type_attr:   0xEE,  // P=1, DPL=3, type=0xE
            offset_mid:  (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved:    0,
        }
    }
}

// 256 entries x 16 bytes = 4096 bytes. 4 KiB-aligned: the physical address
// of IDT (= its virtual address under our identity-mapped paging) becomes
// the value loaded into IDTR.base, no offset arithmetic needed.
#[repr(C, align(4096))]
struct Idt([GateDescriptor; 256]);

static mut IDT: Idt = Idt([GateDescriptor::empty(); 256]);

// Layout consumed by the `lidt` instruction: a packed 10-byte structure
// containing the IDT limit (size-1) followed by the IDT base address.
#[repr(C, packed)]
struct Idtr {
    limit: u16,
    base:  u64,
}

/// Saved CPU + GPR state at the moment an exception was delivered.
///
/// Layout mirrors the push order in idt_stubs.s::isr_common exactly. The asm
/// pushes GPRs in `rax,rcx,rdx,rbx,rbp,rsi,rdi,r8..r15` order; the LAST push
/// (r15) sits at the lowest address, so reading the stack as a struct in
/// ascending memory order yields the fields below.
#[repr(C)]
pub struct InterruptContext {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub r11: u64, pub r10: u64, pub r9:  u64, pub r8:  u64,
    pub rdi: u64, pub rsi: u64, pub rbp: u64,
    pub rbx: u64, pub rdx: u64, pub rcx: u64, pub rax: u64,
    // Pushed by isr_<N>:
    pub vector:     u64,
    pub error_code: u64,
    // Pushed by the CPU on interrupt entry:
    pub rip:    u64,
    pub cs:     u64,
    pub rflags: u64,
    pub rsp:    u64,
    pub ss:     u64,
}

/// Install handlers for vectors 0..32 and execute `lidt`.
///
/// # Safety
/// Must be called once, with interrupts off, after paging is up (so the IDT's
/// virtual address is mapped at the matching physical address).
pub unsafe fn init() {
    // *mut GateDescriptor pointing at IDT[0]. Going via addr_of_mut! avoids
    // ever taking a reference to `static mut IDT` (UB-risky / lint-warned).
    let entries = addr_of_mut!(IDT) as *mut GateDescriptor;

    // 32 trampoline addresses, in vector order.
    let handlers = [
        isr_0,  isr_1,  isr_2,  isr_3,
        isr_4,  isr_5,  isr_6,  isr_7,
        isr_8,  isr_9,  isr_10, isr_11,
        isr_12, isr_13, isr_14, isr_15,
        isr_16, isr_17, isr_18, isr_19,
        isr_20, isr_21, isr_22, isr_23,
        isr_24, isr_25, isr_26, isr_27,
        isr_28, isr_29, isr_30, isr_31,
    ];

    let mut i = 0usize;
    while i < 32 {
        let handler_addr = handlers[i] as usize as u64;
        let gate = if i == 3 {
            GateDescriptor::new_dpl3(handler_addr)
        } else {
            GateDescriptor::new(handler_addr)
        };
        entries.add(i).write(gate);
        i += 1;
    }

    let idtr = Idtr {
        limit: (core::mem::size_of::<Idt>() - 1) as u16,
        base:  addr_of_mut!(IDT) as u64,
    };

    asm!(
        "lidt [{}]",
        in(reg) &idtr,
        options(readonly, nostack, preserves_flags),
    );
}

/// Point an installed IDT entry at an IST stack (`ist` = 1..7, or 0 for none).
///
/// The IST index is bits 0..3 of the byte at offset 4 within a 16-byte gate
/// descriptor; bits 3..8 are reserved-zero (already zero from `init`'s encoder).
/// Safe to call after `lidt`: the CPU re-reads the descriptor on each dispatch.
///
/// # Safety
/// `vector` must be < 256 and the IDT must already be initialized.
pub unsafe fn set_ist(vector: usize, ist: u8) {
    let entries = addr_of_mut!(IDT) as *mut u8;
    let ist_byte = entries.add(vector * 16 + 4);
    *ist_byte = ist & 0x7;
}

/// Common Rust entry from `isr_common`.
///
/// Prints a one-line context dump over COM1 and halts. Doesn't try to recover:
/// returning to faulting code is a scheduler/syscall concern this slice doesn't
/// own. The host sees the resulting `hlt` as KVM_EXIT_HLT and the run loop
/// terminates normally.
#[no_mangle]
pub extern "C" fn rust_isr_dispatch(ctx: &InterruptContext) -> ! {
    // Copy the fields we read into locals -- the InterruptContext is `repr(C)`
    // (not packed), so direct field access is well-aligned, but pulling values
    // into locals keeps the writeln! call site tidy and avoids any temptation
    // to take a reference to a struct field across the macro expansion.
    let vector = ctx.vector;
    let rip = ctx.rip;
    let rflags = ctx.rflags;
    let err = ctx.error_code;

    let mut com = Serial;
    let _ = writeln!(
        com,
        "[guest] EXCEPTION {} ({}) at rip={:#x} rflags={:#x} err={:#x}",
        vector,
        exception_name(vector),
        rip,
        rflags,
        err,
    );

    loop {
        unsafe { asm!("hlt", options(nomem, nostack)); }
    }
}

/// Map a CPU exception vector to its Intel-mnemonic name.
/// Vectors not defined by the architecture (9, 15, 22..32) fall through to
/// "Reserved"; they can't actually be raised this slice but the table is
/// future-proof against accidental IRQ vectors landing in 0..32.
fn exception_name(vector: u64) -> &'static str {
    match vector {
        0  => "#DE Divide Error",
        1  => "#DB Debug",
        2  => "NMI Non-Maskable Interrupt",
        3  => "#BP Breakpoint",
        4  => "#OF Overflow",
        5  => "#BR Bound Range Exceeded",
        6  => "#UD Invalid Opcode",
        7  => "#NM Device Not Available",
        8  => "#DF Double Fault",
        10 => "#TS Invalid TSS",
        11 => "#NP Segment Not Present",
        12 => "#SS Stack-Segment Fault",
        13 => "#GP General Protection",
        14 => "#PF Page Fault",
        16 => "#MF x87 FPU Floating-Point Error",
        17 => "#AC Alignment Check",
        18 => "#MC Machine Check",
        19 => "#XM SIMD Floating-Point Exception",
        20 => "#VE Virtualization Exception",
        21 => "#CP Control Protection Exception",
        _  => "Reserved",
    }
}
