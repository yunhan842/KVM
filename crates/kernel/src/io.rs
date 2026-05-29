//! Minimal port I/O wrappers (the only place the kernel uses raw `in`/`out`).

pub unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val,
                     options(nomem, nostack, preserves_flags));
}

pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port,
                     options(nomem, nostack, preserves_flags));
    val
}

/// Read a 64-bit MSR. MSR index in ECX, value returned in EDX:EAX (high:low).
///
/// # Safety
/// Privileged instruction. Caller must ensure `msr` is a defined index.
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (eax, edx): (u32, u32);
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") eax,
        out("edx") edx,
        options(nomem, nostack, preserves_flags),
    );
    ((edx as u64) << 32) | (eax as u64)
}

/// Write a 64-bit MSR. MSR index in ECX, value taken from EDX:EAX (high:low).
///
/// # Safety
/// Privileged instruction. Caller must ensure `msr` is a defined index and
/// `val` is a legal value for it (bad values raise #GP).
pub unsafe fn wrmsr(msr: u32, val: u64) {
    let eax = val as u32;
    let edx = (val >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") eax,
        in("edx") edx,
        options(nomem, nostack, preserves_flags),
    );
}
