; SPDX-License-Identifier: AGPL-3.0-or-later
; Project-owned bootable VGA exerciser. IBM VGA/XGA Technical Reference
; Manual (May 1992), Figs. 2-32, 2-36, 2-58, 2-60, 2-61, 2-64,
; 2-65, 2-69 and 2-75: map masks, byte addressing and latch copies.
; See backlog/docs/design/vga-modex-test.md for the independent pixel oracle.
; National Semiconductor PC16550D datasheet, LCR, LSR and THR: COM1 output.
cpu 286
bits 16
LOAD_SEG equ 0x0800
PROG_SECTORS equ 4
IMAGE_SIZE equ 1228800
PAGE_BYTES equ 90 * 240
section boot start=0 vstart=0x7C00

boot:
        cli
        xor ax, ax
        mov ds, ax
        mov ss, ax
        mov sp, 0x7C00
        sti
        cld
        mov [boot_drive], dl
        mov si, 3
.retry:
        mov ax, LOAD_SEG
        mov es, ax
        xor bx, bx
        mov ax, 0x0200 | PROG_SECTORS
        mov cx, 0x0002
        xor dh, dh
        mov dl, [boot_drive]
        int 0x13
        jnc .loaded
        xor ax, ax
        mov dl, [boot_drive]
        int 0x13
        dec si
        jnz .retry
        mov si, boot_error
.print:
        lodsb
        test al, al
        jz .halt
        mov ah, 0x0E
        mov bx, 0x0007
        int 0x10
        jmp .print
.halt:
        cli
        hlt
        jmp .halt
.loaded:
        jmp LOAD_SEG:start

boot_drive      db 0
boot_error      db 'vga-modex-test: boot read error', 13, 10, 0

        times 510 - ($ - $$) db 0
        dw 0xAA55

section prog follows=boot vstart=0
prog_start:
start:
        cli
        cld
        mov ax, cs
        mov ds, ax
        mov ss, ax
        mov sp, 0xFFF0
        call uart_init
        mov si, header
        call print
        mov ax, 0x0013
        int 0x10
        cli
        push cs
        pop ds
        mov dx, 0x3C4
        mov ax, 0x0604
        out dx, ax
        mov ax, 0x0100
        out dx, ax
        mov dx, 0x3C2
        mov al, 0xE7
        out dx, al
        mov dx, 0x3C4
        mov ax, 0x0300
        out dx, ax
        mov dx, 0x3D4
        mov al, 0x11
        out dx, al
        inc dx
        in al, dx
        and al, 0x7F
        out dx, al
        dec dx
        mov si, crtc
        mov cx, (crtc_end - crtc) / 2
.crtc:
        lodsw
        out dx, ax
        loop .crtc
        mov dx, 0x3C4
        mov ax, 0x0F02
        out dx, ax
        mov ax, 0xA000
        mov es, ax
        xor di, di
        xor ax, ax
        mov cx, 0x8000
        rep stosw
        mov word [page_base], 0
        mov byte [page_colour], 0
.page:
        mov byte [plane], 0
.plane:
        mov cl, [plane]
        mov ah, 1
        shl ah, cl
        mov al, 2
        mov dx, 0x3C4
        out dx, ax
        mov di, [page_base]
        xor bp, bp
.row:
        xor bx, bx
.column:
        mov ax, bx
        shl ax, 2
        add al, [plane]
        mov si, ax
        xor ax, bp
        and al, 63
        add al, [page_colour]
        cmp bp, 200
        jb .pixel
        mov ax, si
        and al, 7
        cmp al, 5
        jae .paper
        mov ax, bp
        and al, 7
        cmp al, 7
        je .paper
        test si, 7
        jz .ink
        cmp al, 0
        je .ink
        cmp al, 3
        je .ink
        cmp al, 6
        je .ink
.paper:
        mov al, [page_colour]
        add al, 2
        jmp .pixel
.ink:
        mov al, 0xF1
