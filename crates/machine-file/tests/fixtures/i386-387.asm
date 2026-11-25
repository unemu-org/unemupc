; SPDX-License-Identifier: AGPL-3.0-or-later
; Intel 80386 PRM, Chapter 11 Coprocessor Interface; Intel 387 DX User's
; Manual, System Interface. IBM AT Technical Reference, Coprocessor Controls.
; Rebuild: nasm -f bin i386-387.asm -o i386-387.bin
; Reset-vector probe: the last 32 KiB of a 128 KiB BIOS at E0000h.
bits 16
cpu 386
org 0x8000
entry:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7000
    mov di, 0x500
    mov cx, 12
    cld
    rep stosw
    fninit
    fnstsw ax
    mov [0x500], ax
    fnstcw [0x502]
    smsw [0x504]
    mov eax, cr0
    mov [0x506], eax
    cmp word [0x500], 0xffff
    je done
    fild word [cs:seven]
    fild word [cs:six]
    fmulp st1, st0
    fistp word [0x50a]
    mov word [0x75*4], irq13
    mov word [0x75*4+2], 0xf000
    mov word [0x10*4], mf
    mov word [0x10*4+2], 0xf000
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
    mov al, 0xdf
    out 0xa1, al
    mov al, 0xfb
    out 0x21, al
    fldcw [cs:control]
    fld1
    fldz
    sti
    fdivp st1, st0
    wait
done:
    mov word [0x514], 0xa55a
    cli
    hlt
irq13:
    push ax
    inc word [0x50c]
    fnstsw [0x510]
    mov al, 0
    out 0xf0, al
    fnstsw [0x512]
    ; F0h releases BUSY without discarding the exception status.
    wait
    fnclex
    mov al, 0x20
    out 0xa0, al
    out 0x20, al
    pop ax
    iret
mf:
    inc word [0x50e]
    fnclex
    iret
seven: dw 7
six: dw 6
control: dw 0x037b
    times 0x7ff0-($-$$) db 0xf4
    jmp 0xf000:entry
    times 0x8000-($-$$) db 0xf4
