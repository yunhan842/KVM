//! Slice-7 at-merge load-bearing assertion: `--gdb` makes the VMM print the
//! "gdb stub listening" line and wait for a client. Spawns the VMM with an
//! ephemeral port (--gdb-port 0), asserts the listening line on stderr, then
//! kills it. Gated on /dev/kvm + guest.img presence like run_kernel.rs.
//!
//! Invokes the built binary directly via `CARGO_BIN_EXE_vmm` (the same
//! mechanism run_kernel.rs uses) rather than `cargo run`, so the test never
//! triggers a rebuild and the listening line appears within a second.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn gdb_listening_line_appears_on_stderr() {
    if !Path::new("/dev/kvm").exists() {
        eprintln!("SKIP: /dev/kvm not present");
        return;
    }

    // Same guest.img location convention as run_kernel.rs: CARGO_MANIFEST_DIR
    // is crates/vmm, so ../../guest.img is the workspace-root image.
    let img = concat!(env!("CARGO_MANIFEST_DIR"), "/../../guest.img");
    if !Path::new(img).exists() {
        eprintln!("SKIP: missing {img} — run `make` from the workspace root");
        return;
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_vmm"))
        .args(["run", img, "--gdb", "--gdb-port", "0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn vmm");

    let stderr = child.stderr.take().expect("stderr piped");
    let mut reader = BufReader::new(stderr);

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut saw_listen = false;
    while Instant::now() < deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                eprintln!("[vmm-stderr] {}", line.trim_end());
                if line.contains("gdb stub listening on 127.0.0.1:") {
                    saw_listen = true;
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    assert!(saw_listen, "did not see the gdb stub listening line within 30s");
}
