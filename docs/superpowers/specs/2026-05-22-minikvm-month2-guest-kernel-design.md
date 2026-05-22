# MiniKVM — Month 2 (Slice 1): Boot Stub + Long Mode + Rust Kernel (Design)

- **Date:** 2026-05-22
- **Status:** Approved (design); pending implementation plan
- **Scope:** The first slice of Month 2 — get a Rust `no_std` kernel running in 64-bit long
  mode and printing over serial. Corresponds to the project plan's **weeks 5–7** (boot
  stub + paging), explicitly the "hardest phase."

## 1. Context & Scope

Month 1 delivered a host VMM that boots a 16-bit real-mode blob, handles IO/HLT exits, and
prints `hello from guest`. Month 2 makes the guest "feel like a real tiny OS." The project
plan's Month 2 bundles six subsystems (serial driver, paging, IDT/traps, kernel heap,
syscall dispatch, user/kernel transition). That is too large for one spec, so we
**decompose** it. This spec is **slice 1**: the boot path and paging — the documented stall
point. The remaining items (IDT, heap, syscalls, user mode) become follow-on specs.

We pace by **milestones, not the calendar**.

### Environment note

The plan warns that "WSL2 does not reliably expose `/dev/kvm`." This does **not** apply to
us: `/dev/kvm` is present and Month 1 ran for real on this box (WSL2 Ubuntu, kernel 6.6.x,
user in the `kvm` group). All build/run/test happens **inside WSL** — the Windows side has
no `/dev/kvm` and the wrong toolchain.

## 2. Goal

A single `guest.img` that boots in real mode, climbs real → protected → long mode via a
hand-written assembly stub, hands off to a Rust `no_std` kernel which builds a full 64 MiB
identity-mapped page table and prints over a polled 16550 UART.

### Done criteria

```
$ minikvm run guest.img --trace
[host] created VM
[host] mapped 64 MiB guest memory
[host] created vCPU 0
[guest] kernel entered
[guest] paging enabled
hello from the kernel (long mode)
[host] VM exits: io=N, hlt=1, mmio=0
[host] runtime: ...
```

Without `--trace`: only the three guest lines print (serial output is always shown; only the
`[host]` lines are gated by `--trace`). The io count `N` is not asserted anywhere — polling
makes it inherently variable; we assert on output strings instead.

## 3. Approach (decided)

**Approach A — guest-side asm boot ladder.** The vCPU starts in 16-bit real mode (the x86
reset state, as in Month 1). A hand-written assembly stub builds a GDT and page tables *in
the guest*, climbs to long mode, and calls into the Rust kernel. This matches the plan's
Language Role table ("Assembly — entry stubs, real→long mode") and concentrates the new
learning in one place while leaving the host VMM almost unchanged.

Rejected alternatives: host-side long-mode bring-up (production-style, but skips the asm
ladder and contradicts the language roles) and a hybrid (host starts in protected mode).

## 4. Architecture

A new sibling crate joins the workspace:

```
crates/
├── vmm/          # Month-1 host VMM, extended: fuller 16550 UART
└── kernel/       # NEW: no_std + no_main Rust kernel, target x86_64-unknown-none
guest/
├── hello.asm     # Month-1 blob (kept for its integration test)
├── boot.asm      # NEW: 16-bit → 64-bit boot stub (NASM)
└── ...
Makefile          # NEW: builds guest.img (nasm + cargo + objcopy + pad/concat)
```

### Packaging: a single `guest.img`

The CLI stays `minikvm run guest.img --trace` (matching both the plan and Month 1's
positional form). `guest.img` is the concatenation of:

1. `boot.bin` — the 16-bit NASM stub, **padded to a fixed 4 KiB**.
2. `kernel.bin` — the Rust kernel built for `x86_64-unknown-none`, then `objcopy -O binary`.

