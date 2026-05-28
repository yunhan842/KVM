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
        "[guest] testing #DF on IST1",
        "[guest] EXCEPTION 8",
        "hlt=1",
    ] {
        assert!(stdout.contains(needle), "missing '{needle}' in:\n{stdout}");
    }
}
