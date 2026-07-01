# MiniKVM

A from-scratch KVM-based microVM stack in Rust, C, and x86-64 assembly.

## What

A Rust host VMM that opens `/dev/kvm`, allocates guest physical memory, and
drives a single vCPU through a `KVM_RUN` loop, handling IO/HLT/MMIO exits.
The vCPU boots a custom guest kernel: a hand-written 16-bit boot stub
(`guest/boot.asm`) climbs real → protected → long mode, hands off to a
freestanding Rust kernel (`crates/kernel/`) that builds its own page tables,
IDT, GDT/TSS with IST, a bump-allocator heap, and the SYSCALL/SYSRET path.
The kernel embeds a freestanding C program (`user/hello.c`), parses its ELF
at runtime, and dispatches it as ring 3 — where it makes real `write` and
`exit` syscalls back into the kernel through the assembly entry stub.

End-to-end ~1500 LoC across three languages, each load-bearing.

## Why

- Forces understanding of every layer — host VMM, KVM API, guest boot path,
  paging, IDT, SYSCALL machinery, ELF loading, ring transitions. You can't
  fake any of them.
- Each language has a load-bearing role — assembly for entry stubs and IDT
  trampolines, Rust for VMM + guest kernel logic, C for freestanding
  userspace. Nothing is bolted on.
- Covers OS internals AND virtualization — a rarer combination than either
  alone. Maps to real production work (Firecracker, Cloud Hypervisor).

## Quick start

```bash
# Ubuntu 24.04 / WSL2 with /dev/kvm present:
sudo apt install build-essential nasm

# Ensure your user is in the `kvm` group (needed for /dev/kvm without sudo):
getent group kvm | grep -q "$USER" || { sudo usermod -aG kvm "$USER"; echo "log out and back in"; }

# rustup + bare-metal target:
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup target add x86_64-unknown-none

# Build and run:
make
cargo run -p vmm -- run guest.img --trace
```

## Demo output

```
[host] created VM
[host] in-kernel irqchip + PIT created
[host] mapped 64 MiB guest memory
[host] created vCPU 0
[guest] kernel entered
[guest] paging enabled
[guest] idt loaded
[guest] heap initialized (65536 bytes)
[guest] heap demo: Vec<u32>={0,1,2,3,4} Box<u64>=0xDEADBEEF
[guest] gdt+tss installed
hello from the kernel (long mode)
[guest] syscall enabled
[guest] host uptime via MMIO: 4409984 ns
[guest] interrupts: PIC remapped (IRQ0 masked), PIT at 1000 Hz
[guest] loaded /bin/hello
[guest] entering ring 3
[user] hello from C userspace
[guest] user exited (code 0)
[guest] handled 36 timer ticks; first ring-3 RIPs preempted: 0x80002c 0x8000e6 0x8000e6
[host] guest powered off via MMIO
[host] VM exits: io=1307, hlt=0, mmio=2
[host] avg syscall latency: 63 ns
[host] runtime: 49.059117ms
```

The `RIPs preempted` line is the timer-interrupt proof: those are **ring-3
user addresses** the PIT's IRQ0 interrupted. `0x80002c` is the `ret` of the
`preempt()` syscall stub (the tick that was pending when IRQ0 got unmasked);
`0x8000e6` is inside the busy-spin loop. Each was running at `CPL=3`, so the
timer switched `ring3 → ring0` via `TSS.RSP0`, ran the handler, and `iretq`ed
back. The handler only *records* these RIPs (it stays short); `sys_exit` prints
them. See **Timer interrupts** below for how the benchmark stays tick-free while
the spin loop gets preempted.

`avg syscall latency` measures the full SYSCALL/SYSRETQ round-trip through
the kernel's asm entry stub, the Rust dispatch table, and back to ring 3
— averaged over 10000 noop syscalls bracketed by a multi-byte protocol the
host serial path decodes. M will vary by hardware (~50–500 ns typical).

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Linux host process (this binary)                               │
│                                                                 │
│  crates/vmm  (Rust, hosted)                                     │
│   ├── opens /dev/kvm via kvm-ioctls                             │
│   ├── creates VM, maps 64 MiB guest memory                      │
│   ├── creates vCPU, sets initial registers                      │
│   ├── KVM_RUN loop ── handles IO/HLT/MMIO exits                 │
│   ├── DLAB-aware 16550 UART model → stdout                      │
│   └── benchmark protocol (ESC B 0/1 + LE iters) → avg latency   │
│                                                                 │
└──────────────────────┬──────────────────────────────────────────┘
                       │ /dev/kvm + KVM_RUN
              ┌────────▼─────────┐
              │  KVM (kernel)    │ VMX hardware virtualization
              └────────┬─────────┘
                       │ guest physical memory
