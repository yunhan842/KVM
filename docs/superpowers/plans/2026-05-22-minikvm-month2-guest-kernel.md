# MiniKVM Month-2 (Slice 1) Implementation Plan — Boot Stub + Long Mode + Rust Kernel

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a single `guest.img` whose 16-bit asm stub climbs real → protected → long mode and hands off to a `no_std` Rust kernel that builds a 64 MiB identity-mapped page table and prints `[guest] kernel entered` / `[guest] paging enabled` / `hello from the kernel (long mode)` over a polled 16550 UART.

**Architecture:** A new `crates/kernel` (`no_std`, `no_main`, target `x86_64-unknown-none`) is linked at `0x2000` and `objcopy`'d to a flat binary. `guest/boot.asm` (NASM, padded to 4 KiB, loaded at `0x1000`) does the mode transitions and jumps to `0x2000`. A `Makefile` concatenates `boot.bin ++ kernel.bin → guest.img`. The host `crates/vmm` is unchanged except `serial.rs` grows a tiny DLAB-aware 16550 model so the guest's polling driver works. Verification is incremental: the stub emits phase-marker bytes (debuggable with the *unchanged* Month-1 host) before the real serial driver exists.

**Tech Stack:** Rust (2021, stable), `core::arch::asm!`, target `x86_64-unknown-none`; NASM; `objcopy` (binutils); GNU Make. Host crates unchanged: `kvm-ioctls`, `kvm-bindings`, `vm-memory`, `anyhow`.

**Environment:** All commands run **inside WSL** from `~/projects/KVM` (the Windows side has no `/dev/kvm` and the wrong toolchain). The reference invocation used by the implementer is `wsl.exe -e bash -lc "cd ~/projects/KVM && <cmd>"` if driving from Windows.

**Address contract (named identically everywhere):**
- `0x1000` — image load address + real-mode `RIP` (set by the VMM, unchanged from Month 1).
- `0x2000` — kernel entry (boot stub padded to exactly 4 KiB → kernel starts here).
- `0x90000`/`0x91000`/`0x92000` — bootstrap PML4/PDPT/PD (built by asm).
- `0x100000` — stack top.

---

### Task 0: Scaffold the `kernel` crate (builds + objcopies to a flat binary)

The kernel can't *run* yet (it needs the stub to reach long mode), but this task makes it **build**, link at `0x2000`, and flatten to a binary we can inspect. Its `_start` raw-writes a `K` to COM1 then halts — no serial driver yet — which will let Task 1 prove the whole boot path against the unchanged Month-1 host.

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/kernel/Cargo.toml`
- Create: `crates/kernel/.cargo/config.toml`
- Create: `crates/kernel/build.rs`
- Create: `crates/kernel/link.ld`
- Create: `crates/kernel/src/main.rs`

- [ ] **Step 1: Add the kernel to the workspace, keep host commands host-only**

Replace `Cargo.toml` (workspace root) with:

```toml
[workspace]
resolver = "2"
members = ["crates/vmm", "crates/kernel"]
default-members = ["crates/vmm"]
```

`default-members` means a bare `cargo build`/`cargo test` from the root only touches `vmm` (host target) — it will not try to build the bare-metal kernel for the host. The kernel is built explicitly via its own directory (Step 6).

- [ ] **Step 2: Create the kernel manifest**

Create `crates/kernel/Cargo.toml`:

```toml
[package]
name = "kernel"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "kernel"
test = false
bench = false
```

(The `x86_64-unknown-none` target defaults to `panic-strategy = abort`, so we don't need a `panic = "abort"` profile — and we must NOT set one at the workspace root because that would break `vmm`'s test harness, which needs unwinding.)

- [ ] **Step 3: Pin the kernel's build target**

Create `crates/kernel/.cargo/config.toml`:

```toml
[build]
target = "x86_64-unknown-none"
```

This file is directory-scoped: it only applies when cargo is invoked from inside `crates/kernel`, so it never affects the host build of `vmm`.

- [ ] **Step 4: Pass the linker script (path-robust via build.rs)**

Create `crates/kernel/build.rs`:

```rust
fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg=-T{dir}/link.ld");
    println!("cargo:rerun-if-changed=link.ld");
}
```

- [ ] **Step 5: Write the linker script (entry at 0x2000)**

Create `crates/kernel/link.ld`:

```ld
ENTRY(_start)

