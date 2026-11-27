; SPDX-License-Identifier: AGPL-3.0-or-later
; Bootable copy of the register speed-ceiling loop, loaded to LOAD (10000h)
; by boot.asm. Intel 80286 and 80287 Programmer's Reference Manual, System
; Initialization, entering protected mode. Intel 80386 Programmer's Reference
; Manual §5.1 Segment Translation and §5.2 Page Translation.
;
; Assemble with nasm -f bin, -DPASSES=<n> and -DCASE_NAME='<name>', plus:
;   real16       (nothing)
;   protected32  -DPROTECTED -DPROTECTED32
;   paged32      -DPROTECTED -DPROTECTED32 -DPAGED32
;   flat32       -DPROTECTED -DPROTECTED32 -DFLAT32
;   flat-paged32 -DPROTECTED -DPROTECTED32 -DFLAT32 -DPAGED32
;
; The loop body is the ROM fixture's: INC, ADD, XOR, DEC on 16-bit registers.
; A ROM can loop forever, a timed boot cannot, so the closing JMP becomes JNZ
; on the body's own DEC CX. CX starts at zero, so the branch falls through once
; every 65536 iterations into an outer DEC ESI / JNZ that counts PASSES: the
; mix stays five instructions per iteration plus two per 65536.
;
; The ROM's code segment is based at F8000h; here it is based at the load
; address. Paging uses the ROM's tables at 1000h/2000h (identity map of the
; first MiB), which a booted machine leaves free once the BIOS is done.
cpu 386
%ifdef PAGED32
    %define DATA32
%endif
%ifdef FLAT32
    %define DATA32
%endif
%assign ITER PASSES * 65536
bits 16
org 0
%include "common.inc"
start:
    PRELUDE
    xor cx, cx
    mov esi, PASSES
%ifdef PROTECTED
    lgdt [cs:RM(gdtr)]
    smsw ax
    or ax, 1
    lmsw ax
%ifdef FLAT32
    jmp dword 8:LIN(count)
%else
    jmp 8:RM(count)
%endif
%endif
count:
%ifdef PROTECTED32
    bits 32
%endif
%ifdef DATA32
    mov ax, 0x10
    mov ds, ax
    mov es, ax
%endif
%ifdef FLAT32
    mov ss, ax
    mov esp, 0x90000
%endif
%ifdef PAGED32
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
    ; The page-table loop used CX; the iteration counter starts from zero.
    xor cx, cx
%endif
    jmp count_loop

; A fixed address lets the host check that execution is inside the loop.
    times 0x1000-($-$$) db 0
count_loop:
    inc ax
    add bx, ax
    xor dx, bx
    dec cx
    jnz count_loop
    dec esi
    jnz count_loop
%ifdef PROTECTED
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    WRITE_LINE LIN(end_text), END_ROW_ADDRESS
%else
    mov ax, 0xb800
    mov es, ax
    WRITE_LINE RM(end_text), END_ROW_ADDRESS - 0xb8000
%endif
    HALT_FOREVER

end_text: db END_TEXT, 0

%ifdef PROTECTED
align 8
gdt:
    dq 0
%ifdef FLAT32
    dq 0x00CF9A000000FFFF
%else
    ; 64 KiB executable segment based at the load address, present, ring 0.
    dw 0xffff, LOAD & 0xffff
%ifdef PROTECTED32
    db LOAD >> 16, 0x9a, 0x40, 0
%else
    db LOAD >> 16, 0x9a, 0, 0
%endif
%endif
    ; Flat data. The loop of the unpaged 32-bit segment case does not use it;
    ; every case loads it afterwards to store the END line.
    dq 0x00CF92000000FFFF
gdtr:
    dw $-gdt-1
    dd LIN(gdt)
%endif