Because the stub is padded to a fixed 4 KiB, the kernel always begins at a round offset, so
the "address contract" is just two constants — the image loads at `0x1000`; the kernel entry
is at `0x2000` — named identically in the stub's far-jump target, the kernel's linker-script
base, and (implicitly) the VMM's load address. We prefer this over a single `ld` link of
asm+Rust objects: the unified link is more elegant but front-loads object-format/`build.rs`
glue that obscures the OS concepts; fixed-pad concatenation keeps each language's build
separately understandable while still yielding one image.

## 5. Guest physical memory layout

Three constraints drive the addresses: (1) **real-mode reach** — anything the stub touches
before long mode must be < 1 MiB; (2) **4 KiB alignment** for all paging structures; (3) the
stub's minimal bootstrap map must cover the early stack and the kernel it jumps to.

```
GPA            Size      Contents                          Set up by / notes
-----------    -------   -------------------------------   ---------------------------
0x00000        4 KiB     real-mode IVT / BIOS data area    leave untouched
0x01000        4 KiB     boot.bin (stub) + GDT             RIP starts here (real mode)
0x02000        ~568 KiB  kernel.bin: .text/.rodata/        entry _start at 0x02000;
               (budget)  .data/.bss (incl. full tables)    kernel is tiny (KiBs)
0x90000        12 KiB    bootstrap tables PML4/PDPT/PD      built by asm; <1 MiB, aligned
0x100000       —         kernel stack TOP (grows down)     RSP set by stub; within 2 MiB map
...
0x4000000      —         end of 64 MiB guest RAM           all identity-mapped after stage 2
```

- **GDT** lives inside `boot.bin` (a handful of descriptors); no separate region.
- **COM1 is not on this map** — serial is port-mapped IO (`in`/`out` to `0x3f8`), never a GPA.
- **Guest RAM starts zeroed** (KVM's anonymous mmap), so the kernel's `.bss` — including the
  page-table statics — is pre-zeroed; no loader clears it.

### Two-stage paging split

Matching the plan's demo ordering (`[guest] kernel entered` → `[guest] paging enabled`):

- **Stage 1 — bootstrap (asm):** identity-map the **first 2 MiB** with a single 2 MiB huge
  page (three tables: PML4→PDPT→PD, one live entry each) — just enough to enter long mode and
  reach the kernel + stack.
- **Stage 2 — full map (Rust kernel):** the kernel builds its own 4-level tables identity-
  mapping all **64 MiB** (32 × 2 MiB huge pages in one PD), loads `CR3`, and prints
  `[guest] paging enabled`. Bootstrap tables sit at a fixed low address; the full tables are
  4 KiB-aligned `static`s in the kernel's BSS (the linker assigns their address, which — being
  identity-mapped — is the physical address loaded into `CR3`).

## 6. The boot stub: real → protected → long mode

Starting state is unchanged from Month 1: real mode, `CS/DS/ES/SS` base 0, `RIP = 0x1000`,
`RFLAGS` bit 1 set, `KVM_SET_TSS_ADDR` set by the host. Interrupts stay **off** for the
entire slice (no IDT yet).

**Phase A — real mode (16-bit):**
1. `cli`.
2. `lgdt [gdt_ptr]` — load the GDT (in `boot.bin`, < 1 MiB).
3. set `CR0.PE = 1` (protected enable).
4. far-jump to the 32-bit code selector to reload `CS` → 32-bit protected mode.

**Phase B — protected mode (32-bit, flat 4 GiB addressing):**
5. reload data segments (32-bit data selector); set a temporary `ESP`.
6. build the stage-1 tables at `0x90000` (PML4[0]→PDPT, PDPT[0]→PD, PD[0] = 2 MiB huge page;
   only the live entries are written — memory is already zero).
7. `CR4.PAE = 1` (long mode requires PAE).
8. load `CR3 = 0x90000`.
9. `EFER.LME = 1` (MSR `0xC000_0080`, bit 8) via `rdmsr`/`wrmsr` — arms long mode.
10. `CR0.PG = 1` — enables paging; CPU enters IA-32e (compatibility sub-mode until `CS` reload).
11. far-jump to the 64-bit code selector (L-bit set) → true 64-bit long mode.

