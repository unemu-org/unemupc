; SPDX-License-Identifier: AGPL-3.0-or-later
; IBM VGA/XGA Technical Reference, Video BIOS: modes 03h/13h and teletype.
; Rebuild: nasm -f bin seabios-int10.asm -o seabios-int10.bin
; 0500h marks each stage; a bounded delay makes text observable to the host.
bits 16
org 0x7c00
    jmp 0:entry
entry:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00
    cld
    sti
    mov ax, 3
    int 0x10
    mov si, message
next:
    lodsb
    test al, al
    jz text_ready
    mov ah, 0x0e
    mov bx, 7
    int 0x10
    jmp next
text_ready:
    cli
    mov word [0x500], 1
    mov cx, 0xffff
wait_capture:
    loop wait_capture
    mov ax, 0x13
    int 0x10
    mov ax, 0xa000
    mov es, ax
    xor di, di
    xor ax, ax
    mov cx, 64000
pixel:
    stosb
    inc al
    loop pixel
    mov word [0x500], 2
halt:
    cli
    hlt
    jmp halt
message:
    db 'SeaVGABIOS INT 10h mode 3', 0
    times 510-($-$$) db 0
    dw 0xaa55
