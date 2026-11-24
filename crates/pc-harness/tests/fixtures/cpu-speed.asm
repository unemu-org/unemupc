; SPDX-License-Identifier: AGPL-3.0-or-later
; Intel 8254 datasheet, Mode 2; Intel 8259A datasheet, ICWs and EOI.
; Count two-instruction loop iterations between IRQ0 edges. The first
; sample includes setup and is intentionally excluded from the golden.
bits 16
org 0
start:
    cli
    xor ax, ax
    mov ds, ax
    mov ss, ax
    mov sp, 0x7000
    mov word [0x20], irq0
    mov word [0x22], 0xf800
    mov word [0x500], 0
    mov al, 0x11
    out 0x20, al
    mov al, 8
    out 0x21, al
    mov al, 4
    out 0x21, al
    mov al, 1
    out 0x21, al
    mov al, 0xfe
    out 0x21, al
    mov al, 0x34
    out 0x43, al
    mov al, 0
    out 0x40, al
    mov al, 0x10
    out 0x40, al
    xor bx, bx
    sti
count:
    inc bx
    jmp count
irq0:
    push ax
    mov di, [0x500]
    shl di, 1
    mov [di + 0x502], bx
    inc word [0x500]
    xor bx, bx
    mov al, 0x20
    out 0x20, al
    pop ax
    iret
    times 0x7ff0-($-$$) db 0
    jmp 0xf800:start
    times 0x8000-($-$$) db 0
