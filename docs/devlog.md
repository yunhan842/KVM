# MiniKVM Dev Log

## Month 1 — Host VMM

### Design / memory layout
- Guest memory: single 64 MiB region at GPA 0x0; guest blob loaded at 0x1000.
- vCPU starts in 16-bit real mode (CR0.PE=0 at reset); flat segments based at 0 so
  linear address == offset, which is why `org 0x1000` + RIP 0x1000 + DS base 0 line up.
- `KVM_SET_TSS_ADDR` (0xfffbd000) is required for real-mode guests on Intel CPUs without
  "unrestricted guest"; harmless on CPUs that have it.
- Serial is a stub (raw bytes -> stdout), NOT a 16550 UART. Real driver/UART is Month 2.
- `io=17` for "hello from guest\n" = 16 chars + newline.

### Toolchain / dependency gotchas
- kvm-ioctls and kvm-bindings must resolve to compatible versions, or
  `set_user_memory_region` fails to type-check (two `kvm_userspace_memory_region` types).
  Resolved: kvm-ioctls 0.24 pulls kvm-bindings 0.14, so we pin our direct dep to 0.14.
- **vm-memory 0.18 API drift:** `get_host_address` moved off the `GuestMemory` trait onto
  a new `GuestMemoryBackend` trait. The plan (written against an older vm-memory) imported
  `GuestMemory`; the fix is `use vm_memory::{Bytes, GuestAddress, GuestMemoryBackend,
  GuestMemoryMmap};`. `Bytes` still provides `write_slice`.
- **rustc 1.95.0 ICE while emitting dead_code warnings.** During incremental TDD, a `pub`
  item unused until a later task (e.g. `Stats::record_mmio`, wired up only in the run loop)
  triggers a `dead_code` warning, and rustc 1.95.0's diagnostic renderer panics rendering
  it (`slice index starts at 16 but ends at 14` in `annotate_snippets::StyledBuffer`).
  It is a compiler renderer bug, not our code. Worked around with a temporary crate-root
  `#![allow(dead_code)]`, removed once the run loop used every counter (no dead code → no
  warning → no ICE). Revisit after a rustup update.

### Environment note (dev workflow, not the VMM)
- Claude Code ran on the Windows side of WSL; its default shell saw Windows Git Bash with
  no `/dev/kvm`. All build/run/test/git must go through `wsl.exe -e bash -lc "cd
  ~/projects/KVM && ..."`. Windows `git` also misreports the WSL-hosted tree as dirty
  (filemode/line-ending differences) — trust WSL's `git status`, not Windows'.

## Month 2 (slice 1) — Boot stub + long mode + Rust kernel

This slice corresponds to the project plan's weeks 5–7 (boot stub + paging, "the hardest
phase"). IDT/heap/syscalls/user-mode are deliberately deferred to follow-on specs.

### Architecture / memory layout
- Single `guest.img` = `boot.bin` (NASM, padded to *exactly* 4 KiB) ++ `kernel.bin`
  (Rust → `objcopy -O binary`). Stub loads at `0x1000`; kernel entry is therefore at
  `0x2000`. The fixed-pad concatenation means the "address contract" is just those two
  round constants, named identically in the stub's `jmp` target, the kernel's linker base,
  and the VMM's loader.
- Memory map (within 64 MiB): boot stub `0x1000` (4 KiB), kernel image `0x2000`+, bootstrap
  page tables at `0x90000`/`0x91000`/`0x92000`, stack top `0x100000`. All chosen so the
  early stack and the kernel land inside the stub's 2 MiB bootstrap map.
- Boot path: real → (`CR0.PE` + far `jmp 0x08:`) → 32-bit protected → build stage-1 tables
  + `CR4.PAE` + `CR3` + `EFER.LME` + `CR0.PG` → (far `jmp 0x18:` to L-bit segment) → 64-bit
  long mode → enable SSE → `jmp` to kernel at `0x2000`. Stage-1 identity-maps only the
  first 2 MiB (one 2 MiB huge page); the Rust kernel then builds a fresh 4-level map
  identity-mapping all 64 MiB (32 × 2 MiB huge pages) and reloads `CR3`.

### Bring-up gotchas (the real surprises)
- **Bare-metal Rust linked PIE by default.** With just `target = x86_64-unknown-none` +
  `link.ld` setting `. = 0x2000`, the linker placed `.dynsym`/`.gnu.hash`/`.eh_frame`/etc.
  *ahead* of `.text`, so `_start` ended up at `0x2080`. The flat binary's first 128 bytes
  were zeros and the kernel never ran. **Fix:** `rustflags = ["-C", "relocation-model=static"]`
  in `crates/kernel/.cargo/config.toml` (kills the dynamic sections) **plus** a `/DISCARD/`
  block in the linker script that drops `.eh_frame*`, `.note*`, `.comment`. Verify with
  `readelf -SW` and `nm | grep _start` — entry should match `0x2000` and `_start` should be
  at that address.