.pixel:
        stosb
        inc bx
        cmp bx, 90
        jb .column
        inc bp
        cmp bp, 240
        jb .row
        inc byte [plane]
        cmp byte [plane], 4
        jb .plane
        ; A VGA read loads all four latches, irrespective of Read Map Select.
        ; Write mode 1 stores those latches rather than the CPU's byte.
        mov dx, 0x3C4
        mov ax, 0x0F02
        out dx, ax
        mov dx, 0x3CE
        mov ax, 0x4105
        out dx, ax
        mov si, 24 * 90 + 16 / 4
        mov di, 112 * 90 + 224 / 4
        add si, [page_base]
        add di, [page_base]
        mov bp, 32
.copy_row:
        mov cx, 64 / 4
.copy:
        mov al, [es:si]
        xor al, 0xFF
        mov [es:di], al
        inc si
        inc di
        loop .copy
        add si, 90 - 64 / 4
        add di, 90 - 64 / 4
        dec bp
        jnz .copy_row
        mov ax, 0x4005
        out dx, ax
        cmp word [page_base], 0
        jne .drawn
        mov word [page_base], PAGE_BYTES
        mov byte [page_colour], 64
        jmp .page
.drawn:
        mov si, ready
        call print
        xor ax, ax
        mov es, ax
        mov word [es:0x600], 0
.frame:
        mov dx, 0x3DA
.wait_clear:
        in al, dx
        test al, 8
        jnz .wait_clear
.wait_set:
        in al, dx
        test al, 8
        jz .wait_set
        inc word [es:0x600]
        mov bx, [es:0x600]
        dec bx
        and bx, 1
        mov ax, PAGE_BYTES
        mul bx
        mov bx, ax
        mov dx, 0x3D4
        mov al, 0x0C
        mov ah, bh
        out dx, ax
        mov al, 0x0D
        mov ah, bl
        out dx, ax
        ; IBM Fig. 2-53 latches at retrace START. A write after that edge
        ; becomes visible at the following retrace, not at this pulse's end.
        mov dx, 0x3DA
.wait_end:
        in al, dx
        test al, 8
        jnz .wait_end
.wait_latch:
        in al, dx
        test al, 8
        jz .wait_latch
.wait_visible:
        in al, dx
        test al, 8
        jnz .wait_visible
        mov ax, [es:0x600]
        dec ax
        and al, 1
        add al, '0'
        call put_char
        cmp word [es:0x600], 60
        jb .frame
        mov si, done
        call print
.halt:
        cli
        hlt
        jmp .halt

uart_init:
        mov dx, 0x3FB
        mov al, 0x80
        out dx, al
        mov dx, 0x3F8
        mov al, 1
        out dx, al
        inc dx
        xor al, al
        out dx, al
        mov dx, 0x3FB
        mov al, 3
        out dx, al
        mov dx, 0x3F9
        xor al, al
        out dx, al
        ret

put_char:
        pusha
        mov bl, al
        mov dx, 0x3FD
.thre:
        in al, dx
        test al, 0x20
        jz .thre
        mov dx, 0x3F8
        mov al, bl
        out dx, al
        popa
        ret

print:
        lodsb
        test al, al
        jz .end
        call put_char
        jmp print
.end:
        ret

header db 'vga-modex-test', 13, 10, 0
ready db 'pattern ready', 13, 10, 0
done db 13, 10, 'frames 60 done', 13, 10, 0
page_base dw 0
page_colour db 0
plane db 0
; Bridges/Abrash 360-dot horizontal timing plus the 240-row Mode X timing.
; CR09=41h repeats each row twice within the 480-line display interval.
crtc:
        dw 0x6B00, 0x5901, 0x5A02, 0x8E03, 0x5E04, 0x8A05
        dw 0x0D06, 0x3E07, 0x0008, 0x4109
        dw 0x000C, 0x000D, 0xEA10, 0xAC11, 0xDF12, 0x2D13
        dw 0x0014, 0xE715, 0x0616, 0xE317, 0xFF18
crtc_end:
prog_end:
        times PROG_SECTORS * 512 - (prog_end - prog_start) db 0
        times IMAGE_SIZE - 512 - PROG_SECTORS * 512 db 0
