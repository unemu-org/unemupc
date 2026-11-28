; SPDX-License-Identifier: AGPL-3.0-or-later
; Bochs VBE DISPI interface (vbe.h): the banked A0000h window and the linear
; framebuffer are two views of one video memory. From flat protected mode
; (Intel486 Programmer's Reference Manual, Chapter 10 Initialization) this
; program stores through each view and reads through the other, in both
; orders, re-selects banks, checks that enabling clears video memory and
; writes a few served ports. Results go to bytes 600h-60Fh.
; Assemble with nasm -f bin. See crates/pc-harness/tests/bochs_vbe_golden.rs.
cpu 486
bits 16
org 0xf8000
%define RM(x) (x - $$)
%define LFB 0xe0000000

%macro DISPI 2
    mov dx, 0x1ce
    mov ax, %1
    out dx, ax
    mov dx, 0x1cf
    mov ax, %2
    out dx, ax
%endmacro

start:
    cli
    cld
    xor ax, ax
    mov ds, ax
    mov ss, ax
    mov sp, 0x8000
    lgdt [cs:RM(gdtr)]
    mov eax, cr0
    or eax, 1
    mov cr0, eax
    jmp dword 8:protected
bits 32
protected:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov esp, 0x8000
    ; 640x480x8 keeping memory, then enabled again with the clear.
    DISPI 4, 0
    DISPI 3, 8
    DISPI 1, 640
    DISPI 2, 480
    DISPI 4, 0xc1
    mov byte [LFB+0x100], 0x5a
    mov al, [LFB+0x100]
    mov [0x600], al
    DISPI 4, 0
    DISPI 4, 0x41
    mov al, [LFB+0x100]
    mov [0x601], al
    ; Window first: stores on both sides of a framebuffer store.
    DISPI 5, 1
    mov byte [0xa0010], 0x11
    mov byte [LFB+0x10018], 0x22
    mov byte [0xa0030], 0x33
    mov al, [0xa0018]
    mov [0x602], al
    mov al, [LFB+0x10010]
    mov [0x603], al
    mov al, [LFB+0x10030]
    mov [0x604], al
    ; Framebuffer first, then the window, then the bank selected again.
    mov byte [LFB+0x10040], 0x44
    mov al, [0xa0040]
    mov [0x605], al
    mov byte [0xa0040], 0x55
    mov al, [LFB+0x10040]
    mov [0x606], al
    DISPI 5, 1
    mov al, [0xa0040]
    mov [0x607], al
    ; Another bank, and a doubleword through the window.
    DISPI 5, 2
    mov byte [0xa0000], 0x66
    mov al, [LFB+0x20000]
    mov [0x608], al
    mov dword [0xa0100], 0x04030201
    mov eax, [LFB+0x20100]
    mov [0x60c], eax
    mov al, [0xa0040]
    mov [0x609], al
    ; Port writes the machine serves while the CPU runs.
    mov al, 1
    out 0x80, al
    out 0x80, al
    out 0x80, al
    hlt

align 8
gdt:
    dq 0
    dq 0x00cf9a000000ffff
    dq 0x00cf92000000ffff
gdtr:
    dw 3*8-1
    dd 0xf8000+RM(gdt)

bits 16
    times 0x7ff0-($-$$) db 0
    jmp 0xf800:0
    times 0x8000-($-$$) db 0