**Phase C — long mode (64-bit):**
12. reload data segments (64-bit data selector); set `RSP = 0x100000`.
13. **enable SSE** — clear `CR0.EM`, set `CR0.MP`, set `CR4.OSFXSR` + `CR4.OSXMMEXCPT`. The
    Rust compiler can emit SSE instructions; executing one with SSE disabled faults. Enabling
    it here (≈4 instructions) lets the kernel use the stock target regardless of its float
    settings.
14. jump to the kernel at `0x2000` (`mov rax, 0x2000; jmp rax`).

**GDT contents:** null, 32-bit code, 32-bit data (Phase B), 64-bit code (L-bit, Phase C),
64-bit data — all base 0 / max limit (segmentation is vestigial; paging does protection).
Selectors are byte-offsets (`0x08`, `0x10`, `0x18`, `0x20`).

**A20 line:** on real hardware you'd enable A20 before addressing > 1 MiB. Under KVM there is
no A20 gate in our path, so we skip it — to be verified empirically and noted in the dev log.

## 7. The Rust kernel

Crate `crates/kernel`, `#![no_std]` (no OS beneath us — `core` only) + `#![no_main]` (the stub
jumps to our entry directly), built for `x86_64-unknown-none`.

- **Entry:** `#[no_mangle] pub extern "C" fn _start() -> !`. The linker script sets
  `ENTRY(_start)`, links at base `0x2000`, and places `_start` first so it is the first byte
  of `kernel.bin`. It never returns; it `hlt`-loops when done (→ `KVM_EXIT_HLT`).
- **`#[panic_handler]`:** prints the panic over serial (if up) and halt-loops. Without an IDT
  this is our primary diagnostic.
- **`_start` sequence:** init UART → print `[guest] kernel entered` → build full page tables +
  load `CR3` → print `[guest] paging enabled` → print `hello from the kernel (long mode)` →
  halt.
- **Serial driver (polling 16550):** registers are offsets from `0x3f8` (`+0` data, `+1` IER,
  `+2` FCR, `+3` LCR, `+4` MCR, `+5` LSR). Transmit polls LSR bit 5 (`0x20`, THR-empty) before
  writing the byte. Wrapped in a `Serial` type implementing `core::fmt::Write` for formatted
  output. Runs the classic init sequence (mask IER, set DLAB + baud divisor, 8N1, enable+clear
  FIFO, set MCR) — the emulated UART ignores the divisor but the sequence exercises the host.
- **Full paging:** 4 KiB-aligned BSS statics `PML4`/`PDPT`/`PD`; `PD[0..32]` =
  `(i*2MiB) | PRESENT | WRITABLE | HUGE`; then `mov cr3, &PML4`. The identity map keeps the
  running code (`0x2000`) and stack (`0x100000`) valid across the `CR3` switch. No frame
  allocator, no 4 KiB pages, no heap (later specs).
- **Privileged instructions** (`in`/`out`, `mov cr3`, MSR access) use `core::arch::asm!`. These
  are unavoidable single-instruction accesses, not general logic — consistent with the
  language-role rules.

## 8. Host VMM changes

- **vCPU init & loading: unchanged in shape.** Still real mode at `RIP = 0x1000`, still
  `KVM_SET_TSS_ADDR`, still copies the image to `0x1000`. The VMM need not know about `0x2000`.
- **`[guest]` lines need no plumbing** — they are serial bytes flowing through the existing IO
  path; `[host]` lines remain gated by `--trace`; the two interleave.
- **Fuller 16550 emulation in `serial.rs`** (the one real change). Month 1 only handled `out`
  of data. Now:
  - `in` from LSR (`0x3f8+5`) → return `0x60` (THR-empty + transmitter-empty) so polling proceeds.
  - `in` from other UART ports → `0`.
  - `out` to data port (`0x3f8`) → stdout **only when `DLAB = 0`**.
  - `out` to LCR (`0x3f8+3`) → record the **DLAB** bit; other register writes → accept + ignore.

  DLAB tracking is a correctness requirement: during init the driver writes a baud divisor to
  `0x3f8` with DLAB set; blind forwarding would print it as garbage. `serial.rs` thus grows a
  tiny register-state model (it tracks DLAB), still far from a full UART.