SECTIONS {
    . = 0x2000;

    .text : {
        *(.text.boot)        /* _start goes here, first, so it lands exactly at 0x2000 */
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) }
    .data   : { *(.data .data.*) }
    .bss    : { *(.bss .bss.*) *(COMMON) }
}
```

- [ ] **Step 6: Write the minimal kernel**

Create `crates/kernel/src/main.rs`:

```rust
#![no_std]
#![no_main]

use core::panic::PanicInfo;

/// Write one byte to an I/O port.
unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val,
                     options(nomem, nostack, preserves_flags));
}

#[no_mangle]
#[link_section = ".text.boot"]
pub extern "C" fn _start() -> ! {
    // No serial driver yet: raw-write 'K' to COM1 (the Month-1 host prints it),
    // proving the boot stub reached 64-bit Rust. Replaced in Task 3.
    unsafe { outb(0x3f8, b'K'); }
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

// --- mem intrinsics the compiler may emit calls to (no libc in no_std) ---
#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n { *dest.add(i) = *src.add(i); i += 1; }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n { *dest.add(i) = c as u8; i += 1; }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if (dest as usize) < (src as usize) {
        let mut i = 0;
        while i < n { *dest.add(i) = *src.add(i); i += 1; }
    } else {
        let mut i = n;
        while i > 0 { i -= 1; *dest.add(i) = *src.add(i); }
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let (x, y) = (*a.add(i), *b.add(i));
        if x != y { return x as i32 - y as i32; }
        i += 1;
    }
    0
}
```

- [ ] **Step 7: Install the bare-metal target**

Run: `rustup target add x86_64-unknown-none`
Expected: `info: component 'rust-std' for target 'x86_64-unknown-none' installed` (or "up to date").

- [ ] **Step 8: Build the kernel and flatten it**

Run:
```bash
( cd crates/kernel && cargo build )
objcopy -O binary target/x86_64-unknown-none/debug/kernel /tmp/kernel.bin
ndisasm -b 64 -e 0 /tmp/kernel.bin | head -6
```
Expected: builds cleanly; the first disassembled instructions are the `_start` body (`mov dx,0x3f8` / `mov al,0x4b` / `out dx,al` / `hlt`), confirming `_start` is at offset 0 (→ load address `0x2000`).

- [ ] **Step 9: Verify the host build is untouched**

Run: `cargo build`
Expected: builds only `vmm` (default-members), no attempt to build `kernel` for the host.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml crates/kernel/Cargo.toml crates/kernel/.cargo/config.toml \
        crates/kernel/build.rs crates/kernel/link.ld crates/kernel/src/main.rs
git commit -m "feat(kernel): scaffold no_std kernel crate linked at 0x2000"
```

---

### Task 1: Boot stub + Makefile + first end-to-end boot

This is the milestone and the riskiest task. The stub climbs to long mode and jumps to the kernel. It emits **phase-marker bytes** (`1` after protected mode, `2` after long mode, `3` before the kernel jump) so a failed transition is diagnosable — without an IDT, a mistake is otherwise a silent triple-fault. Crucially, markers + the kernel's `K` use *raw* `out` (no LSR polling), so this runs against the **unchanged Month-1 host**.

**Files:**
- Create: `guest/boot.asm`
- Create: `Makefile`
- Modify: `.gitignore`

- [ ] **Step 1: Write the boot stub**

Create `guest/boot.asm`:

```nasm
; guest/boot.asm — Month-2 boot stub.
; Loaded at 0x1000, entered in 16-bit real mode. Climbs real -> protected -> long mode,
; enables SSE, then jumps to the Rust kernel at 0x2000.
; Phase markers '1','2','3' are emitted to COM1 for debugging (no IDT => silent faults).
; They use raw `out` (no polling), so this works with the Month-1 host. Removed in Task 3.

bits 16
org 0x1000

KERNEL_ENTRY equ 0x2000
PML4         equ 0x90000
PDPT         equ 0x91000
PD           equ 0x92000
STACK_TOP    equ 0x100000
COM1         equ 0x3f8

start:
    cli
    cld
    lgdt [gdt_desc]              ; GDT base (~0x1000) fits in 24 bits, fine in real mode

    mov eax, cr0
    or  eax, 1                   ; CR0.PE = protected mode
    mov cr0, eax
    jmp 0x08:protected           ; far jump reloads CS with the 32-bit code selector

bits 32
protected:
    mov ax, 0x10                 ; 32-bit data selector
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov esp, STACK_TOP

    mov dx, COM1                 ; marker '1' — reached protected mode
    mov al, '1'
    out dx, al

    ; stage-1 tables: identity-map first 2 MiB via one 2 MiB huge page.
    ; (guest RAM is zeroed by KVM; write only the live entries)
    mov dword [PML4], PDPT | 0x3        ; present|writable -> PDPT
    mov dword [PML4 + 4], 0
    mov dword [PDPT], PD | 0x3          ; present|writable -> PD
    mov dword [PDPT + 4], 0
    mov dword [PD], 0x83                ; present|writable|huge, phys 0
    mov dword [PD + 4], 0

    mov eax, cr4
    or  eax, 1 << 5              ; CR4.PAE (required for long mode)
    mov cr4, eax

    mov eax, PML4
    mov cr3, eax                ; point at the PML4

    mov ecx, 0xC0000080         ; EFER MSR
    rdmsr
    or  eax, 1 << 8             ; EFER.LME (long mode enable)
    wrmsr

    mov eax, cr0
    or  eax, 1 << 31            ; CR0.PG -> paging on -> enter IA-32e (compat sub-mode)
    mov cr0, eax

    jmp 0x18:long_mode         ; far jump to the 64-bit code selector

bits 64
long_mode:
    mov ax, 0x20               ; 64-bit data selector
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov rsp, STACK_TOP

    mov dx, COM1               ; marker '2' — reached long mode
    mov al, '2'
    out dx, al

    ; enable SSE (the Rust compiler may emit SSE instructions)
    mov rax, cr0
    and ax, 0xFFFB             ; clear CR0.EM (bit 2)
    or  ax, 0x2                ; set CR0.MP (bit 1)
    mov cr0, rax
    mov rax, cr4
    or  rax, (1 << 9) | (1 << 10)   ; CR4.OSFXSR | CR4.OSXMMEXCPT
    mov cr4, rax

    mov dx, COM1               ; marker '3' — about to enter the kernel
    mov al, '3'
    out dx, al

    mov rax, KERNEL_ENTRY
    jmp rax

align 8
gdt:
    dq 0x0000000000000000      ; 0x00 null
    dq 0x00CF9A000000FFFF      ; 0x08 32-bit code
    dq 0x00CF92000000FFFF      ; 0x10 32-bit data
    dq 0x00AF9A000000FFFF      ; 0x18 64-bit code (L=1)
    dq 0x00AF92000000FFFF      ; 0x20 64-bit data
gdt_end:

gdt_desc:
    dw gdt_end - gdt - 1       ; limit
    dd gdt                     ; base (32-bit)

times 4096 - ($ - $$) db 0    ; pad stub to exactly 4 KiB so the kernel begins at 0x2000
```

- [ ] **Step 2: Write the Makefile**

Create `Makefile`:

