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
