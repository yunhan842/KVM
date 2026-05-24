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

    ; stage-1 tables: identity-map first 2 MiB via one 2 MiB huge page.
    ; (guest RAM is zeroed by KVM; write only the live entries)
    mov dword [PML4], PDPT | 0x3        ; present|writable -> PDPT
    mov dword [PML4 + 4], 0
    mov dword [PDPT], PD | 0x3          ; present|writable -> PD
    mov dword [PDPT + 4], 0
    mov dword [PD], 0x83                ; present|writable|huge, phys 0
    mov dword [PD + 4], 0

    mov eax, cr4
    or  eax, 1 << 5            ; CR4.PAE (required for long mode)
    mov cr4, eax

    mov eax, PML4
    mov cr3, eax              ; point at the PML4

    mov ecx, 0xC0000080       ; EFER MSR
    rdmsr
    or  eax, 1 << 8           ; EFER.LME (long mode enable)
    wrmsr

    mov eax, cr0
    or  eax, 1 << 31          ; CR0.PG -> paging on -> enter IA-32e (compat sub-mode)
    mov cr0, eax

    jmp 0x18:long_mode        ; far jump to the 64-bit code selector

bits 64
long_mode:
    mov ax, 0x20              ; 64-bit data selector
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov rsp, STACK_TOP

    ; enable SSE (the Rust compiler may emit SSE instructions)
    mov rax, cr0
    and ax, 0xFFFB             ; clear CR0.EM (bit 2)
    or  ax, 0x2                ; set CR0.MP (bit 1)
    mov cr0, rax
    mov rax, cr4
    or  rax, (1 << 9) | (1 << 10)   ; CR4.OSFXSR | CR4.OSXMMEXCPT
    mov cr4, rax

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