```makefile
# Builds guest.img = [4 KiB boot stub | Rust kernel] for `minikvm run guest.img`.
KERNEL_ELF := target/x86_64-unknown-none/debug/kernel

.PHONY: all clean kernel
all: guest.img

build:
	mkdir -p build

build/boot.bin: guest/boot.asm | build
	nasm -f bin guest/boot.asm -o build/boot.bin

kernel: | build
	cd crates/kernel && cargo build
	objcopy -O binary $(KERNEL_ELF) build/kernel.bin

guest.img: build/boot.bin kernel
	cat build/boot.bin build/kernel.bin > guest.img

clean:
	rm -rf build guest.img
```

- [ ] **Step 3: Ignore the build dir (but not guest.img)**

Add to `.gitignore` (append):

```gitignore
# Month-2 guest build intermediates (guest.img itself is committed)
/build
```

- [ ] **Step 4: Build guest.img**

Run: `make`
Expected: produces `build/boot.bin` (exactly 4096 bytes — verify with `wc -c build/boot.bin`), `build/kernel.bin`, and `guest.img`. If `nasm` errors with a negative `times` count, the stub exceeded 4 KiB (it shouldn't).

- [ ] **Step 5: First end-to-end boot (against the unchanged host)**

Run: `cargo run -p vmm -- run guest.img --trace`
Expected:
```
[host] created VM
[host] mapped 64 MiB guest memory
[host] created vCPU 0
123K
[host] VM exits: io=4, hlt=1, mmio=0
[host] runtime: ...
```
`1`,`2`,`3` are the phase markers, `K` is the kernel. If you see only `1` (or nothing), the failing transition is right after the last marker printed — debug there. (`io=4` = three markers + `K`.)

- [ ] **Step 6: Commit**

```bash
git add guest/boot.asm Makefile .gitignore
git commit -m "feat: boot stub climbs real->long mode and enters the kernel"
```

---

### Task 2: Host VMM — DLAB-aware 16550 UART (TDD)

The kernel's real serial driver (Task 3) will *poll* the Line Status Register, so the host must answer `in` from the UART and track the DLAB bit (so a baud-divisor write isn't printed as data). This must land before Task 3 or the guest's poll loop hangs forever.

**Files:**
- Modify: `crates/vmm/src/serial.rs` (replace contents)
- Modify: `crates/vmm/src/vcpu.rs` (run-loop dispatch)
- Modify: `crates/vmm/src/main.rs` (construct the UART)
- Test: inline `#[cfg(test)]` in `crates/vmm/src/serial.rs`

- [ ] **Step 1: Replace the serial module with a 16550 model + tests**

Replace `crates/vmm/src/serial.rs` with:

