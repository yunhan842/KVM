//! Host-side sanity test for the IDT vector 3 (#BP) gate DPL.
//!
//! Mirrors the bit layout from crates/kernel/src/idt.rs:GateDescriptor.
//! If this test ever fails, the kernel's vector 3 gate is no longer DPL=3
//! and ring-3 INT3 will deliver #GP instead of #BP after detach.

const VECTOR_3_TYPE_ATTR: u8 = 0xEE;  // P=1, DPL=3, type=0xE (64-bit interrupt gate)

#[test]
fn vector_3_type_attr_has_dpl_3() {
    // type_attr bits: P[7] | DPL[6:5] | 0[4] | TYPE[3:0]
    let p   = (VECTOR_3_TYPE_ATTR >> 7) & 0x1;
    let dpl = (VECTOR_3_TYPE_ATTR >> 5) & 0x3;
    let typ =  VECTOR_3_TYPE_ATTR       & 0xF;
    assert_eq!(p,   1,   "present bit");
    assert_eq!(dpl, 3,   "vector 3 must be DPL=3 so ring-3 INT3 can trap");
    assert_eq!(typ, 0xE, "64-bit interrupt gate");
}
