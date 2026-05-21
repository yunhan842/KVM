# MiniKVM Month-1 Host VMM Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust VMM that opens `/dev/kvm`, maps guest memory, runs one vCPU executing a tiny 16-bit real-mode guest blob, handles serial-output and HLT exits, prints `hello from guest`, and reports exit stats under `--trace`.

**Architecture:** A Cargo workspace with one binary crate `vmm`. Leaf modules (`config`, `serial`, `stats`) are pure and unit-tested. The KVM glue (`vm`, `vcpu`) is verified by an end-to-end integration test that runs the built binary against a committed `hello.bin` (auto-skipped when `/dev/kvm` is absent). The guest is a throwaway NASM blob — long mode and paging are deferred to Month 2.

**Tech Stack:** Rust (2021), `kvm-ioctls`, `kvm-bindings`, `vm-memory` (`backend-mmap`), `anyhow`; NASM for the guest blob.

**Environment note:** All commands run inside WSL2 Ubuntu from the workspace root `~/projects/KVM`. `/dev/kvm` must be accessible (`[ -r /dev/kvm ] && [ -w /dev/kvm ] && echo ok`).

---

### Task 0: Scaffold the Cargo workspace

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/vmm/Cargo.toml`
- Create: `crates/vmm/src/main.rs`
- Create: `.gitignore`

- [ ] **Step 1: Create the workspace manifest**

Create `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = ["crates/vmm"]
```

- [ ] **Step 2: Create the vmm crate manifest**

Create `crates/vmm/Cargo.toml`:

```toml
[package]
name = "vmm"
version = "0.1.0"
edition = "2021"

