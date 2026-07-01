//! 8259 PIC + 8254 PIT timer-interrupt setup and the timer IRQ handler.
//!
//! KVM provides the in-kernel PIC/PIT (created VMM-side); here the guest
//! programs them: remap the PIC so IRQ0 lands on a free vector, set the PIT
//! frequency, then enable IF. The handler counts ticks, prints the first few
//! with the interrupted RIP (proving asynchronous preemption), and EOIs.

use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::io::outb;
use crate::serial::Serial;

// 8259 PIC ports.
const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;
const PIC_EOI: u8 = 0x20; // OCW2 non-specific EOI

// 8254 PIT ports.
const PIT_CH0: u16 = 0x40;
const PIT_CMD: u16 = 0x43;
const PIT_FREQ: u32 = 1_193_182; // input clock

const TICK_HZ: u32 = 1000;
const PRINT_FIRST_N: u64 = 3;

static TICKS: AtomicU64 = AtomicU64::new(0);

/// PIT reload divisor for `hz`, clamped to a valid 16-bit range. Pure — the
/// host-side test `crates/vmm/tests/pit_divisor.rs` mirrors this and pins the
/// values (the kernel crate is no_std/no_main and can't host-test).
pub fn pit_divisor(hz: u32) -> u16 {
    let d = PIT_FREQ / hz;
    if d == 0 {
        1
    } else if d > 0xFFFF {
        0xFFFF
    } else {
        d as u16
    }
}

/// Remap the 8259 PIC so master IRQs occupy vectors 0x20..0x27 and slave
/// 0x28..0x2F (away from CPU exception vectors 0..31), then mask every line
/// except IRQ0 (the PIT). Standard ICW1..ICW4 + OCW1 sequence.
///
/// # Safety
/// Port I/O to the (in-kernel) PIC; call once during init, interrupts off.
pub unsafe fn remap_pic() {
    outb(PIC1_CMD, 0x11); // ICW1: begin init, expect ICW4
    outb(PIC2_CMD, 0x11);
    outb(PIC1_DATA, 0x20); // ICW2: master vectors 0x20..0x27
    outb(PIC2_DATA, 0x28); // ICW2: slave vectors 0x28..0x2F
    outb(PIC1_DATA, 0x04); // ICW3: slave on master IRQ2 (bit 2)
    outb(PIC2_DATA, 0x02); // ICW3: slave cascade identity 2
    outb(PIC1_DATA, 0x01); // ICW4: 8086/88 mode
    outb(PIC2_DATA, 0x01);
    outb(PIC1_DATA, 0xFE); // OCW1: master mask — unmask only IRQ0
    outb(PIC2_DATA, 0xFF); // OCW1: slave mask — mask all
}

/// Program PIT channel 0 as a rate generator (mode 2) at `TICK_HZ`.
///
/// # Safety
/// Port I/O to the (in-kernel) PIT; call once during init, interrupts off.
pub unsafe fn program_pit() {
    let div = pit_divisor(TICK_HZ);
    outb(PIT_CMD, 0x34); // channel 0, lobyte/hibyte, mode 2, binary
    outb(PIT_CH0, (div & 0xFF) as u8);
    outb(PIT_CH0, (div >> 8) as u8);
}

/// Unmask IRQ0 (the PIT) at the master PIC, leaving every other line masked.
/// After this call an `IF=1` context (i.e. ring 3) starts taking timer
/// interrupts. Pairs with `remap_pic`, which masks IRQ0 initially so boot and
/// the syscall benchmark run interrupt-free.
///
/// # Safety
/// Port I/O to the (in-kernel) PIC; call after `remap_pic`.
pub unsafe fn unmask_timer() {
    outb(PIC1_DATA, 0xFE); // unmask only IRQ0 (bit 0 clear), keep IRQ1..7 masked
}

/// Remap the PIC and program the PIT. Call once after `idt::init()` and
/// `gdt::init()`, with interrupts still disabled.
///
/// # Safety
/// See `remap_pic`/`program_pit`.
pub unsafe fn init() {
    remap_pic();
    program_pit();
}

/// Enable interrupt delivery (`sti`). Call once, after `init()`, when ready to
/// be preempted — here, right before entering ring 3.
///
/// # Safety
/// Only call after the IDT gates for 0x20..0x2F and the PIC remap are in place.
pub unsafe fn enable() {
    core::arch::asm!("sti", options(nomem, nostack));
}

/// Timer IRQ handler body (called from `rust_isr_dispatch` for vector 0x20).
/// `rip` is the interrupted instruction pointer (from the IRQ frame). Counts the
/// tick, prints the first few with their RIP, and EOIs the master PIC.
pub fn handle_timer(rip: u64) {
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= PRINT_FIRST_N {
        let mut com = Serial;
        let _ = writeln!(com, "[guest] timer tick {} (interrupted rip={:#x})", n, rip);
    }
    // EOI BEFORE iretq so the PIC will deliver the next IRQ0.
    unsafe {
        outb(PIC1_CMD, PIC_EOI);
    }
}

/// Total ticks handled so far. Read at exit (IF is off in the syscall path, so
/// the value is stable).
pub fn tick_count() -> u64 {
    TICKS.load(Ordering::Relaxed)
}
