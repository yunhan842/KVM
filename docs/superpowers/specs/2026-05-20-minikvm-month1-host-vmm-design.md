# MiniKVM — Month 1: Host VMM + Raw Guest Execution (Design)

- **Date:** 2026-05-20
- **Status:** Approved (design); pending implementation plan
- **Scope:** Month 1 of the MiniKVM project only

## 1. Context & Scope

MiniKVM is a from-scratch KVM-based microVM stack (see `MiniKVM_Project_Plan.pdf`).
The full project decomposes into three independent sub-projects, each with its own
spec → plan → build cycle:

1. **Month 1 — Host VMM (this spec):** a Rust program that drives the Linux KVM API
   to boot and run a *tiny* guest.
2. Month 2 — Tiny guest kernel (Rust `no_std` + asm boot path).
3. Month 3 — C userspace + measurements.

This document covers **Month 1 only**. The "3 months" in the plan is a learning-paced
estimate; we pace by milestones, not the calendar, and treat later phases as a backlog.

### Environment (decided)

- Development happens in **WSL2 Ubuntu** on Windows 11. `/dev/kvm` is confirmed present
  (kernel 6.6.x). No cloud VM or dual-boot needed.
- The code repo lives in the **WSL ext4 filesystem** (`~/minikvm`), not under `/mnt/c`,
  for build performance and correct file permissions/line endings.
- Implementation/build/test happens inside WSL (run Claude Code from `~/minikvm`).
- Prerequisites: user in the `kvm` group; `build-essential`, `nasm`, `git`, and Rust via
  `rustup` installed.

## 2. Goal

A Rust VMM that opens `/dev/kvm`, maps guest memory, loads a tiny guest, runs one vCPU,
handles serial-output and HLT exits, and prints `hello from guest`.

### Done criteria

```
$ minikvm run guest/hello.bin --trace
[host] created VM
[host] mapped 64 MiB guest memory
[host] created vCPU 0
hello from guest
[host] VM exits: io=16, hlt=1
```

## 3. Approach (decided)

**Approach A — 16-bit real-mode guest blob.** The vCPU starts in 16-bit real mode (the
x86 reset state). The Month-1 guest is a few bytes of hand-written assembly that `out`s a
string to COM1 (port `0x3f8`) and `hlt`s. This is the canonical LWN "Using the KVM API" /
rust-vmm starting point. It exercises every host-side mechanism (memory map, register
setup, IO exit, HLT exit) **without** paging or long mode.

The hard real-mode → protected-mode → long-mode boot path is **deliberately deferred to
Month 2** (the plan's weeks 5–6, the documented stall point). The Month-1 guest is a
throwaway, not the real kernel.

Crates: `kvm-ioctls` (safe ioctl wrappers), `kvm-bindings` (struct defs), `vm-memory`
(guest memory), `anyhow` (binary-level error ergonomics). As we implement each ioctl, we
explain what the safe wrapper does underneath — understanding without the footguns.

## 4. Architecture

Cargo **workspace** so Month 2/3 crates join as siblings:

```
minikvm/
├── Cargo.toml            # [workspace]
├── crates/
│   └── vmm/              # Month-1 deliverable: the host VMM binary
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs   # CLI: `minikvm run <guest.bin> [--trace]`
│           ├── vm.rs     # open /dev/kvm, create VM, map guest memory, load blob
│           ├── vcpu.rs   # create vCPU, set initial real-mode registers, run loop
│           ├── exit.rs   # dispatch VM exits (IO, HLT, unknown)
│           ├── serial.rs # COM1 (port 0x3f8) → host stdout
│           └── stats.rs  # exit counters + timing
└── guest/
    └── hello.asm         # 16-bit real-mode blob (NASM) → assembled to hello.bin
```

## 5. Components

- **main.rs** — Parse args: guest image path + optional `--trace`. Hand-rolled arg
  parsing is sufficient (YAGNI — no `clap` yet). Wires the pieces together and prints the
  trace summary.
- **vm.rs** — `Kvm::new()` opens `/dev/kvm`; check API version; `create_vm()`; allocate a
  64 MiB `GuestMemoryMmap` region at guest-physical `0x0`; register it as a user memory
  region; copy `hello.bin` into guest memory at `0x1000`.
- **vcpu.rs** — `create_vcpu(0)`. Set `sregs` for **16-bit real mode** (CS base 0). Set
  `regs`: `RIP = 0x1000`, `RFLAGS` with bit 1 set, a valid stack pointer. Then drive the
  run loop, delegating each exit to `exit.rs`.
- **exit.rs** — `match vcpu.run()`: `IoOut` → `serial`; `Hlt` → terminate loop; any other
  reason → log the reason and return a fatal error (never silently loop).
- **serial.rs** — Month-1 simplification: the blob blasts raw bytes to `0x3f8` without
  checking UART status registers, so the handler simply takes the byte and writes it to
  stdout. **Not** a full 16550 UART (LSR/IER) — that arrives in Month 2 when a real kernel
  serial driver polls the Line Status Register.
- **stats.rs** — Count exits per reason and total wall-clock runtime. Printed only under
  `--trace`.

## 6. Data flow

```
guest: out 0x3f8,al ─► KVM_EXIT_IO  ─► vcpu.run() returns IoOut{port:0x3f8,data}
                                        └─► serial.rs writes byte to stdout ─┐
                       ◄──────────────────  re-enter vcpu.run() ◄────────────┘
guest: hlt          ─► KVM_EXIT_HLT ─► break loop ─► stats.rs prints summary
```

## 7. Error handling

- **`/dev/kvm` access is the #1 real-world stumble.** On open failure, emit a clear,
  actionable message ("is /dev/kvm present? are you in the kvm group?") rather than a raw
  errno.
- All ioctl results propagate via `anyhow::Result`.
- Unknown/unhandled VM-exit reasons are fatal with a diagnostic, not ignored — this
  prevents silent infinite loops.

## 8. Testing

`/dev/kvm` logic can't run without the device, so:

- **Unit tests** for pure functions: arg parsing, stats aggregation.
- **One integration test** runs the VMM against the committed `hello.bin` and asserts
  stdout contains `hello from guest` and the stats show `hlt=1`. The test auto-skips when
  `/dev/kvm` is absent (e.g. if run on Windows).
- Implementation follows TDD for the pure logic; the KVM path is verified via the
  integration test plus a manual `--trace` run.

## 9. Out of scope (deferred)

- Long mode, paging, GDT/IDT, the real boot stub (Month 2).
- Full 16550 UART emulation (Month 2).
- The Rust `no_std` guest kernel, syscalls, user/kernel transition (Month 2).
- C userspace, ELF loading, benchmarks (Month 3).
- Multiple vCPUs/SMP, networking, virtio, disk images, filesystems (explicitly out of
  scope for the whole project per the plan's "Scope Discipline").

## 10. Open questions

None blocking. Exact guest load address (`0x1000`) and initial register values are
implementation details to be confirmed against the LWN/rust-vmm reference during the
implementation plan.
