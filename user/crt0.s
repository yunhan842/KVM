# user/crt0.s — _start + write/exit syscall wrappers for the ring-3 program.
#
# GNU `as` defaults to AT&T syntax; the .intel_syntax directive must come
# before any instruction so the rest of the file parses as Intel.
#
# Comment convention: GAS recognizes `#` as a line comment. It does NOT accept
# `//` (unlike LLVM's integrated assembler the kernel's global_asm! uses),
# and `/* */` block comments only work if the file is preprocessed (uppercase
# .S extension); this .s file is not preprocessed.
#
# Syscall ABI alignment: the SysV AMD64 C calling convention puts the first
# three args in RDI/RSI/RDX, which is EXACTLY what our SYSCALL ABI expects.
# No register marshaling — just "set RAX, syscall, ret".
#
# Syscall numbers: 1 = write, 2 = exit, 3 = preempt. Must match
# crates/kernel/src/syscall.rs (SYS_WRITE / SYS_EXIT / SYS_PREEMPT).

.intel_syntax noprefix
.section .text

.globl _start
_start:
    call main                 # main returns its int in EAX
    mov edi, eax              # EDI = (zero-extended) exit code, SysV arg1
    call exit                 # diverging — does not return
    ud2                       # unreachable safety net

.globl write
write:                        # write(fd, buf, len): RDI/RSI/RDX already set
    mov rax, 1                # SYS_WRITE
    syscall                   # returns to caller; result in RAX
    ret

.globl exit
exit:                         # exit(code): RDI already set
    mov rax, 2                # SYS_EXIT
    syscall                   # sys_exit halts the kernel; never returns
    ud2

.globl preempt
preempt:                      # preempt(): no args; asks the kernel to start
    mov rax, 3                # SYS_PREEMPT  delivering timer interrupts to us
    syscall                   # returns to caller; result (0) in RAX, ignored
    ret

# Declare non-executable stack. Without this section GNU ld 2.42 warns
# "missing .note.GNU-stack section implies executable stack".
.section .note.GNU-stack, "", @progbits
