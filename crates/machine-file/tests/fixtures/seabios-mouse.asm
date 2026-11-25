; SPDX-License-Identifier: AGPL-3.0-or-later
; IBM PS/2 Hardware Interface Technical Reference, Auxiliary Device Interface
; and Pointing Device, stream packet and commands; BIOS INT 15h C2h.
; See backlog/docs/design/windows95-on-486.md §5.
; Rebuild: nasm -f bin seabios-mouse.asm -o seabios-mouse.bin
bits 16
org 0x7c00
    jmp 0:boot
boot:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00
    cld
    mov ax, 0x0202
    mov cx, 2
    xor dh, dh
    mov bx, 0x7e00
    int 0x13
    jc fail
    jmp start
fail:
    cli
    hlt
    jmp $
    times 510-($-$$) db 0
    dw 0xaa55
start:
    mov ax, 0xc205
    mov bh, 3
    stc
    int 0x15
    mov [0x502], ax
    pushf
    pop word [0x504]
    mov ax, 0xc207
    mov bx, callback
    stc
    int 0x15
    mov [0x506], ax
    pushf
    pop word [0x508]
    mov ax, 0xc200
    mov bh, 1
    stc
    int 0x15
    mov [0x50a], ax
    pushf
    pop word [0x50c]
    mov word [0x500], 1
    sti
bios_wait:
    cmp word [0x52a], 1
    jb bios_wait
    mov ax, 0xc200
    xor bh, bh
    stc
    int 0x15
    mov [0x50e], ax
    pushf
    pop word [0x510]
    cli
    mov al, 0xff
    out 0xa1, al
    out 0x21, al
    ; Suppress auxiliary IRQs while command replies are consumed by polling.
    mov al, 0x20
    out 0x64, al
    call read_output
    and al, 0xdd
    mov bl, al
    mov al, 0x60
    call controller
    mov al, bl
    out 0x60, al
    mov al, 0xa8
    call controller
    mov si, 0x600
    mov di, 0x610
    mov al, 0xff
    call aux
    call reply
    call reply
    call reply
    mov al, 0xf3
    call aux
    call reply
    mov al, 100
    call aux
    call reply
    mov al, 0xf4
    call aux
    call reply
    mov al, 0xf2
    call aux
    call reply
    call reply
    mov word [0x74*4], irq12
    mov word [0x74*4+2], 0
    mov al, 0x20
    call controller
    call read_output
    and al, 0xde
    or al, 2
    mov bl, al
    mov al, 0x60
    call controller
    mov al, bl
    out 0x60, al
    mov al, 0xfb
    out 0x21, al
    mov al, 0xef
    out 0xa1, al
    mov word [0x500], 2
    sti
direct_wait:
    cmp word [0x528], 3
    jb direct_wait
    mov word [0x500], 0xcafe
    cli
    hlt
    jmp $
controller:
    push ax
input_wait:
    in al, 0x64
    test al, 2
    jnz input_wait
    pop ax
    out 0x64, al
    ret
aux:
    push ax
    mov al, 0xd4
    call controller
    pop ax
    out 0x60, al
    ret
read_output:
    in al, 0x64
    test al, 1
    jz read_output
    in al, 0x60
    ret
reply:
    in al, 0x64
    test al, 1
    jz reply
    mov [si], al
    inc si
    in al, 0x60
    stosb
    ret
callback:
    push bp
    mov bp, sp
    push ax
    mov ax, [ss:bp+12]
    mov [cs:0x530], ax
    mov ax, [ss:bp+10]
    mov [cs:0x532], ax
    mov ax, [ss:bp+8]
    mov [cs:0x534], ax
    mov ax, [ss:bp+6]
    mov [cs:0x536], ax
    inc word [cs:0x52a]
    pop ax
    pop bp
    retf
irq12:
    push ax
    push bx
    in al, 0x64
    mov ah, al
    and al, 0x21
    cmp al, 0x21
    jne eoi
    mov bx, [0x528]
    mov [bx+0x620], ah
    in al, 0x60
    mov [bx+0x630], al
    inc word [0x528]
eoi:
    mov al, 0x20
    out 0xa0, al
    out 0x20, al
    pop bx
    pop ax
    iret
    times 1536-($-$$) db 0
