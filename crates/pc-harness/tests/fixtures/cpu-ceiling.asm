; SPDX-License-Identifier: AGPL-3.0-or-later
; Intel 80286 and 80287 Programmer's Reference Manual, System Initialization,
; entering protected mode. A 16-bit protected loop also works on the 286.
bits 16
org 0
start:
    cli
%ifdef PROTECTED
    lgdt [cs:gdtr]
    smsw ax
    or ax, 1
    lmsw ax
    jmp 8:count
%endif
count:
%ifdef PROTECTED32
    bits 32
%endif
    inc ax
    add bx, ax
    xor dx, bx
    dec cx
    jmp count
%ifdef PROTECTED
align 8
gdt:
    dq 0
    ; 64 KiB executable segment based at F8000h, present, ring 0.
    dw 0xffff, 0x8000
%ifdef PROTECTED32
    db 0x0f, 0x9a, 0x40, 0
%else
    db 0x0f, 0x9a, 0, 0
%endif
gdtr:
    dw $-gdt-1
    dd 0xf8000+gdt
%endif
    bits 16
    times 0x7ff0-($-$$) db 0
    jmp 0xf800:start
    times 0x8000-($-$$) db 0