- **`wrmsr EFER.LME` triple-faulted on the first run** (`KVM_EXIT_SHUTDOWN` between phase
  markers `C` and `E`). Long-mode enable requires the guest CPUID to advertise the LM bit
  (CPUID.80000001H:EDX[29]), but KVM populates **no** CPUID by default — Month 1 happened
  not to need it. **Fix (host-side):** `create_vcpu` now takes `kvm: &Kvm` and runs
  `kvm.get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)?` then `vcpu.set_cpuid2(&cpuid)?`. The
  spec's "vCPU init unchanged" was wrong on this point.
- **Phase-marker debugging is load-bearing without an IDT.** A boot-stub mistake is a silent
  triple-fault → shutdown. Solution baked into the design: each transition `out`s a marker
  byte to COM1 (raw `out`, no LSR polling — so it works against the *unchanged* Month-1
  host). The last byte printed pinpoints the failing step. The first bring-up showed `1`
  (protected ✓) then nothing; an extra round of finer probes (`P`/`A`/`C`/`E`/`G`/`L`)
  localised the CPUID fault to `wrmsr` in seconds — exactly the technique the PDF says
  saves people "1–2 weeks." Removed once the path is stable.
- **The DLAB/divisor trap.** The classic 16550 init writes a baud divisor to port `0x3f8`
  with DLAB set. If the host blindly forwarded `0x3f8` writes to stdout, that divisor byte
  would print as garbage before any real output. The host now tracks DLAB in the LCR write
  and suppresses data-port writes while it's set.

### Build & toolchain
- Stable Rust, no nightly: `rustup target add x86_64-unknown-none` then explicit
  `--target` (no `-Zbuild-std`). `memcpy`/`memset`/`memmove`/`memcmp` are hand-written
  (~12 lines each, `#[no_mangle] pub unsafe extern "C"`) so the compiler's calls to those
  symbols resolve without pulling in `compiler-builtins-mem` via `build-std-features`.
- Kernel build config is isolated in `crates/kernel/.cargo/config.toml`; the workspace's
  `default-members = ["crates/vmm"]` keeps a bare `cargo build`/`cargo test` from the
  workspace root from ever touching the bare-metal crate (which would fail trying to build
  it for the host).
- The linker script path is passed via `build.rs` (`cargo:rustc-link-arg=-T{CARGO_MANIFEST_DIR}/link.ld`)
  rather than a relative path, so it works regardless of CWD.

### x86 mode-transition trivia worth remembering
- **GDT in real mode:** `lgdt` loads only a 24-bit base in 16-bit mode (no `o32` prefix);
  fine here because the GDT sits inside the stub at ~`0x10xx`, well under 16 MiB.
- **The far-jump rule:** the only way to reload `CS` is a far jump (or `iret`/`retf`/etc.).
  We use far jumps for every mode transition (real→protected, protected→long).
- **Enabling SSE before Rust runs is non-optional.** The compiler can emit SSE instructions
  (e.g. for struct copies); executing one with SSE disabled faults. The stub clears
  `CR0.EM`, sets `CR0.MP`, and sets `CR4.OSFXSR` / `CR4.OSXMMEXCPT` before the kernel jump.
- **A20 line:** not gated under KVM in our path; the boot worked without enabling it.
  Would matter on bare metal / real BIOS.

### Two-stage paging split
- The stub builds the *minimum* tables to enter long mode (identity-map first 2 MiB with
  one huge page — three live entries across PML4/PDPT/PD, hardcoded at `0x90000`+).
- The kernel later builds its *own* full 4-level tables (32 × 2 MiB huge pages → 64 MiB),
  stored as `#[repr(C, align(4096))]` `static mut` arrays in `.bss`, and writes their
  address into `CR3` via inline asm. Writing `CR3` implicitly flushes the TLB, and because
  the new map is identity, RIP/RSP stay valid across the switch.
- **Modern Rust idiom:** use `core::ptr::addr_of_mut!(PML4)` to get a `*mut PageTable`
  *without* creating a reference. Refs to `static mut` allow undefined aliasing and are
  warning/error in recent editions.

### Host changes (kept minimal)
- `vcpu.rs::create_vcpu` now sets guest CPUID (above).
- `serial.rs` grew from a write-only sink into a tiny `Uart<W>` with `write_reg`/`read_reg`
  and a `dlab: bool`. The run loop dispatches IO exits in the COM1 port range
  (`0x3f8..=0x3ff`) to the model. Four unit tests cover the DLAB and LSR behaviour.
- Everything else (vCPU init shape, memory mapping, image load address) is unchanged from
  Month 1.

### Exit-count sanity checks
- `io=121` for the Task-3 output ("[guest] kernel entered\n" + "hello from the kernel (long
  mode)\n" = 57 chars; 2 exits per char (LSR poll + data write) + ~7 init writes = 121).
- `io=167` after adding "[guest] paging enabled\n" (23 chars × 2 = +46 → 167). Don't assert
  these in tests — polling makes them inherently variable. Assert on substrings instead.
