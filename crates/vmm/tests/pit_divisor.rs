//! Host-side guard for the kernel's PIT divisor math (crates/kernel/src/interrupts.rs).
//! The kernel crate is no_std/no_main and can't host-test, so we re-derive the
//! identical pure function and pin the same values (cf. idt_gate_encoding.rs).

const PIT_FREQ: u32 = 1_193_182;

fn pit_divisor(hz: u32) -> u16 {
    let d = PIT_FREQ / hz;
    if d == 0 {
        1
    } else if d > 0xFFFF {
        0xFFFF
    } else {
        d as u16
    }
}

#[test]
fn divisor_1khz() {
    assert_eq!(pit_divisor(1000), 1193); // 1_193_182 / 1000
}

#[test]
fn divisor_clamps_high() {
    assert_eq!(pit_divisor(18), 0xFFFF); // 1_193_182 / 18 = 66287 > 0xFFFF
    assert_eq!(pit_divisor(1_193_182), 1); // exactly the input clock
}

#[test]
fn divisor_clamps_low() {
    assert_eq!(pit_divisor(2_000_000), 1); // faster than the clock → raw div 0 → clamp
}
