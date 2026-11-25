; SPDX-License-Identifier: AGPL-3.0-or-later
; BIOS Enhanced Disk Drive Services 3.0, INT 13h functions 08h/41h/42h/48h.
; See backlog/docs/design/windows95-on-486.md §5.
; Rebuild: nasm -f bin seabios-int13.asm -o seabios-int13.bin
; Host patches last LBA at boot offset 01E0h; results 0500h, EDD 0600h,
; extended sector 1000h, last reported CHS cylinder/head/sector 1200h.
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
    mov dl, 0x80
    mov ah, 8
    stc
    int 0x13
    mov [0x500], ax
    mov [0x502], cx
    mov [0x504], dx
    pushf
    pop word [0x506]
    mov ax, 0x4100
    mov bx, 0x55aa
    mov dl, 0x80
    stc
    int 0x13
    mov [0x508], ax
    mov [0x50a], bx
    mov [0x50c], cx
    pushf
    pop word [0x50e]
    mov word [0x600], 0x42
    mov si, 0x600
    mov dl, 0x80
    mov ah, 0x48
    stc
    int 0x13
    mov [0x510], ax
    pushf
    pop word [0x512]
    mov eax, [last_lba]
    mov [dap+8], eax
    mov si, dap
    mov dl, 0x80
    mov ah, 0x42
    stc
    int 0x13
    mov [0x514], ax
    pushf
    pop word [0x516]
    mov cx, [0x502]
    mov dx, [0x504]
    mov dl, 0x80
    mov bx, 0x1200
    mov ax, 0x0201
    stc
    int 0x13
    mov [0x518], ax
    pushf
    pop word [0x51a]
    mov word [0x51c], 0xcafe
    cli
    hlt
    jmp $
align 4
dap: db 0x10, 0
    dw 1, 0x1000, 0
    dq 0
    times 0x1e0-($-$$) db 0
last_lba: dd 0
    times 510-($-$$) db 0
    dw 0xaa55
