; SPDX-License-Identifier: AGPL-3.0-or-later
; APM BIOS Interface Specification 1.2, §§4.6.1, 4.6.4, 4.6.6, 4.6.8:
; installation, protected connection, CPU idle and Set Power State (off).
; See backlog/docs/design/windows95-on-486.md §5.
; Rebuild: nasm -f bin seabios-apm.asm -o seabios-apm.bin
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
    mov ax, 0x0201
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
    mov ax, 0x5300
    xor bx, bx
    stc
    int 0x15
    mov [0x500], ax
    mov [0x502], bx
    mov [0x504], cx
    pushf
    pop word [0x506]
    mov ax, 0x5303
    xor bx, bx
    stc
    int 0x15
    mov [0x508], ax
    mov [0x50a], ebx
    mov [0x50e], cx
    mov [0x510], dx
    mov [0x512], esi
    mov [0x516], di
    pushf
    pop word [0x518]
    mov [apm_pointer], ebx
    ; APM requires adjacent 32-bit code, 16-bit code and data descriptors.
    movzx eax, ax
    shl eax, 4
    mov [gdt+24+2], ax
    shr eax, 16
    mov [gdt+24+4], al
    movzx eax, cx
    shl eax, 4
    mov [gdt+32+2], ax
    shr eax, 16
    mov [gdt+32+4], al
    movzx eax, dx
    shl eax, 4
    mov [gdt+40+2], ax
    shr eax, 16
    mov [gdt+40+4], al
    mov [gdt+24], si
    dec word [gdt+24]
    mov eax, esi
    shr eax, 16
    mov [gdt+32], ax
    dec word [gdt+32]
    mov [gdt+40], di
    dec word [gdt+40]
    cli
    lgdt [gdtr]
    mov eax, cr0
    or al, 1
    mov cr0, eax
    jmp 8:protected
bits 32
protected:
    mov ax, 16
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov esp, 0x7000
    xor eax, eax
    mov edi, 0x2000
    mov ecx, 512
    rep stosd
    mov word [0x2000+8*8], timer
    mov word [0x2000+8*8+2], 8
    mov word [0x2000+8*8+4], 0x8e00
    lidt [idtr]
    mov al, 0xfe
    out 0x21, al
    mov al, 0xff
    out 0xa1, al
    mov ax, 0x530e
    xor ebx, ebx
    mov cx, 0x0102
    stc
    call far [apm_pointer]
    mov [0x51a], ax
    pushfd
    pop dword [0x51c]
    mov ax, 0x5305
    stc
    call far [apm_pointer]
    mov [0x520], ax
    pushfd
    pop dword [0x522]
    mov word [0x526], 0xcafe
    mov ax, 0x5307
    mov bx, 1
    mov cx, 3
    call far [apm_pointer]
    ; A returning power-off is a failure even if it subsequently halts here.
    mov word [0x528], 0xdead
    cli
    hlt
    jmp $
timer:
    push eax
    inc word [0x52a]
    mov al, 0x20
    out 0x20, al
    pop eax
    iretd
align 8
gdt: dq 0, 0x00cf9a000000ffff, 0x00cf92000000ffff
    dq 0x00409a000000ffff, 0x00009a000000ffff, 0x000092000000ffff
gdtr: dw $-gdt-1
    dd gdt
idtr: dw 2047
    dd 0x2000
apm_pointer: dd 0
    dw 24
    times 1024-($-$$) db 0