```rust
//! Month-2 16550 UART model (transmit-only, polling-friendly).
//! Models just enough that a guest polling driver works:
//!   - reading LSR always reports "ready to transmit"
//!   - writes to the data register go to the sink, but ONLY when DLAB is clear
//!     (when DLAB is set, a write to the data port is the baud-divisor low byte, not data)
//!   - all other register writes are accepted and ignored
//! This is still NOT a full UART (no receive, no interrupts).

use std::io::Write;

pub const COM1_BASE: u16 = 0x3f8;
pub const COM1_LAST: u16 = COM1_BASE + 7;

const REG_DATA: u16 = 0; // THR (or divisor-low when DLAB=1)
const REG_LCR: u16 = 3; // line control; bit 7 = DLAB
const REG_LSR: u16 = 5; // line status (read-only)

const LSR_READY: u8 = 0x60; // THR-empty (0x20) | transmitter-empty (0x40)
const LCR_DLAB: u8 = 0x80;

pub struct Uart<W: Write> {
    out: W,
    dlab: bool,
}

impl<W: Write> Uart<W> {
    pub fn new(out: W) -> Self {
        Self { out, dlab: false }
    }

    /// Handle a guest `out` of one byte to a UART port.
    pub fn write_reg(&mut self, port: u16, value: u8) {
        match port - COM1_BASE {
            REG_DATA if !self.dlab => {
                let _ = self.out.write_all(&[value]);
                let _ = self.out.flush();
            }
            REG_LCR => self.dlab = value & LCR_DLAB != 0,
            _ => {} // divisor latch, IER, FCR, MCR, ... — accept and ignore
        }
    }

    /// Handle a guest `in` from a UART port; returns the byte to give the guest.
    pub fn read_reg(&self, port: u16) -> u8 {
        match port - COM1_BASE {
            REG_LSR => LSR_READY,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_write_forwards_when_dlab_clear() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut u = Uart::new(&mut buf);
            u.write_reg(COM1_BASE, b'A');
        }
        assert_eq!(buf, b"A");
    }

    #[test]
    fn divisor_write_suppressed_when_dlab_set() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut u = Uart::new(&mut buf);
            u.write_reg(COM1_BASE + 3, 0x80); // LCR: set DLAB
            u.write_reg(COM1_BASE, 0x0C); // divisor low — must NOT print
            u.write_reg(COM1_BASE + 3, 0x03); // LCR: clear DLAB, 8N1
            u.write_reg(COM1_BASE, b'B'); // real data
        }
        assert_eq!(buf, b"B");
    }

    #[test]
    fn lsr_reports_ready() {
        let u = Uart::new(Vec::new());
        assert_eq!(u.read_reg(COM1_BASE + 5), 0x60);
    }

    #[test]
    fn other_reads_are_zero() {
        let u = Uart::new(Vec::new());
        assert_eq!(u.read_reg(COM1_BASE + 1), 0);
    }
}
```

- [ ] **Step 2: Run the serial tests**

Run: `cargo test -p vmm serial`
Expected: 4 tests pass (`data_write_forwards_when_dlab_clear`, `divisor_write_suppressed_when_dlab_set`, `lsr_reports_ready`, `other_reads_are_zero`).

- [ ] **Step 3: Route UART IO exits to the model**

In `crates/vmm/src/vcpu.rs`, update the imports and the run loop. Replace the import line:

```rust
use crate::serial::{Serial, COM1_PORT};
```
with:
```rust
use crate::serial::{Uart, COM1_BASE, COM1_LAST};
```

Change the `run` signature from `serial: &mut Serial<W>` to `uart: &mut Uart<W>`, and replace the `IoOut`/`IoIn` arms with:

```rust
            VcpuExit::IoOut(port, data) => {
                stats.record_io();
                if (COM1_BASE..=COM1_LAST).contains(&port) {
                    for &b in data {
                        uart.write_reg(port, b);
                    }
                }
            }
            VcpuExit::IoIn(port, data) => {
                stats.record_io();
                if (COM1_BASE..=COM1_LAST).contains(&port) {
                    let v = uart.read_reg(port);
                    for b in data.iter_mut() {
                        *b = v;
                    }
                }
            }
```

(The `MmioRead/MmioWrite`, `Hlt`, and `other` arms are unchanged.)

- [ ] **Step 4: Construct the UART in main**

In `crates/vmm/src/main.rs`, replace:

```rust
    let mut serial = serial::Serial::new(stdout.lock());
```
with:
```rust
    let mut uart = serial::Uart::new(stdout.lock());
```
and update the run call from `vcpu::run(&mut vcpu, &mut serial, &mut stats)?;` to:
```rust
    vcpu::run(&mut vcpu, &mut uart, &mut stats)?;
```

- [ ] **Step 5: Build, test, and confirm no Month-1 regression**

Run: `cargo test -p vmm`
Expected: unit tests pass AND the Month-1 integration test `run_guest` still passes (the old `hello.bin` writes data with DLAB clear, so it still prints `hello from guest` and halts).

- [ ] **Step 6: Commit**

```bash
git add crates/vmm/src/serial.rs crates/vmm/src/vcpu.rs crates/vmm/src/main.rs
git commit -m "feat(vmm): DLAB-aware 16550 UART so a polling guest driver works"
```

---

### Task 3: Kernel serial driver (polling) + real boot lines; drop phase markers

