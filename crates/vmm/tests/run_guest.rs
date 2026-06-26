//! End-to-end: run the built VMM against the committed guest blob.
//! Auto-skips when /dev/kvm is unavailable (e.g. on Windows/CI without KVM).

use std::path::Path;
use std::process::Command;

#[test]
fn boots_guest_and_prints_hello() {
    if !Path::new("/dev/kvm").exists() {
        eprintln!("SKIP: /dev/kvm not present");
        return;
    }

    // Guest blob lives at the workspace root: crates/vmm/../../guest/hello.bin
    let blob = concat!(env!("CARGO_MANIFEST_DIR"), "/../../guest/hello.bin");
    assert!(
        Path::new(blob).exists(),
        "missing {blob} — run `nasm -f bin guest/hello.asm -o guest/hello.bin`"
    );

    // --no-irqchip: this throwaway blob ends with `hlt` and relies on
    // KVM_EXIT_HLT reaching userspace (asserted below as hlt=1). The default
    // in-kernel irqchip (added in the timer-interrupt slice) would instead make
    // the LAPIC swallow the halt into a wait-for-interrupt and hang it, since
    // the blob runs with IF=0 and no IDT.
    let output = Command::new(env!("CARGO_BIN_EXE_vmm"))
        .args(["run", blob, "--trace", "--no-irqchip"])
        .output()
        .expect("failed to spawn vmm");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "vmm failed: {stderr}\n{stdout}");
    assert!(
        stdout.contains("hello from guest"),
        "guest output missing; got:\n{stdout}"
    );
    assert!(
        stdout.contains("hlt=1"),
        "expected exactly one HLT exit; got:\n{stdout}"
    );
}