┌──────────────────────▼──────────────────────────────────────────┐
│  Guest (single vCPU, 64 MiB RAM, identity-mapped)               │
│                                                                 │
│  guest/boot.asm                  ← 16-bit real → 64-bit long    │
│   ├── lgdt (bootstrap GDT)                                      │
│   ├── CR0.PE, CR4.PAE, CR3, EFER.LME, CR0.PG                    │
│   ├── enable SSE                                                │
│   └── jmp 0x2000 (kernel entry)                                 │
│                                                                 │
│  crates/kernel  (Rust no_std, no_main)                          │
│   ├── paging       4-level identity map, 32× 2 MiB huge pages   │
│   ├── idt          256-entry IDT, 32 exception trampolines      │
│   ├── heap         64 KiB BSS, bump allocator + alloc crate     │
│   ├── gdt          kernel GDT + TSS + IST1 stack for #DF        │
│   ├── syscall      IA32_STAR/LSTAR/FMASK MSRs + asm entry stub  │
│   ├── elf          ELF64 PT_LOAD parser + loader                │
│   ├── user         embedded ring-3 ELF + iretq dispatch         │
│   ├── serial       polling 16550 UART driver (writeln! target)  │
│   └── io           inb/outb/rdmsr/wrmsr helpers                 │
│                                                                 │
│  Ring 3 (DPL=3, in PD[4] = 8–10 MiB user region, U/S=1)         │
│   stack top at 0x9FFFF0 (last 16 B inside PD[4];                │
│                          0xA00000 sits in PD[5], supervisor)    │
│                                                                 │
│  user/hello.c + user/crt0.s     ← freestanding C, no libc       │
│   ├── benchmark protocol emitter (ESC B + LE iters; 10000 ops)  │
│   ├── write(1, "[user] hello from C userspace\n", 30)           │
│   └── exit(0)                                                   │
└─────────────────────────────────────────────────────────────────┘
```

## GDB debugging

The `--gdb` flag opens a GDB Remote Serial Protocol stub on
`127.0.0.1:1234` (override with `--gdb-port N`). The VMM blocks waiting
for a client; once gdb attaches it stops the vCPU before the first guest
instruction.

Pre-flight (one-time per build):
```bash
make sanity
```
That confirms `build/hello.elf` carries DWARF and the kernel ELF links
`.text` at `0x2000` (so gdb's symbol addresses match the GPA where the
boot stub jumps).

Demo session:
```bash
# Terminal 1
cargo run -p vmm -- run guest.img --gdb
# [host] gdb stub listening on 127.0.0.1:1234 (waiting for client)