Now the kernel gets a real polling 16550 driver implementing `core::fmt::Write`, prints the first real lines, and we delete the stub's phase markers (the path is proven). Requires Task 2.

**Files:**
- Create: `crates/kernel/src/io.rs`
- Create: `crates/kernel/src/serial.rs`
- Modify: `crates/kernel/src/main.rs`
- Modify: `guest/boot.asm` (remove markers)

- [ ] **Step 1: Port I/O helpers**

Create `crates/kernel/src/io.rs`:

```rust
//! Minimal port I/O wrappers (the only place the kernel uses raw `in`/`out`).

pub unsafe fn outb(port: u16, val: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") val,
                     options(nomem, nostack, preserves_flags));
}

pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    core::arch::asm!("in al, dx", out("al") val, in("dx") port,
                     options(nomem, nostack, preserves_flags));
    val
}
```

- [ ] **Step 2: Polling 16550 serial driver**

Create `crates/kernel/src/serial.rs`:

```rust
//! Polling 16550 serial driver. Transmit waits for the LSR THR-empty bit, which is
//! exactly why the host models the LSR (see vmm/src/serial.rs).

use core::fmt::{self, Write};

use crate::io::{inb, outb};

const COM1: u16 = 0x3f8;
const LSR_THR_EMPTY: u8 = 0x20;

pub struct Serial;

impl Serial {
    /// Classic 16550 init (the emulated UART ignores the divisor, but the sequence is real).
    pub fn init() {
        unsafe {
            outb(COM1 + 1, 0x00); // disable UART interrupts
            outb(COM1 + 3, 0x80); // DLAB on
            outb(COM1 + 0, 0x01); // divisor low (115200)
            outb(COM1 + 1, 0x00); // divisor high
            outb(COM1 + 3, 0x03); // 8N1, DLAB off
            outb(COM1 + 2, 0xC7); // enable + clear FIFO, 14-byte threshold
            outb(COM1 + 4, 0x0B); // DTR | RTS | OUT2
        }
    }

    fn write_byte(b: u8) {
        unsafe {
            while inb(COM1 + 5) & LSR_THR_EMPTY == 0 {}
            outb(COM1, b);
        }
    }
}

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            Serial::write_byte(b);
        }
        Ok(())
    }
}
```

- [ ] **Step 3: Rewrite `_start` to use the driver**

In `crates/kernel/src/main.rs`: add the module declarations and `use`, delete the standalone `outb` helper (it now lives in `io.rs`), and rewrite `_start`. The mem intrinsics and panic handler stay.

Add near the top (after the attributes):

```rust
mod io;
mod serial;

use core::fmt::Write;
use serial::Serial;
```

Remove the old `unsafe fn outb(...) { ... }` from `main.rs`.

Replace `_start` with:

```rust
#[no_mangle]
#[link_section = ".text.boot"]
pub extern "C" fn _start() -> ! {
    Serial::init();
    let mut com = Serial;
    let _ = writeln!(com, "[guest] kernel entered");
    // (full paging added in Task 4 prints "[guest] paging enabled" here)
    let _ = writeln!(com, "hello from the kernel (long mode)");
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}
```

Update the panic handler to report before halting:

```rust
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    let mut com = Serial;
    let _ = writeln!(com, "[guest] PANIC");
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}
```

- [ ] **Step 4: Remove the phase markers from the boot stub**

In `guest/boot.asm`, delete the three marker blocks (each is the `mov dx, COM1` / `mov al, 'N'` / `out dx, al` trio after `protected:`, in `long_mode:` after the segment loads, and just before `mov rax, KERNEL_ENTRY`). Leave everything else.

- [ ] **Step 5: Rebuild and run**

