// crates/kernel/src/syscall_entry.s
//
// SYSCALL entry stub. Pulled into the kernel crate via
// global_asm!(include_str!("syscall_entry.s")) in syscall.rs.
//
// On SYSCALL the CPU:
//   - loads CS/SS from IA32_STAR (kernel selectors, CPL -> 0)
//   - saves return RIP into RCX
//   - saves pre-mask RFLAGS into R11
//   - masks RFLAGS via IA32_FMASK (clears IF/TF/DF)
//   - loads RIP from IA32_LSTAR (this stub)
//   - DOES NOT switch RSP (we still run on the user stack)
//   - DOES NOT save any general-purpose registers
//
// This stub:
//   1. Swaps RSP with the global KERNEL_RSP to switch to the kernel syscall stack.
//      Single-vCPU, IF=0, non-reentrant: the symmetric xchg pair is sound.
//   2. Builds a SyscallFrame on the kernel stack (six pushes, 48 bytes = 3*16, so the
//      "call rust_syscall_dispatch" site is 16-byte aligned per SysV when KERNEL_RSP
//      starts 16-aligned -- which it does, via the #[repr(C, align(16))] wrapper on
//      SYSCALL_STACK).
//   3. Calls into Rust with RDI = &SyscallFrame. The dispatch's return is in RAX.
//   4. Pops the frame, restores user RCX/R11 (consumed by sysretq).
//   5. Swaps RSP back to the user stack; sysretq returns to ring 3.
//
// Syntax: Intel (the global_asm! default; no syntax-selector directive needed).

.section .text
.extern rust_syscall_dispatch
.extern KERNEL_RSP

.globl syscall_entry
syscall_entry:
    xchg rsp, [rip + KERNEL_RSP]   // RSP <- kernel top; global holds user RSP

    // Build SyscallFrame (num lands at lowest addr; struct mirrors push order).
    push rcx                       // user RIP   ┐
    push r11                       // user RFLAGS │
    push rdx                       // arg3        │ 6 pushes = 48 B = 3 * 16
    push rsi                       // arg2        │
    push rdi                       // arg1        │
    push rax                       // num         ┘

    mov rdi, rsp                   // SysV first arg: pointer to SyscallFrame
    call rust_syscall_dispatch     // returns u64 in RAX (sys_exit halts inside)

    add rsp, 32                    // drop num, arg1, arg2, arg3
    pop r11                        // restore user RFLAGS
    pop rcx                        // restore user RIP

    xchg rsp, [rip + KERNEL_RSP]   // RSP <- user; global back to kernel top
    sysretq                        // REX.W: CS/SS from STAR (RPL3), RIP<-RCX, RFLAGS<-R11