[dependencies]
```

- [ ] **Step 3: Create a placeholder main**

Create `crates/vmm/src/main.rs`:

```rust
fn main() {
    println!("minikvm vmm");
}
```

- [ ] **Step 4: Create .gitignore**

Create `.gitignore`:

```gitignore
/target
**/*.rs.bk
Cargo.lock
```

Note: `Cargo.lock` is ignored because this workspace's only deliverable is a binary used locally for learning; if you later want reproducible CI builds, remove this line and commit the lock.

- [ ] **Step 5: Verify it builds**

Run: `cargo build`
Expected: compiles, produces `target/debug/vmm`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/vmm/Cargo.toml crates/vmm/src/main.rs .gitignore
git commit -m "scaffold: cargo workspace with vmm binary crate"
```

---

### Task 1: The guest blob (NASM)

**Files:**
- Create: `guest/hello.asm`
- Create (build artifact, committed): `guest/hello.bin`

- [ ] **Step 1: Write the guest assembly**

Create `guest/hello.asm`:

```nasm
; guest/hello.asm — Month-1 throwaway guest.
; 16-bit real mode. Writes a string to COM1 (port 0x3f8) byte-by-byte, then halts.
; The VMM loads this at guest-physical 0x1000 with CS/DS base = 0 and RIP = 0x1000.
bits 16
org 0x1000

start:
    mov dx, 0x3f8          ; COM1 data port
    mov si, msg
.next:
    lodsb                  ; al = [ds:si], si++  (DF=0 so forward)
    test al, al
    jz .done
    out dx, al             ; -> KVM_EXIT_IO; host prints the byte
    jmp .next
.done:
    hlt                    ; -> KVM_EXIT_HLT; host stops the run loop

msg: db "hello from guest", 10, 0   ; 10 = '\n', 0 = terminator
```

- [ ] **Step 2: Assemble to a flat binary**

Run: `nasm -f bin guest/hello.asm -o guest/hello.bin`
Expected: creates `guest/hello.bin` (a few dozen bytes, no errors).

- [ ] **Step 3: Sanity-check the bytes**

Run: `ndisasm -b 16 -o 0x1000 guest/hello.bin | head -20`
Expected: disassembly shows `mov dx,0x3f8`, a `lodsb`/`out dx,al` loop, and `hlt`. The string `hello from guest` is visible with `xxd guest/hello.bin`.

- [ ] **Step 4: Commit (including the binary artifact)**

```bash
git add guest/hello.asm guest/hello.bin
git commit -m "feat: add 16-bit real-mode guest blob that prints to COM1 and halts"
```

Note: we commit `hello.bin` so the integration test (Task 7) has a fixed artifact without needing a build step. Re-run Step 2 whenever `hello.asm` changes.

---

### Task 2: `stats` module (TDD)

**Files:**
- Create: `crates/vmm/src/stats.rs`
- Modify: `crates/vmm/src/main.rs` (declare `mod stats;`)
- Test: inline `#[cfg(test)]` in `crates/vmm/src/stats.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/vmm/src/stats.rs`:

```rust
//! VM-exit counters and a trace summary line.

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub io_exits: u64,
    pub hlt_exits: u64,
    pub mmio_exits: u64,
}

impl Stats {
    pub fn record_io(&mut self) {
        self.io_exits += 1;
    }

    pub fn record_hlt(&mut self) {
        self.hlt_exits += 1;
    }

    pub fn record_mmio(&mut self) {
        self.mmio_exits += 1;
    }

    /// One-line summary matching the project's demo format.
    pub fn summary(&self) -> String {
        format!(
            "[host] VM exits: io={}, hlt={}, mmio={}",
            self.io_exits, self.hlt_exits, self.mmio_exits
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_formats() {
        let mut s = Stats::default();
        s.record_io();
        s.record_io();
        s.record_hlt();
        assert_eq!(s.io_exits, 2);
        assert_eq!(s.hlt_exits, 1);
        assert_eq!(s.summary(), "[host] VM exits: io=2, hlt=1, mmio=0");
    }
}
```

Add `mod stats;` to the top of `crates/vmm/src/main.rs`:

```rust
mod stats;

fn main() {
    println!("minikvm vmm");
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p vmm stats`
Expected: PASS (`counts_and_formats`). The implementation and test are written together here because the logic is trivial; if it fails, the assertion message pinpoints the mismatch.

- [ ] **Step 3: Commit**

```bash
git add crates/vmm/src/stats.rs crates/vmm/src/main.rs
git commit -m "feat: add VM-exit stats counter with trace summary"
```

---

### Task 3: `serial` module (TDD)

**Files:**
- Create: `crates/vmm/src/serial.rs`
- Modify: `crates/vmm/src/main.rs` (declare `mod serial;`)
- Test: inline `#[cfg(test)]` in `crates/vmm/src/serial.rs`

- [ ] **Step 1: Write the module and its failing test**

Create `crates/vmm/src/serial.rs`:

```rust
//! Month-1 COM1 handler: bytes the guest `out`s to port 0x3f8 are written to a sink.
//! This is intentionally NOT a 16550 UART — no status/IER registers. The guest blob
//! writes raw data bytes only. Full UART emulation arrives in Month 2.

use std::io::Write;

pub const COM1_PORT: u16 = 0x3f8;

pub struct Serial<W: Write> {
    out: W,
}

impl<W: Write> Serial<W> {
    pub fn new(out: W) -> Self {
        Self { out }
    }

    /// Write the bytes from a single KVM IO-out exit to the sink.
    /// Errors are intentionally swallowed: a console write failure must not abort the VM.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        let _ = self.out.write_all(bytes);
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forwards_bytes_to_sink() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut serial = Serial::new(&mut buf);
            serial.write_bytes(b"hi");
            serial.write_bytes(&[b'!']);
        }
        assert_eq!(buf, b"hi!");
    }
}
```

Add `mod serial;` to `crates/vmm/src/main.rs`:

```rust
mod serial;
mod stats;

fn main() {
    println!("minikvm vmm");
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p vmm serial`
Expected: PASS (`forwards_bytes_to_sink`).

- [ ] **Step 3: Commit**

```bash
git add crates/vmm/src/serial.rs crates/vmm/src/main.rs
git commit -m "feat: add COM1 serial handler forwarding bytes to a writer"
```

---

### Task 4: `config` module — CLI parsing (TDD)

**Files:**
- Create: `crates/vmm/src/config.rs`
- Modify: `crates/vmm/src/main.rs` (declare `mod config;`)
- Test: inline `#[cfg(test)]` in `crates/vmm/src/config.rs`

- [ ] **Step 1: Write the module and failing tests**

Create `crates/vmm/src/config.rs`:

```rust
//! Hand-rolled CLI parsing for `minikvm run <guest.bin> [--trace]`.
//! No clap yet (YAGNI) — one subcommand, one positional, one flag.

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub guest_path: String,
    pub trace: bool,
}

/// Parse args *after* the program name (i.e. `std::env::args().skip(1)`).
pub fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut iter = args.iter();
    match iter.next().map(String::as_str) {
        Some("run") => {}
        Some(other) => return Err(format!("unknown command '{other}'; expected 'run'")),
        None => return Err("usage: minikvm run <guest.bin> [--trace]".to_string()),
    }

    let mut guest_path: Option<String> = None;
    let mut trace = false;
    for arg in iter {
        match arg.as_str() {
            "--trace" => trace = true,
            s if s.starts_with('-') => return Err(format!("unknown flag '{s}'")),
            s => {
                if guest_path.is_some() {
                    return Err(format!("unexpected extra argument '{s}'"));
                }
                guest_path = Some(s.to_string());
            }
        }
    }

    let guest_path = guest_path.ok_or("missing guest image path")?;
    Ok(Config { guest_path, trace })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_run_with_trace() {
        let cfg = parse_args(&v(&["run", "guest/hello.bin", "--trace"])).unwrap();
        assert_eq!(
            cfg,
            Config { guest_path: "guest/hello.bin".to_string(), trace: true }
        );
    }

    #[test]
    fn trace_defaults_off() {
        let cfg = parse_args(&v(&["run", "g.bin"])).unwrap();
        assert!(!cfg.trace);
    }

    #[test]
    fn missing_path_is_error() {
        assert!(parse_args(&v(&["run"])).is_err());
    }

    #[test]
    fn unknown_command_is_error() {
        assert!(parse_args(&v(&["boot", "g.bin"])).is_err());
    }
}
```

Add `mod config;` to `crates/vmm/src/main.rs`:

```rust
mod config;
mod serial;
mod stats;

fn main() {
    println!("minikvm vmm");
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p vmm config`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/vmm/src/config.rs crates/vmm/src/main.rs
git commit -m "feat: add CLI arg parsing for 'run <guest.bin> [--trace]'"
```

---

### Task 5: Add KVM dependencies

**Files:**
- Modify: `crates/vmm/Cargo.toml`

- [ ] **Step 1: Add anyhow and vm-memory**

Run:
```bash
cargo add anyhow -p vmm
cargo add vm-memory -p vmm --features backend-mmap
```
Expected: both appear under `[dependencies]` in `crates/vmm/Cargo.toml`.

- [ ] **Step 2: Add kvm-ioctls**

Run: `cargo add kvm-ioctls -p vmm`
Expected: `kvm-ioctls` added.

- [ ] **Step 3: Add kvm-bindings at the version kvm-ioctls uses (avoids a type mismatch)**

`kvm-ioctls::VmFd::set_user_memory_region` takes a `kvm_bindings::kvm_userspace_memory_region`. If your direct `kvm-bindings` dep resolves to a *different* version than the one `kvm-ioctls` depends on, you get two copies in the tree and a confusing type error at the call site. Pin them to the same version:

Run:
```bash
cargo tree -i kvm-bindings    # note the version kvm-ioctls pulled, e.g. 0.x.y
cargo add kvm-bindings@<that-exact-version> -p vmm
```
Expected: `cargo tree -i kvm-bindings` now shows a single version. Record this gotcha in your dev log.

- [ ] **Step 4: Verify the tree builds**

Run: `cargo build -p vmm`
Expected: compiles (the placeholder main still runs).

- [ ] **Step 5: Commit**

```bash
git add crates/vmm/Cargo.toml
git commit -m "build: add kvm-ioctls, kvm-bindings, vm-memory, anyhow deps"
```

---

### Task 6: `vm` and `vcpu` modules — the KVM glue

**Files:**
- Create: `crates/vmm/src/vm.rs`
- Create: `crates/vmm/src/vcpu.rs`
- Modify: `crates/vmm/src/main.rs` (declare modules + full run wiring)

These modules call into `/dev/kvm` and can't be unit-tested without the device; they're exercised by the integration test in Task 7 and the manual run below.

- [ ] **Step 1: Write the `vm` module**

Create `crates/vmm/src/vm.rs`:

```rust
//! Open /dev/kvm, create the VM, map guest memory, and load the guest blob.

use anyhow::{Context, Result};
use kvm_bindings::kvm_userspace_memory_region;
use kvm_ioctls::{Kvm, VmFd};
use vm_memory::{Bytes, GuestAddress, GuestMemory, GuestMemoryMmap};

/// Size of the single guest memory region, mapped at guest-physical 0x0.
pub const MEM_SIZE: usize = 64 * 1024 * 1024;
/// Where the guest blob is copied and where execution starts.
pub const GUEST_LOAD_ADDR: u64 = 0x1000;

/// Open /dev/kvm with an actionable error if it isn't usable.
pub fn open_kvm() -> Result<Kvm> {
    Kvm::new().context(
        "failed to open /dev/kvm — is it present (`ls -l /dev/kvm`) and are you in the 'kvm' group? \
         (sudo usermod -aG kvm $USER, then `wsl --shutdown` and reopen)",
    )
}

/// Allocate a 64 MiB host-backed region and register it with the VM at GPA 0.
pub fn setup_memory(vm_fd: &VmFd) -> Result<GuestMemoryMmap> {
    let mem = GuestMemoryMmap::<()>::from_ranges(&[(GuestAddress(0), MEM_SIZE)])
        .context("failed to mmap guest memory")?;

    let host_addr = mem
        .get_host_address(GuestAddress(0))
        .context("failed to resolve host address of guest memory")? as u64;

    let region = kvm_userspace_memory_region {
        slot: 0,
        guest_phys_addr: 0,
        memory_size: MEM_SIZE as u64,
        userspace_addr: host_addr,
        flags: 0,
    };
    // Safety: `host_addr` points to a live mmap of `MEM_SIZE` bytes owned by `mem`,
    // which the caller keeps alive for the lifetime of the VM.
    unsafe { vm_fd.set_user_memory_region(region) }
        .context("KVM_SET_USER_MEMORY_REGION failed")?;

    Ok(mem)
}

/// Copy the guest blob into guest memory at GUEST_LOAD_ADDR.
pub fn load_guest(mem: &GuestMemoryMmap, blob: &[u8]) -> Result<()> {
    mem.write_slice(blob, GuestAddress(GUEST_LOAD_ADDR))
        .context("failed to copy guest blob into guest memory")?;
    Ok(())
}
```

- [ ] **Step 2: Write the `vcpu` module**

Create `crates/vmm/src/vcpu.rs`:

```rust
//! Create the vCPU in 16-bit real mode and run it until HLT, dispatching exits.

use anyhow::{bail, Context, Result};
use kvm_ioctls::{VcpuExit, VcpuFd, VmFd};
use std::io::Write;

use crate::serial::{Serial, COM1_PORT};
use crate::stats::Stats;
use crate::vm::GUEST_LOAD_ADDR;

/// Create vCPU 0 and set initial 16-bit real-mode register state.
pub fn create_vcpu(vm_fd: &VmFd) -> Result<VcpuFd> {
    let vcpu = vm_fd.create_vcpu(0).context("KVM_CREATE_VCPU failed")?;

    // Real mode with flat segments based at 0 so linear addr == offset.
    // (CR0.PE is 0 at reset, which get_sregs reflects — we leave it real mode.)
    let mut sregs = vcpu.get_sregs().context("KVM_GET_SREGS failed")?;
    for seg in [&mut sregs.cs, &mut sregs.ds, &mut sregs.es, &mut sregs.ss] {
        seg.base = 0;
        seg.selector = 0;
    }
    vcpu.set_sregs(&sregs).context("KVM_SET_SREGS failed")?;

    let mut regs = vcpu.get_regs().context("KVM_GET_REGS failed")?;
    regs.rip = GUEST_LOAD_ADDR; // start at the loaded blob
    regs.rflags = 0x2; // bit 1 is reserved-and-must-be-1; DF=0 so lodsb counts up
    // The blob uses no stack (no push/call), so RSP is left at its reset value.
    vcpu.set_regs(&regs).context("KVM_SET_REGS failed")?;

    Ok(vcpu)
}

/// Run the vCPU until HLT, forwarding serial output and counting exits.
pub fn run<W: Write>(vcpu: &mut VcpuFd, serial: &mut Serial<W>, stats: &mut Stats) -> Result<()> {
    loop {
        match vcpu.run().context("KVM_RUN failed")? {
            VcpuExit::IoOut(port, data) => {
                stats.record_io();
                if port == COM1_PORT {
                    serial.write_bytes(data);
                }
            }
            VcpuExit::IoIn(_, _) => {
                stats.record_io();
            }
            VcpuExit::MmioRead(_, _) | VcpuExit::MmioWrite(_, _) => {
                stats.record_mmio();
            }
            VcpuExit::Hlt => {
                stats.record_hlt();
                break;
            }
            other => bail!("unexpected VM exit: {other:?}"),
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Wire up `main.rs`**

Replace `crates/vmm/src/main.rs` with:

```rust
mod config;
mod serial;
mod stats;
mod vcpu;
mod vm;

use anyhow::{anyhow, Result};
use std::io;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = config::parse_args(&args).map_err(|e| anyhow!(e))?;

    let log = |msg: &str| {
        if cfg.trace {
            println!("{msg}");
        }
    };

    let blob = std::fs::read(&cfg.guest_path)
        .map_err(|e| anyhow!("failed to read guest image '{}': {e}", cfg.guest_path))?;

    let kvm = vm::open_kvm()?;
    let vm_fd = kvm.create_vm()?;
    // Needed for real-mode guests on Intel CPUs without "unrestricted guest";
    // harmless otherwise. (Dev-log gotcha.)
    vm_fd
        .set_tss_address(0xfffb_d000)
        .map_err(|e| anyhow!("KVM_SET_TSS_ADDR failed: {e}"))?;
    log("[host] created VM");

    let mem = vm::setup_memory(&vm_fd)?;
    log(&format!(
        "[host] mapped {} MiB guest memory",
        vm::MEM_SIZE / (1024 * 1024)
    ));
    vm::load_guest(&mem, &blob)?;

    let mut vcpu = vcpu::create_vcpu(&vm_fd)?;
    log("[host] created vCPU 0");

    let stdout = io::stdout();
    let mut serial = serial::Serial::new(stdout.lock());
    let mut stats = stats::Stats::default();

    let start = std::time::Instant::now();
    vcpu::run(&mut vcpu, &mut serial, &mut stats)?;
    let elapsed = start.elapsed();

    if cfg.trace {
        println!("{}", stats.summary());
        println!("[host] runtime: {elapsed:?}");
    }
    Ok(())
}
```

- [ ] **Step 4: Build**

Run: `cargo build -p vmm`
Expected: compiles cleanly. If `set_user_memory_region` shows a type mismatch, revisit Task 5 Step 3 (kvm-bindings version pin).

- [ ] **Step 5: Manual run (the real milestone)**

Run: `cargo run -p vmm -- run guest/hello.bin --trace`
Expected output:
```
[host] created VM
[host] mapped 64 MiB guest memory
[host] created vCPU 0
hello from guest
[host] VM exits: io=17, hlt=1, mmio=0
[host] runtime: ...
```
(`io=17` = 16 characters + newline. If `/dev/kvm` is missing or you lack group access, you'll get the actionable error from `open_kvm`.)

Also run without trace: `cargo run -p vmm -- run guest/hello.bin`
Expected: only `hello from guest`.

- [ ] **Step 6: Commit**

```bash
git add crates/vmm/src/vm.rs crates/vmm/src/vcpu.rs crates/vmm/src/main.rs
git commit -m "feat: boot real-mode guest via KVM, handle IO/HLT exits, trace stats"
```

---

### Task 7: End-to-end integration test

**Files:**
- Create: `crates/vmm/tests/run_guest.rs`

- [ ] **Step 1: Write the integration test**

Create `crates/vmm/tests/run_guest.rs`:

```rust
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

    let output = Command::new(env!("CARGO_BIN_EXE_vmm"))
        .args(["run", blob, "--trace"])
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
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test -p vmm --test run_guest -- --nocapture`
Expected: PASS on the WSL2 box (prints the guest output); prints `SKIP` and passes if `/dev/kvm` is absent.

- [ ] **Step 3: Run the full suite**

Run: `cargo test -p vmm`
Expected: all unit tests (`stats`, `serial`, `config`) plus the integration test pass.

- [ ] **Step 4: Commit**

```bash
git add crates/vmm/tests/run_guest.rs
git commit -m "test: add end-to-end integration test booting the guest blob"
```

---

### Task 8: Dev-log note

**Files:**
- Create: `docs/devlog.md`

- [ ] **Step 1: Capture the non-obvious Month-1 decisions**

Create `docs/devlog.md`:

```markdown
# MiniKVM Dev Log

## Month 1 — Host VMM

- Guest memory: single 64 MiB region at GPA 0x0; guest blob loaded at 0x1000.
- vCPU starts in 16-bit real mode (CR0.PE=0 at reset); flat segments based at 0 so
  linear address == offset, which is why `org 0x1000` + RIP 0x1000 + DS base 0 line up.
- `KVM_SET_TSS_ADDR` (0xfffbd000) is required for real-mode guests on Intel CPUs without
  "unrestricted guest"; harmless on CPUs that have it.
- kvm-ioctls and kvm-bindings must resolve to compatible versions, or
  `set_user_memory_region` fails to type-check (two kvm_userspace_memory_region types).
- Serial is a stub (raw bytes -> stdout), NOT a 16550 UART. Real driver/UART is Month 2.
- `io=17` for "hello from guest\n" = 16 chars + newline.
```

- [ ] **Step 2: Commit**

```bash
git add docs/devlog.md
git commit -m "docs: start dev log with Month-1 decisions"
```

---

## Definition of Done

- `cargo test -p vmm` passes (unit + integration) on the WSL2 box.
- `cargo run -p vmm -- run guest/hello.bin --trace` prints the host lifecycle lines,
  `hello from guest`, and `VM exits: io=17, hlt=1, mmio=0`.
- Without `--trace`, only `hello from guest` is printed.
- Code is committed in small, focused commits per task.

## Deferred to Month 2 (do not pull forward)

Long mode / paging / GDT / IDT / real boot stub; full 16550 UART emulation; the Rust
`no_std` guest kernel, syscalls, and user/kernel transition. (Per spec §9.)