Run: `make && cargo run -p vmm -- run guest.img --trace`
Expected:
```
[host] created VM
[host] mapped 64 MiB guest memory
[host] created vCPU 0
[guest] kernel entered
hello from the kernel (long mode)
[host] VM exits: io=N, hlt=1, mmio=0
[host] runtime: ...
```
(`io=N` is now larger — every LSR poll counts. Don't assert on it.)

- [ ] **Step 6: Commit**

```bash
git add crates/kernel/src/io.rs crates/kernel/src/serial.rs crates/kernel/src/main.rs guest/boot.asm
git commit -m "feat(kernel): polling 16550 driver; print boot lines; drop phase markers"
```

---

### Task 4: Kernel full paging + `[guest] paging enabled`

The kernel builds its own 4-level tables identity-mapping all 64 MiB (32 × 2 MiB huge pages), loads CR3, and announces it. Because the map is identity, code at `0x2000` and stack at `0x100000` stay valid across the CR3 switch.

**Files:**
- Create: `crates/kernel/src/paging.rs`
- Modify: `crates/kernel/src/main.rs`

- [ ] **Step 1: Write the paging module**

Create `crates/kernel/src/paging.rs`:

```rust
//! Stage-2 paging: replace the stub's 2 MiB bootstrap map with a full 64 MiB identity map
//! (32 x 2 MiB huge pages). Tables live in BSS (zeroed by KVM); identity mapping means a
//! table's virtual address equals its physical address, which is what CR3 wants.

use core::ptr::addr_of_mut;

#[repr(C, align(4096))]
struct PageTable([u64; 512]);

static mut PML4: PageTable = PageTable([0; 512]);
static mut PDPT: PageTable = PageTable([0; 512]);
static mut PD: PageTable = PageTable([0; 512]);

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const HUGE: u64 = 1 << 7;
const PAGE_2MIB: u64 = 0x20_0000;

/// Build a 64 MiB identity map and load it into CR3.
pub unsafe fn init_identity_map() {
    let pml4 = addr_of_mut!(PML4);
    let pdpt = addr_of_mut!(PDPT);
    let pd = addr_of_mut!(PD);

    (*pml4).0[0] = (pdpt as u64) | PRESENT | WRITABLE;
    (*pdpt).0[0] = (pd as u64) | PRESENT | WRITABLE;

    let mut i = 0usize;
    while i < 32 {
        (*pd).0[i] = (i as u64 * PAGE_2MIB) | PRESENT | WRITABLE | HUGE;
        i += 1;
    }

    core::arch::asm!("mov cr3, {}", in(reg) pml4 as u64, options(nostack, preserves_flags));
}
```

- [ ] **Step 2: Call it from `_start`**

In `crates/kernel/src/main.rs`, add `mod paging;` with the other module declarations, and insert the paging step into `_start` between the two `writeln!`s:

```rust
    let _ = writeln!(com, "[guest] kernel entered");
    unsafe { paging::init_identity_map(); }
    let _ = writeln!(com, "[guest] paging enabled");
    let _ = writeln!(com, "hello from the kernel (long mode)");
```

- [ ] **Step 3: Rebuild and run**

Run: `make && cargo run -p vmm -- run guest.img --trace`
Expected (matches the spec's done criteria):
```
[host] created VM
[host] mapped 64 MiB guest memory
[host] created vCPU 0
[guest] kernel entered
[guest] paging enabled
hello from the kernel (long mode)
[host] VM exits: io=N, hlt=1, mmio=0
[host] runtime: ...
```

- [ ] **Step 4: Confirm the no-trace output**

Run: `cargo run -p vmm -- run guest.img`
Expected: just the three `[guest]`/hello lines (no `[host]` lines).

- [ ] **Step 5: Commit**

```bash
git add crates/kernel/src/paging.rs crates/kernel/src/main.rs
git commit -m "feat(kernel): build full 64 MiB identity map and load CR3"
```

---

### Task 5: End-to-end integration test + commit `guest.img`

Mirror Month-1's `run_guest.rs`: run the built VMM against the committed `guest.img` and assert on output substrings. Auto-skips without `/dev/kvm`.

**Files:**
- Create: `crates/vmm/tests/run_kernel.rs`
- Commit: `guest.img` (the built artifact)

- [ ] **Step 1: Write the integration test**

Create `crates/vmm/tests/run_kernel.rs`:

```rust
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
        "hello from the kernel",
        "hlt=1",
    ] {
        assert!(stdout.contains(needle), "missing '{needle}' in:\n{stdout}");
    }
}
```

- [ ] **Step 2: Ensure guest.img is current, then run the test**

Run: `make && cargo test -p vmm --test run_kernel -- --nocapture`
Expected: PASS (prints the guest output); SKIP+pass if `/dev/kvm` is absent.

- [ ] **Step 3: Run the full suite**

Run: `cargo test -p vmm`
Expected: all unit tests + both integration tests (`run_guest` from Month 1 and `run_kernel`) pass.

- [ ] **Step 4: Commit the test and the image artifact**

```bash
git add crates/vmm/tests/run_kernel.rs guest.img
git commit -m "test: end-to-end test booting the Rust kernel into long mode"
```

---

### Task 6: Dev-log update

**Files:**
- Modify: `docs/devlog.md`

- [ ] **Step 1: Append the Month-2 (slice 1) decisions**

Append to `docs/devlog.md`:

```markdown

## Month 2 (slice 1) — Boot stub + long mode + Rust kernel

- Single `guest.img` = `boot.bin` (NASM, padded to exactly 4 KiB) ++ `kernel.bin`
  (Rust → objcopy). Stub loads at 0x1000; kernel entry is therefore at 0x2000.
- Boot path: real → (CR0.PE + far jmp) → 32-bit protected → build stage-1 tables, CR4.PAE,
  CR3, EFER.LME, CR0.PG → (far jmp to L-bit segment) → 64-bit long mode → enable SSE → jump
  to the kernel. Stage-1 maps only the first 2 MiB (one huge page); the kernel then builds
  the full 64 MiB identity map and reloads CR3.
- GDT lives in the stub; in real mode `lgdt` loads a 24-bit base, which is fine since the
  GDT sits at ~0x1000.
- SSE must be enabled before Rust runs (compiler may emit SSE) — done in the stub.
- Host UART grew a DLAB bit: a data-port write while DLAB is set is a baud divisor, not a
  character — forwarding it blindly would corrupt output. The guest's polling driver reads
  LSR, so the host now answers `in` with the THR-empty bit set.
- Debugging without an IDT: the stub emitted phase-marker bytes ('1','2','3') via raw `out`
  during bring-up (works with the unchanged Month-1 host); removed once stable.
- Kernel build stays on stable Rust: target `x86_64-unknown-none` (no -Zbuild-std), and the
  `memcpy`/`memset`/`memmove`/`memcmp` intrinsics are hand-written. Kernel build config is
  isolated in `crates/kernel/.cargo/config.toml`; `default-members` keeps host `cargo` off
  the kernel.
- A20: assumed unnecessary under KVM; the boot succeeded without touching it.
```

- [ ] **Step 2: Commit**

```bash
git add docs/devlog.md
git commit -m "docs: dev-log notes for Month-2 boot stub + kernel"
```

---

## Definition of Done

- `cargo test -p vmm` passes (unit + `run_guest` + `run_kernel`) on the WSL2 box.
- `make && cargo run -p vmm -- run guest.img --trace` prints the host lifecycle lines, then
  `[guest] kernel entered`, `[guest] paging enabled`, `hello from the kernel (long mode)`,
  then `VM exits: io=N, hlt=1, mmio=0`.
- Without `--trace`, only the three guest lines print.
- `guest.img` is committed; `make` regenerates it; `/build` is git-ignored.
- Code is committed in small, focused commits per task.

## Deferred to later Month-2 slices (do not pull forward)

IDT / trap & exception handlers; kernel heap / frame allocator; syscall dispatch
(SYSCALL/SYSRET); user/kernel privilege transition; C userspace and ELF loading.
(Per the spec §12.)
```
