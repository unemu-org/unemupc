; SPDX-License-Identifier: AGPL-3.0-or-later
; Open mode 12h attribute-palette compatibility exerciser.
; IBM VGA/XGA Technical Reference Manual (May 1992), Figs. 2-32, 2-75,
; 2-81–2-83 and 2-94: plane writes, attribute addressing and DAC data.
; PAS=1 writes are a compatibility policy, not a verified hardware fact.
; National Semiconductor PC16550D datasheet, LCR, LSR and THR: COM1 output.
; See backlog/docs/design/vga-modex-test.md for the independent colour oracle.
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
boot_error      db 'vga-pas-test: boot read error', 13, 10, 0

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
        mov ax, 0x0012
        int 0x10
        cli
        push cs
        pop ds
        ; Distinct DAC colours expose any palette entry left at BIOS defaults.
        mov dx, 0x3C8
        xor al, al
        out dx, al
        inc dx
        xor bx, bx
.dac:
        mov al, bl
        out dx, al
        mov al, 63
        sub al, bl
        out dx, al
        mov al, bl
        xor al, 21
        out dx, al
        inc bx
        cmp bx, 64
        jb .dac
        ; Compatibility sequence observed in Windows 3.x vga.drv: PAS stays set.
        mov dx, 0x3DA
        in al, dx
        mov dx, 0x3C0
        xor bx, bx
.palette:
        mov al, bl
        or al, 0x20
        out dx, al
        mov al, bl
        out dx, al
        inc bx
        cmp bx, 16
        jb .palette
        mov ax, 0xA000
        mov es, ax
        xor bx, bx
.bar:
        mov dx, 0x3C4
        mov ax, 0x0102
.plane:
        out dx, ax
        push ax
        test bl, ah
        mov al, 0
        jz .fill
        dec al
.fill:
        mov di, bx
        shl di, 2
        add di, bx
        mov bp, 480
.row:
        mov cx, 5
        rep stosb
        add di, 75
        dec bp
        jnz .row
        pop ax
        shl ah, 1
        cmp ah, 16
        jb .plane
        inc bx
        cmp bx, 16
        jb .bar
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

done db 'vga-pas-test: bars ready', 13, 10, 0
prog_end:
        times PROG_SECTORS * 512 - (prog_end - prog_start) db 0
        times IMAGE_SIZE - 512 - PROG_SECTORS * 512 db 0
