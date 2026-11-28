; SPDX-License-Identifier: AGPL-3.0-or-later
; Bochs VBE DISPI interface (vbe.h): detect the interface, set a 640x480
; linear-framebuffer mode and fill it from protected mode. Intel486
; Programmer's Reference Manual, Chapter 10 Initialization, enters the flat
; 32-bit segments. Assemble with nasm -f bin and one of:
;   (nothing)        32 bpp; pixel n holds the dword n
;   -DBPP16          16 bpp; pixel n holds the word n
;   -DSPEED=address  store dwords forever over 1 MiB at address
;   -DSPEED=0xa0000 -DBANKED  the same over the 64 KiB banked window, after
;                    setting a DISPI mode
; See crates/pc-harness/tests/bochs_vbe_golden.rs.
cpu 486
bits 16
org 0xf8000
%define RM(x) (x - $$)
%ifdef BPP16
    %define BPP 16
%else
    %define BPP 32
%endif

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
%ifndef SPEED
    ; The handshake and the memory size, for the host to check.
    DISPI 0, 0xb0c0
    in ax, dx
    mov [0x500], ax
    DISPI 0, 0xb0c5
    mov dx, 0x1ce
    mov ax, 0x0a
    out dx, ax
    mov dx, 0x1cf
    in ax, dx
    mov [0x502], ax
    DISPI 4, 0
    DISPI 3, BPP
    DISPI 1, 640
    DISPI 2, 480
    DISPI 4, 0x41
%elifdef BANKED
    DISPI 3, 32
    DISPI 1, 640
    DISPI 2, 480
    DISPI 4, 0xc1
%endif
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
%ifdef SPEED
    mov ebx, SPEED
    xor esi, esi
    xor eax, eax
.store:
    mov [ebx+esi], eax
    add esi, 4
%ifdef BANKED
    and esi, 0xfffc
%else
    and esi, 0xffffc
%endif
    inc eax
    jmp .store
%else
    mov edi, 0xe0000000
    mov ecx, 640*480
    xor eax, eax
.pixel:
%if BPP == 16
    stosw
%else
    stosd
%endif
    inc eax
    dec ecx
    jnz .pixel
    hlt
%endif

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
