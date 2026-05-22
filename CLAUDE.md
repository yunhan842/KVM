# MiniKVM

A from-scratch KVM-based microVM stack: a Rust host VMM that boots a custom guest
kernel, which loads and runs freestanding C userspace programs. Full project reference:
`MiniKVM_Project_Plan.pdf`. The full design for the current phase lives in
`docs/superpowers/specs/`.

## Current phase

**Month 2 (slice 1) — Boot stub + long mode + Rust kernel.** Month 1 (host VMM + raw
guest execution) is complete and merged. Now: a hand-written asm stub climbs
real→protected→long mode and hands off to a `no_std` Rust kernel that builds a 64 MiB
identity-mapped page table and prints over a polled 16550 UART. Design spec:
`docs/superpowers/specs/2026-05-22-minikvm-month2-guest-kernel-design.md`
(Month-1 spec retained at `docs/superpowers/specs/2026-05-20-minikvm-month1-host-vmm-design.md`).

We pace by **milestones, not the calendar** — the plan's "3 months" is a learning estimate.
This spec is the first slice of the plan's Month 2 (weeks 5–7); IDT/heap/syscalls/user mode
are deferred to follow-on specs.

## Environment

- **WSL2 Ubuntu on Windows 11.** `/dev/kvm` is present and the dev user is in the `kvm`
  group (KVM works without sudo). Do NOT assume a cloud VM.
- Repo lives in the **WSL ext4 filesystem** (`~/projects/KVM`), not `/mnt/c` — build there.
- Run Claude Code from inside WSL so it can build/run/test against `/dev/kvm` directly.
- Toolchain: `rustc`/`cargo` (via rustup), `nasm`, `build-essential`. Confirm with
  `rustc --version && nasm --version && ls -l /dev/kvm`.

## Locked design decisions

- **Approach A (Month-1 guest):** a *throwaway* hand-written **16-bit real-mode** asm blob
  that `out`s a string to COM1 (port `0x3f8`) and `hlt`s. The real-mode → long-mode boot
  path and paging are **deferred to Month 2** (the documented stall point — don't pull it
  forward).
- **Cargo workspace.** Month-1 binary is `crates/vmm`; Month 2 (`kernel`) and Month 3
  (`userspace`) join as sibling crates later.
- **Crates:** `kvm-ioctls`, `kvm-bindings`, `vm-memory`, `anyhow`. When using a safe
  ioctl wrapper, explain what it does underneath (learning goal: understand every layer).
- **Month-1 serial is simplified:** handle raw bytes written to `0x3f8` → stdout. No full
  16550 UART (LSR/IER) emulation until Month 2.

## Language roles (do not violate)

- **Assembly** — entry stubs, real→long mode, IDT stubs, syscall/sysret entry only. Not
  general logic.
- **Rust (`no_std` for guest kernel)** — VMM host + all kernel logic above the asm stubs.
- **C** — freestanding userspace programs and libc-like wrappers only. Never kernel/VMM.

## Scope discipline (out of scope for the whole project)

No multiple vCPUs/SMP, networking/virtio-net, virtio-blk/disk images, full Linux guest,
filesystem, or an early scheduler. One vCPU, no network, no disk.

## Working style

The developer is a comfortable programmer with some Rust/systems exposure (has done xv6).
Give well-structured code and design, explain the tricky/non-obvious parts, skip basics.
Keep a running dev log of non-obvious decisions (memory layout, exit handling, syscall
design) for the eventual README.

## Next step

Turn the Month-2 slice-1 spec into an ordered implementation plan (kernel crate scaffold →
`boot.asm` real→long stub → Rust kernel entry + serial driver → full paging → host UART
upgrade → Makefile/`guest.img` → tests), then implement, explaining each piece in build order.