- **Stats:** unchanged counters; the io count is no longer a clean constant and is not asserted.

## 9. Build orchestration

`cargo build` alone cannot produce `guest.img`; a **Makefile** drives the pipeline: NASM →
`boot.bin`; `cargo build -p kernel --target x86_64-unknown-none` → kernel ELF;
`objcopy -O binary` → `kernel.bin`; pad + concat → `guest.img`. We **commit `guest.img`** so the
integration test has a fixed artifact (as Month 1 committed `hello.bin`), with the regenerate
command documented.

Two decisions keep us on **stable Rust** (no nightly): build with an explicit
`--target x86_64-unknown-none` (available via `rustup target add`, no `-Zbuild-std`); and
**hand-write the tiny `memcpy`/`memset`/`memmove`/`memcmp`** intrinsics (~a dozen lines) the
compiler requires, rather than pulling them via `build-std-features` — which also makes the
requirement visible.

## 10. Testing

- **Host unit tests** (Month-1 pattern) for the new pure logic in `serial.rs`: LSR read returns
  ready bits; data write forwards with DLAB=0 and is suppressed with DLAB=1; LCR write updates
  DLAB. Plus any pure kernel helpers extracted as `const fn` (e.g. a page-directory-entry
  computation).
- **End-to-end integration test** (like `run_guest.rs`): run the VMM against `guest.img`;
  assert stdout **contains** `[guest] kernel entered`, `[guest] paging enabled`,
  `hello from the kernel`, and `hlt=1`. Auto-skips without `/dev/kvm`. Substring assertions,
  not exact counts.
- **Boot-stub debugging technique (load-bearing):** with no IDT, a stub mistake is a silent
  triple-fault → CPU reset → hang. During bring-up the stub `out`s a **marker byte after each
  phase** (e.g. `1` in protected mode, `2` in long mode, `3` before the kernel jump). The last
  byte seen pinpoints the failing transition. Markers are removed once the path is stable. This
  is the single technique that prevents the plan's "lose 1–2 weeks here" outcome.

## 11. Risks / known gotchas

- **Triple-fault on any boot-stub error** (mitigated by phase-marker bytes, §10).
- **DLAB/divisor corruption** if the host forwards `0x3f8` writes blindly (handled, §8).
- **Missing `memcpy` & friends** — hand-written intrinsics (§9).
- **SSE not enabled** before Rust runs → fault; enabled in the stub (§6.13).
- **A20** — assumed unnecessary under KVM; verify (§6).
- **Multi-target workspace build** — the kernel builds for `x86_64-unknown-none` while the vmm
  builds for the host; orchestrated explicitly via the Makefile (§9).
- **kvm-bindings/kvm-ioctls version pin** — carried over from Month 1.

## 12. Out of scope (deferred to later specs)

IDT / trap & exception handlers; kernel heap / frame allocator; syscall dispatch
(SYSCALL/SYSRET); user/kernel privilege transition; C userspace and ELF loading. (Per the
plan's Month-2 weeks 8–10 and Month 3.)

## 13. References

- **OSDev Wiki** (osdev.org) — x86_64 boot path, GDT, paging: the canonical architecture reference.
- **Phil Oppermann, "Writing an OS in Rust"** (os.phil-opp.com) — freestanding `no_std` kernels,
  paging, linker setup: the best single resource for the kernel phase.
- **Linux KVM API docs** (kernel.org/doc/html/latest/virt/kvm/api.html) — exit types, register formats.
- **rust-vmm** (github.com/rust-vmm) — `kvm-ioctls`, `kvm-bindings`, `vm-memory` sources.
```
