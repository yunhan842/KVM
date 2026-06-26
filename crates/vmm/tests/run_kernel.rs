//! End-to-end: run the VMM against the committed guest.img (boot stub + Rust kernel).
//! Auto-skips when /dev/kvm is unavailable. Regenerate guest.img with `make`.

use std::path::Path;
use std::process::Command;

#[test]
fn kernel_boots_to_long_mode_and_prints() {
    if !Path::new("/dev/kvm").exists() {
        eprintln!("SKIP: /dev/kvm not present");
        return;
    }

    let img = concat!(env!("CARGO_MANIFEST_DIR"), "/../../guest.img");
    assert!(
        Path::new(img).exists(),
        "missing {img} — run `make` from the workspace root"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_vmm"))
        .args(["run", img, "--trace"])
        .output()
        .expect("failed to spawn vmm");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "vmm failed: {stderr}\n{stdout}");

    for needle in [
        "[guest] kernel entered",
        "[guest] paging enabled",
        "[guest] idt loaded",
        "[guest] heap initialized",
        "[guest] heap demo: Vec<u32>={0,1,2,3,4} Box<u64>=0xDEADBEEF",
        "[guest] gdt+tss installed",
        "hello from the kernel",
        "[guest] syscall enabled",
        "[guest] host uptime via MMIO:",               // NEW (slice 8) — prefix only; ns is variable
        "[guest] interrupts: PIC remapped, PIT at 1000 Hz", // NEW (slice 9)
        "[guest] loaded /bin/hello",                   // NEW (slice 5)
        "[guest] entering ring 3",
        "[guest] timer tick 1 (interrupted rip=0x",    // NEW (slice 9) — PIT IRQ0 preempts ring 3; rip is variable
        "[user] hello from C userspace",               // NEW (slice 5; was "hello from ring 3")
        "[guest] user exited (code 0)",
        "[guest] handled ",                            // NEW (slice 9) — tick total; count prefix only (host-timing-variable)
        "[host] guest powered off via MMIO",           // NEW (slice 8)
        "[host] avg syscall latency:",                 // NEW (slice 6) — prefix only; M ns is hardware-variable
        "hlt=0",                                        // slice 8: poweroff replaces hlt
        "mmio=2",                                       // slice 8: uptime read + poweroff write
    ] {
        assert!(stdout.contains(needle), "missing '{needle}' in:\n{stdout}");
    }
}
