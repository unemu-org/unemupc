; SPDX-License-Identifier: AGPL-3.0-or-later
; SeaBIOS rel-1.17.0, system.c handle_15: query memory after floppy boot.
; See backlog/docs/design/machine-files.md, Generic 386 ISA board.
; Rebuild: nasm -f bin seabios-int15.asm -o seabios-int15.bin
; Results: 0500h = AX/FLAGS from AH=88h; 0504h = AX/BX/CX/DX/FLAGS
; from E801h; 0510h = end of E820 records; 0512h = completion marker.
; E820 records at 0600h: 20-byte entry, EAX/EBX/ECX, FLAGS, padding.
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
    ; A zero CMOS legacy-size word must not replace the E820-derived size.
    mov al, 0x30
    out 0x70, al
    xor al, al
    out 0x71, al
    mov al, 0x31
    out 0x70, al
    xor al, al
    out 0x71, al
    mov di, 0x500
    mov ax, 0x8800
    stc
    int 0x15
    stosw
    pushf
    pop ax
    stosw
    mov ax, 0xe801
    stc
    int 0x15
    stosw
    mov ax, bx
    stosw
    mov ax, cx
    stosw
    mov ax, dx
    stosw
    pushf
    pop ax
    stosw
    mov di, 0x600
    xor ebx, ebx
next:
    mov eax, 0xe820
    mov edx, 0x534d4150
    mov ecx, 20
    stc
    int 0x15
    mov [di+20], eax
    mov [di+24], ebx
    mov [di+28], ecx
    pushf
    pop ax
    mov [di+32], ax
    add di, 36
    test ebx, ebx
    jz done
    cmp di, 0xa80
    jb next
done:
    mov [0x510], di
    mov word [0x512], 0xcafe
halt:
    cli
    hlt
    jmp halt
    times 510-($-$$) db 0
    dw 0xaa55
