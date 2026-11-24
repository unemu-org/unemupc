; SPDX-License-Identifier: AGPL-3.0-or-later
; IBM PC AT Technical Reference, NMI and Coprocessor Controls / System Interrupts.
; Rebuild: nasm -f bin at-irq13.asm -o at-irq13.bin
; BIOS chain boot sector: nasm -DBIOS_CHAIN -f bin at-irq13.asm -o at-irq13-boot.bin
; Immediate IRQ: nasm -DIMMEDIATE_IRQ13 -f bin at-irq13.asm -o at-irq13-immediate.bin
; RAM 0500h: IRQ13, #MF, timer counts, SW before/after F0h, completion.
bits 16
cpu 286
%ifdef BIOS_CHAIN
org 0x7c00
%define HANDLER_SEG 0
%else
org 0x8000
%define HANDLER_SEG 0xf000
%endif
entry:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7000
    mov di, 0x500
    mov cx, 6
    cld
    rep stosw
%ifdef BIOS_CHAIN
    mov word [2*4], irq13
    mov word [2*4+2], HANDLER_SEG
%else
    mov word [0x75*4], irq13
    mov word [0x75*4+2], HANDLER_SEG
%endif
    mov word [0x10*4], mf
    mov word [0x10*4+2], HANDLER_SEG
    mov word [8*4], timer
    mov word [8*4+2], HANDLER_SEG
    mov al, 0x11
    out 0x20, al
    out 0xa0, al
    mov al, 8
    out 0x21, al
    mov al, 0x70
    out 0xa1, al
    mov al, 4
    out 0x21, al
    mov al, 2
    out 0xa1, al
    mov al, 1
    out 0x21, al
    out 0xa1, al
%ifdef IMMEDIATE_IRQ13
    mov al, 0xdf
    out 0xa1, al
    mov al, 0xfb
    out 0x21, al
%else
    ; Hold IRQ13 masked until IRQ0 arrives while WAIT is blocked on BUSY.
    mov al, 0xff
    out 0xa1, al
    mov al, 0xfa
    out 0x21, al
    mov al, 0x30
    out 0x43, al
    mov al, 0
    out 0x40, al
    mov al, 4
    out 0x40, al
%endif
    fninit
    fldcw [cs:control]
    fld1
    fldz
    sti
    fdivp st1, st0
blocked_wait:
    wait
    ; A new cleared episode must generate exactly one new IRQ13.
    fld1
    fldz
    fdivp st1, st0
    wait
    ; Default masked exceptions must never assert IRQ13.
    fninit
    fld1
    fldz
    fdivp st1, st0
    wait
    mov word [0x50a], 0xa55a
%ifdef IMMEDIATE_IRQ13
    jmp $
%else
    cli
    hlt
%endif
irq13:
    push ax
    inc word [0x500]
    fnstsw [0x506]
    mov al, 0x55
    out 0xf0, al
    fnstsw [0x508]
    ; F0h releases WAIT without clearing the sticky numeric error.
    wait
    fnclex
    mov al, 0x20
    out 0xa0, al
    out 0x20, al
    pop ax
    iret
timer:
    push ax
    inc word [0x504]
    mov al, 0xdf
    out 0xa1, al
    mov al, 0xfb
    out 0x21, al
    mov al, 0x20
    out 0x20, al
    pop ax
    iret
mf:
    inc word [0x502]
    fnclex
    iret
control: dw 0x037b
%ifdef BIOS_CHAIN
    times 510-($-$$) db 0
    dw 0xaa55
%else
    times 0x7ff0-($-$$) db 0xf4
    jmp 0xf000:entry
    times 0x8000-($-$$) db 0xf4
%endif
