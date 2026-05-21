; guest/hello.asm — Month-1 throwaway guest.
; 16-bit real mode. Writes a string to COM1 (port 0x3f8) byte-by-byte, then halts.
; The VMM loads this at guest-physical 0x1000 with CS/DS base = 0 and RIP = 0x1000.
bits 16
org 0x1000

start:
    mov dx, 0x3f8          ; COM1 data port
    mov si, msg
.next:
    lodsb                  ; al = [ds:si], si++  (DF=0 so forward)
    test al, al
    jz .done
    out dx, al             ; -> KVM_EXIT_IO; host prints the byte
    jmp .next
.done:
    hlt                    ; -> KVM_EXIT_HLT; host stops the run loop

msg: db "hello from guest", 10, 0   ; 10 = '\n', 0 = terminator
