; SPDX-License-Identifier: AGPL-3.0-or-later
; Shared pieces of the bootable speed-ceiling payloads. The boot sector loads
; a payload to LOAD and jumps to its first byte in real mode with CS = LOAD/16.
;
; The real-mode prelude runs before any case setup, while the BIOS is still
; usable: it selects 80x25 colour text, opens the A20 gate, stops the floppy
; motor, masks every interrupt line and NMI, clears FS and GS and prints the
; START line. Everything after it runs with interrupts off and never calls the
; BIOS again; the END line is stored straight into text memory at B8000h.
;
; IBM VGA Technical Reference, Video BIOS, INT 10h functions 00h (set mode)
; and 0Eh (teletype output). IBM Personal Computer AT Technical Reference,
; Keyboard Controller, commands D1h (write output port, bit 1 is the A20
; gate) and FFh (pulse output port with no line selected, a null command);
; System Board, CMOS RAM address register at port 70h (bit 7 masks NMI).
; IBM Personal System/2 Hardware Interface Technical Reference, System Control
; Port A (port 92h, bit 1 is the fast A20 gate, bit 0 the fast reset).
; Intel 82077AA datasheet, Digital Output Register (motor enables).
; Intel 8259A datasheet, OCW1 (interrupt mask register).

%define LOAD 0x10000
%define RM(x) ((x) - $$)
%define LIN(x) (LOAD + RM(x))

; Row 0 holds the START line, row 2 the END line.
%define END_ROW_ADDRESS (0xb8000 + 2 * 160)

%defstr ITER_STR ITER
%strcat START_TEXT "UNEMU-BENCH ", CASE_NAME, " START count=", ITER_STR
%strcat END_TEXT "UNEMU-BENCH ", CASE_NAME, " END count=", ITER_STR

; Copy a zero-terminated string to text memory with grey-on-black attributes.
; %1 is the source address as DS sees it, %2 the destination as ES sees it.
; The same mnemonics assemble for 16-bit and 32-bit code.
%macro WRITE_LINE 2
    mov esi, %1
    mov edi, %2
%%next:
    lodsb
    test al, al
    jz %%done
    mov ah, 0x07
    stosw
    jmp %%next
%%done:
%endmacro

; Stop for good. Interrupts are masked at the CPU and at the controllers.
%macro HALT_FOREVER 0
    cli
%%halt:
    hlt
    jmp %%halt
%endmacro

%macro PRELUDE 0
    cli
    mov ax, cs
    mov ds, ax
    xor ax, ax
    mov ss, ax
    mov sp, 0x7c00
    sti
    cld
    mov ax, 0x0003
    int 0x10
    cli
    call %%a20_enable
    mov word [RM(%%a20_text)], RM(%%a20_on)
    call %%a20_wraps
    jnz %%a20_reported
    mov word [RM(%%a20_text)], RM(%%a20_off)
%%a20_reported:
    ; Drive A selected, controller out of reset, DMA enabled, every motor off:
    ; the timer interrupt that would normally stop the motor stays masked.
    mov dx, 0x3f2
    mov al, 0x0c
    out dx, al
    mov al, 0xff
    out 0xa1, al
    out 0x21, al
    ; NMI off; the CMOS index selects status register D, which is read-only.
    mov al, 0x8d
    out 0x70, al
    ; Real-mode values in FS and GS would otherwise reach every case unchanged.
    xor ax, ax
    mov fs, ax
    mov gs, ax
    mov si, RM(%%start_text)
    call %%puts
    mov si, [RM(%%a20_text)]
    call %%puts
    jmp %%done

; Teletype output of DS:SI. Some video BIOSes do not preserve every register
; across INT 10h, so keep the string pointer on the stack.
%%puts:
    lodsb
    test al, al
    jz %%puts_done
    push si
    mov ah, 0x0e
    mov bx, 0x0007
    int 0x10
    pop si
    jmp %%puts
%%puts_done:
    ret

; Open the gate both ways: through the keyboard controller output port, which
; every AT-compatible board has, and through port 92h where it exists. A board
; without port 92h usually reads FFh there; bit 1 then already looks set and the
; write, which could otherwise pulse the reset bit, is skipped.
%%a20_enable:
    call %%kbc_wait
    mov al, 0xd1
    out 0x64, al
    call %%kbc_wait
    mov al, 0xdf
    out 0x60, al
    call %%kbc_wait
    ; A null command lets slow controllers finish the output-port write.
    mov al, 0xff
    out 0x64, al
    call %%kbc_wait
    in al, 0x92
    test al, 2
    jnz %%a20_enabled
    or al, 2
    and al, 0xfe
    out 0x92, al
%%a20_enabled:
    ret

; Wait, bounded, until the controller's input buffer is empty.
%%kbc_wait:
    mov cx, 0xffff
%%kbc_poll:
    in al, 0x64
    test al, 2
    jz %%kbc_ready
    loop %%kbc_poll
%%kbc_ready:
    ret

; ZF clear when 0000:0500 and FFFF:0510 are different bytes, that is when
; address bit 20 reaches memory. Both bytes are restored.
%%a20_wraps:
    push ds
    push es
    xor ax, ax
    mov ds, ax
    dec ax
    mov es, ax
    mov bl, [ds:0x500]
    mov bh, [es:0x510]
    mov byte [ds:0x500], 0x00
    mov byte [es:0x510], 0xff
    cmp byte [ds:0x500], 0xff
    pushf
    mov [es:0x510], bh
    mov [ds:0x500], bl
    popf
    pop es
    pop ds
    jz %%a20_closed
    ; Clear ZF: the gate is open.
    or ax, ax
%%a20_closed:
    ret

%%a20_text: dw 0
%%start_text: db START_TEXT, 0
%%a20_on: db " A20=on", 0
%%a20_off: db " A20=off", 0
%%done:
    cli
%endmacro
