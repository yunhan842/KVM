// crates/kernel/src/idt_stubs.s
//
// 32 ISR trampolines (vectors 0..32) + shared isr_common. Pulled into the kernel
// crate via global_asm!(include_str!("idt_stubs.s")) in idt.rs.
//
// Two macros emit two flavors of stub:
//   ISR_NOERR <vec>: vectors where the CPU does NOT push an error code. The stub
//                   pushes a synthetic 0 so the stack shape is uniform.
//   ISR_ERR   <vec>: vectors where the CPU has already pushed an error code.
//
// Both flavors then push the vector number and jump to isr_common, which saves
// the GPR file, calls into Rust, restores, and iretq's. For exception vectors
// the Rust dispatcher never returns -- the post-call epilogue is structurally
// dead code, kept for shape uniformity with future IRQ/syscall slices.
//
// Stack at the moment `call rust_isr_dispatch` executes (RSP -> low addr):
//
//   +0      r15  ┐
//   +8      r14  │  pushed by isr_common (15 GPRs, 120 B)
//   ...          │
//   +112    rax  ┘
//   +120    vector              ┐ pushed by isr_<N>
//   +128    error_code (or 0)   ┘
//   +136    rip   ┐
//   +144    cs    │
//   +152    rflags│ pushed by the CPU on entry (5 * 8 = 40 B)
//   +160    rsp   │
//   +168    ss    ┘
//
// 40 + 16 + 120 = 176 B = 11 * 16 -> RSP at the call site is 16-aligned
// when pre-fault RSP was 16-aligned (it is: slice-1 STACK_TOP = 0x100000,
// and Rust preserves stack alignment across calls). Required by SysV.
//
// Syntax: Intel (the global_asm! default; no syntax-selector directive needed).

.section .text

.extern rust_isr_dispatch

.macro ISR_NOERR vec
    .globl isr_\vec
isr_\vec:
    push 0                  // synthetic error code
    push \vec
    jmp isr_common
.endm

.macro ISR_ERR vec
    .globl isr_\vec
isr_\vec:
    // CPU has already pushed a real error code
    push \vec
    jmp isr_common
.endm

// Vectors 0..32. Per Intel SDM Vol 3A Ch 6, vectors 8, 10, 11, 12, 13, 14, 17,
// and 21 push an error code; the others don't.
ISR_NOERR 0
ISR_NOERR 1
ISR_NOERR 2
ISR_NOERR 3
ISR_NOERR 4
ISR_NOERR 5
ISR_NOERR 6
ISR_NOERR 7
ISR_ERR   8
ISR_NOERR 9
ISR_ERR   10
ISR_ERR   11
ISR_ERR   12
ISR_ERR   13
ISR_ERR   14
ISR_NOERR 15
ISR_NOERR 16
ISR_ERR   17
ISR_NOERR 18
ISR_NOERR 19
ISR_NOERR 20
ISR_ERR   21
ISR_NOERR 22
ISR_NOERR 23
ISR_NOERR 24
ISR_NOERR 25
ISR_NOERR 26
ISR_NOERR 27
ISR_NOERR 28
ISR_NOERR 29
ISR_NOERR 30
ISR_NOERR 31
ISR_NOERR 32
ISR_NOERR 33
ISR_NOERR 34
ISR_NOERR 35
ISR_NOERR 36
ISR_NOERR 37
ISR_NOERR 38
ISR_NOERR 39
ISR_NOERR 40
ISR_NOERR 41
ISR_NOERR 42
ISR_NOERR 43
ISR_NOERR 44
ISR_NOERR 45
ISR_NOERR 46
ISR_NOERR 47

isr_common:
    // Save all 15 GPRs. rsp lives in the iretq frame, no need to save here.
    // Pushed in this order, so memory layout (ascending) is r15..rax -- the
    // InterruptContext struct in idt.rs mirrors that order exactly.
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    mov rdi, rsp            // SysV first-arg: pointer to InterruptContext
    call rust_isr_dispatch  // never returns for exception vectors; epilogue is dead code

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax

    add rsp, 16             // drop vector + error_code
    iretq