# Terminal 2 (run from the workspace root so `list` finds source paths)
gdb
(gdb) file target/x86_64-unknown-none/debug/kernel
(gdb) add-symbol-file build/hello.elf        # NOTE: no address
(gdb) target remote :1234
(gdb) b kernel::syscall::rust_syscall_dispatch
(gdb) c
... breaks inside the kernel's syscall dispatcher
(gdb) info registers rip rsp cs
(gdb) p $rax                 # syscall number
(gdb) c                      # continue; repeat to step through the noop syscalls
(gdb) detach                 # hand the vCPU back; the guest runs to completion
```

Stock `gdb` (Ubuntu 24.04 ships 15.1) works — `gdb-multiarch` is
unnecessary because host and guest are both x86_64.

Notes:
- `add-symbol-file build/hello.elf` takes **no address**: hello.elf is an
  EXEC linked at `0x800000` already; passing `0x800000` would double it.
- gdb prints `warning: Architecture rejected target-supplied description`
  on connect. This is **harmless**: the stub serves a minimal 24-register
  description (16 GPRs + rip + eflags + 6 segment selectors) that gdb
  declines, falling back to its built-in `i386:x86-64` layout — which is
  byte-compatible with our register packet, so every register reads and
  writes correctly.
- Software breakpoints are stub-managed: gdb's `Z0` makes the stub write
  `0xCC` (saving the original byte) and `KVM_GUESTDBG_USE_SW_BP` traps the
  int3. KVM rewinds RIP to the breakpoint address itself, so the stub
  reports it unchanged (no manual fixup needed).
- Stepping (`si`) over a `SYSCALL` appears atomic — the next stop is the
  first user instruction after `SYSRET`, because `IA32_FMASK` clears the
  trap flag on ring-0 entry. To step into the kernel handler, set a
  breakpoint on `kernel::syscall::rust_syscall_dispatch` instead.

Async Ctrl-C break, hardware breakpoints, and watchpoints are deferred to
follow-on slices.

## MMIO device

Alongside the PIO serial port, the VMM emulates a tiny **memory-mapped I/O**
device at guest-physical `0x04000000` — the first address past the 64 MiB of
RAM. Because no KVM memslot backs that address, any guest access traps as
`KVM_EXIT_MMIO` and the VMM services it (`crates/vmm/src/mmio.rs`). Two
registers:

- **`0x00` (read)** — host uptime in nanoseconds since the VMM started.
- **`0x08` (write)** — writing a magic value powers the VM off (the guest's
  normal termination, replacing `hlt`).

The guest maps the device with one uncacheable page-table entry
(`paging::map_mmio_page`) — uncacheable because a device register changes
outside the CPU's knowledge, so a cached read would see a frozen clock. This
is the same unbacked-address → trap → emulate mechanism every real device
(NIC, disk, interrupt controller) uses; the serial port is its PIO sibling.

## Timer interrupts

Every exit so far is *guest-caused* — an IO/MMIO access or a syscall the guest
chose to make. A timer interrupt is the first **asynchronous** event: the
outside world preempting the guest mid-instruction. This is the mechanism a
real OS uses to take the CPU back from a runaway program.

**Host side.** One ioctl pair, `setup_irqchip` (`crates/vmm/src/vm.rs`):

- `KVM_CREATE_IRQCHIP` puts the legacy interrupt controllers — 8259 PIC,
  IOAPIC, and per-vCPU LAPIC — *inside* KVM.
- `KVM_CREATE_PIT2` adds an in-kernel 8254 PIT.

With both in-kernel, KVM injects IRQ0 on the right vCPU automatically; the
`KVM_RUN` loop needs **no** change to deliver interrupts. The price is that an
in-kernel LAPIC also changes `hlt`: a halted vCPU now waits in-kernel for an
interrupt instead of exiting to userspace. That is why the Month-1 real-mode
blob (which ends in `hlt` with interrupts off) must run with `--no-irqchip`.

**Guest side** (`crates/kernel/src/interrupts.rs`):

1. **Remap the PIC.** The 8259 defaults to vectors 0x08–0x0F, which collide
   with the CPU's exception vectors. The ICW1–ICW4 init sequence moves IRQ0–7
   to 0x20–0x27 (so the timer is vector 32), then OCW1 masks *every* line —
   including IRQ0. The timer stays masked until ring 3 explicitly opts in (see
   "Preempting ring 3" below).
2. **Program the PIT.** Mode 2 (rate generator), divisor =
   `1193182 / 1000` → a ~1 kHz tick. The divisor math is a pure function,
   re-derived host-side in `crates/vmm/tests/pit_divisor.rs`.
3. **IDT gate 32.** `idt_stubs.s` gained 16 IRQ trampolines (vectors 32–47)
   that reuse the same `isr_common` save/restore path the exception stubs use;
   its epilogue (`pop`s, `add rsp,16`, `iretq`) was already built to *return*,
   which is exactly what an IRQ handler must do.
4. **Handle + EOI.** `handle_timer` bumps an `AtomicU64`, records the first few
   interrupted RIPs, and sends the PIC end-of-interrupt (`out 0x20, 0x20`) —
   without the EOI the PIC never delivers IRQ0 again. It does **no** serial I/O
   (see the top-half note below); `sys_exit` prints the RIPs.

**Preempting ring 3 (two gates, not one).** Whether a tick reaches the CPU
depends on *two* independent switches: the `IF` flag (per-context) and the PIC
mask (per-IRQ-line). This kernel keeps them separate so it can preempt user
code without disturbing the syscall benchmark:

- `enter_ring3` hands off with **`IF=1`** (`rflags = 0x202`), so ring 3 is
  preemptible. This is the *only* place interrupts get enabled — the kernel
  itself never runs `sti`; the `iretq` into ring 3 is what turns `IF` on, right
  at the privilege boundary.
- But IRQ0 is **masked** at the PIC through boot *and* the benchmark, so nothing
  actually fires. The 10 000-syscall latency loop runs interrupt-free and its
  measurement stays clean.
- After the benchmark, the user program calls **`SYS_PREEMPT`** (syscall 3),
  whose handler unmasks IRQ0 (`out 0x21, 0xFE`). The user code then busy-spins,
  and now every ~1 ms a timer tick lands *in that ring-3 loop*.

Because the tick arrives at `CPL=3`, the CPU performs a privilege switch: it
loads the kernel stack from **`TSS.RSP0`** (set up back in the GDT/TSS slice,
exercised for real here), vectors through IDT gate 32, runs `handle_timer`,
EOIs, and `iretq`s back to ring 3 with `IF` restored. The recorded RIPs are
user-space addresses — the first (`0x80002c`) is the `preempt()` `ret` the
pending tick caught on the way out of the syscall; the rest (`0x8000e6`, …) are
inside the busy-spin loop. Those addresses being in the user program is the
end-to-end proof that the timer genuinely preempted ring 3.

**Why the handler prints nothing.** An interrupt handler must be *short*. The
first version printed the RIP from inside `handle_timer`, but serial output is
many COM1 VM-exits — slower than the 1 kHz tick — so the handler kept getting
re-interrupted at the *same* return address before it could finish, and every
"sample" was that one `ret`. Splitting it into a fast top half (record the RIP,
EOI) and a bottom half (`sys_exit` prints them) fixes that: the handler returns
immediately, so ticks 2+ land in the spin loop where the work actually is. The
total is reported too (~tens of ticks; wall-clock-dependent).

## Language roles

| Language       | Use                                                  | Files |
|----------------|------------------------------------------------------|-------|
| Assembly       | Real→long-mode boot, IDT stubs, SYSCALL stub, crt0   | `guest/boot.asm`, `crates/kernel/src/idt_stubs.s`, `crates/kernel/src/syscall_entry.s`, `user/crt0.s` |
| Rust (hosted)  | VMM host (opens /dev/kvm, KVM_RUN loop, UART model)  | `crates/vmm/` |
| Rust (no_std)  | Guest kernel logic above the asm stubs               | `crates/kernel/` |
| C              | Freestanding ring-3 userspace                        | `user/hello.c` |

## The slice-by-slice journey

Each slice was its own design doc → branch → incremental commits → merge.
The "at-merge demo line" was the load-bearing assertion each slice added;
some were superseded by later slices.

- **M1 — Host VMM + raw 16-bit guest** → `hello from guest`
- **M2 slice 1 — Boot stub + long mode + Rust kernel** → `[guest] paging enabled`
- **M2 slice 2 — IDT + 32 CPU exception handlers** → `[guest] EXCEPTION 6 (#UD ...)`
- **M2 slice 3 — Bump allocator + `alloc` crate** → `[guest] heap demo: Vec<u32>={0,1,2,3,4} Box<u64>=0xDEADBEEF`
- **M2 slice 4a — GDT + TSS + `#DF` on IST** → `[guest] gdt+tss installed`
  *(at-merge marquee was `EXCEPTION 8 (#DF Double Fault)`; the provocation
  was replaced by slice 4b's ring-3 entry sequence.)*
- **M2 slice 4b — SYSCALL/SYSRET + ring 3** → `[guest] entering ring 3`
  *(at-merge marquee was `hello from ring 3` written by an asm program;
  slice 5 replaced the asm program with the C version.)*
- **M3 slice 5 — C userspace + ELF loading** → `[user] hello from C userspace`
- **M3 slice 6 — Benchmark + this README** → `[host] avg syscall latency: M ns`
- **Slice 7 — GDB stub (post-PDF Tier C)** → `[host] gdb stub listening on 127.0.0.1:1234`
- **Slice 8 — MMIO device emulation** → `[guest] host uptime via MMIO: N ns` + poweroff
- **Slice 9 — Timer interrupts (in-kernel irqchip + PIT)** → `[guest] timer tick 1 (interrupted rip=0x…)` (fired in kernel mode)
- **Slice 10 — Ring-3 preemption (PIC-mask gate + `SYS_PREEMPT`)** → `[guest] timer tick 1 (interrupted rip=0x80002c)` (a user address — real preemption + `TSS.RSP0`)

## Key lessons learned

A few of the load-bearing gotchas that took the most time and would have
been invisible without slice 2's IDT or slice 4a's IST catching them:

- **The SYSRETQ +16/+8 derivation.** `IA32_STAR[63:48]` is the SYSRET *base*,
  not the user CS directly. `SYSRETQ` computes `CS = base+16` and `SS = base+8`
  (both with RPL forced to 3). With our GDT ordering of user CS32/SS/CS64 at
  0x28/0x30/0x38, setting the base to `0x28` lands user CS at `0x38` and SS
  at `0x30`. Setting it to "the obvious" `0x38` would land CS at `0x48` (past
  the user slots, in the TSS descriptor) and `#GP`.
- **LLVM Intel-syntax `mov reg, sym` is a memory load, not an immediate.**
  Even after `.equ msg_len, ...`, `mov rdx, msg_len` assembles as
  `mov rdx, [msg_len]` — a memory read from address 18. Fix:
  `mov rdx, OFFSET msg_len`. Without slice 2's IDT, this would have been a
  silent ring-3 triple-fault; with it, the surface area was a precise
  `EXCEPTION 14 (#PF) at rip=0x800015 err=0x5`.
- **GAS vs LLVM asm comment characters.** GNU `as` driving a lowercase
  `.s` file accepts only `#` for line comments — neither `//` nor `/* */`
  works (those are LLVM-integrated-assembler extensions the kernel's
  `global_asm!` blocks rely on). Worth knowing when porting asm between
  the two contexts.
- **Ubuntu gcc 13 freestanding defaults.** `-fno-pic -fno-pie
  -mno-red-zone -fno-stack-protector -fno-stack-clash-protection
  -fcf-protection=none` — every flag is fighting a default-on behavior.
  The CET (`endbr64` at every function prologue) was the most surprising.
- **The slice-4a kernel-CS-selector preservation.** The "textbook" SYSCALL
  GDT layout would move kernel CS to `0x08`, but slices 1–3 already run
  with `CS=0x18` and the IDT gates hardcode that selector. The fix was
  keeping kernel CS/SS at boot.asm's `0x18`/`0x20` and appending user
  segments after. Caught at runtime by the first interrupt after the GDT
  swap silently triple-faulting.
- **`include_bytes!` path resolution.** `CARGO_MANIFEST_DIR =
  crates/kernel/`; `../../build/hello.elf` reaches the workspace root.
  Two `../`, not three.
- **The 16-byte alignment contract at syscall dispatch.** SysV requires
  the call site to be 16-byte aligned. The asm stub pushes exactly 6
  qwords (48 bytes = 3×16) so if the kernel stack top is 16-aligned, the
  Rust dispatcher's prologue sees the right alignment. The dedicated
  syscall stack is `#[repr(C, align(16))]` for this reason.
- **An in-kernel LAPIC silently changes `hlt`.** `KVM_CREATE_IRQCHIP` was a
  prerequisite for timer-interrupt delivery, but it also means a halted vCPU
  blocks in-kernel waiting for an interrupt instead of returning
  `KVM_EXIT_HLT`. The Month-1 real-mode blob ends in `hlt` with `IF=0` and no
  IDT, so once the irqchip became the default it hung forever — surfaced as a
  70-second test timeout. The fix was a `--no-irqchip` opt-out for that legacy
  guest; the lesson is that one ioctl quietly rewires an instruction three
  layers away.

## References

- **OSDev Wiki** — [osdev.org](https://wiki.osdev.org/) — x86_64 boot path,
  paging, GDT, IDT.
- **Phil Oppermann "Writing an OS in Rust"** — [os.phil-opp.com](https://os.phil-opp.com/)
- **AMD64 APM Vol 2** — SYSCALL/SYSRET semantics, `STAR` field math.
- **Intel SDM Vol 3A** — IDT, paging, MSRs.
- **rust-vmm crates** — [github.com/rust-vmm](https://github.com/rust-vmm) —
  `kvm-ioctls`, `kvm-bindings`, `vm-memory`.
- **Linux KVM API docs** — [kernel.org/doc/html/latest/virt/kvm/api.html](https://kernel.org/doc/html/latest/virt/kvm/api.html)
- **Firecracker** — [github.com/firecracker-microvm/firecracker](https://github.com/firecracker-microvm/firecracker)

## Status

Solo learning project, written May–June 2026. Pace was by milestones, not
the calendar. Not production code.

**Scope discipline** (locked from the start): single vCPU, no networking,
no disk by design. Per-slice design rationale lives in a private notes
tree and is not included in this clone.

The repo directory is named `KVM/` for legacy reasons; the project name
everywhere else is **MiniKVM** (mixed case).
