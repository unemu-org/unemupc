; SPDX-License-Identifier: AGPL-3.0-or-later
; Intel 80286 and 80287 Programmer's Reference Manual, System Initialization,
; entering protected mode. A 16-bit protected loop also works on the 286.
; Intel 80386 Programmer's Reference Manual §5.2 Page Translation defines
; the optional identity-mapped paged32 case; see machine-files.md §Measured speed ceilings.
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
%ifdef PAGED32
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    xor eax, eax
    mov edi, 0x1000
    mov ecx, 2048
    rep stosd
    mov dword [0x1000], 0x2003
    mov edi, 0x2000
    mov eax, 3
    mov ecx, 256
.pages:
    stosd
    add eax, 0x1000
    loop .pages
    mov eax, 0x1000
    mov cr3, eax
    mov eax, cr0
    or eax, 0x80000000
    mov cr0, eax
%endif
count_loop:
    inc ax
    add bx, ax
    xor dx, bx
    dec cx
    jmp count_loop
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
%ifdef PAGED32
    dq 0x00CF92000000FFFF
%endif
gdtr:
    dw $-gdt-1
    dd 0xf8000+gdt
%endif
    bits 16
    times 0x7ff0-($-$$) db 0
    jmp 0xf800:start
    times 0x8000-($-$$) db 0
