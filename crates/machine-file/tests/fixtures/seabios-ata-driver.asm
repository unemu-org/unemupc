; SPDX-License-Identifier: AGPL-3.0-or-later
; ATA-2 §§7, 8.3–8.4: PIO multiple blocks, interrupt acknowledgement.
; Intel 80386 Programmer's Reference Manual, §9.4 IDT; Intel 8259A, ICWs/EOI.
; See backlog/docs/design/windows95-on-486.md §5.
; Rebuild: nasm -f bin seabios-ata-driver.asm -o seabios-ata-driver.bin
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
    mov ax, 0x0203
    mov cx, 2
    xor dh, dh
    mov bx, 0x7e00
    int 0x13
    jc boot_fail
    jmp start
boot_fail:
    cli
    hlt
    jmp $
    times 510-($-$$) db 0
    dw 0xaa55
start:
    cli
    mov al, 2
    out 0x92, al
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
    mov edi, 0x500
    mov ecx, 64
    rep stosd
    mov edi, 0x2000
    mov ecx, 512
    rep stosd
    mov word [0x2000+0x2e*8], irq14
    mov word [0x2000+0x2e*8+2], 8
    mov word [0x2000+0x2e*8+4], 0x8e00
    lidt [idtr]
    mov al, 0x11
    out 0x20, al
    out 0xa0, al
    mov al, 0x20
    out 0x21, al
    mov al, 0x28
    out 0xa1, al
    mov al, 4
    out 0x21, al
    mov al, 2
    out 0xa1, al
    mov al, 1
    out 0x21, al
    out 0xa1, al
    mov al, 0xfb
    out 0x21, al
    mov al, 0xbf
    out 0xa1, al
    mov dx, 0x3f6
    xor al, al
    out dx, al
    mov dx, 0x1f6
    mov al, 0xe0
    out dx, al
    xor ebx, ebx
    sti
    mov al, 0xec
    call command
    call wait_irq
    mov edi, 0x1000
    mov ecx, 256
    mov dx, 0x1f0
    rep insw
    mov dx, 0x1f2
    mov al, 2
    out dx, al
    mov al, 0xc6
    call command
    call wait_irq
    mov al, 4
    call address
    mov al, 0xc4
    call command
    call wait_irq
    mov edi, 0x1800
    call read_block
    call wait_irq
    call read_block
    mov edi, 0x3000
    mov ecx, 1024
    mov ax, 0x1234
fill:
    stosw
    inc ax
    loop fill
    mov al, 8
    call address
    mov al, 0xc5
    call command
    ; The first write block is polled; subsequent blocks and final completion IRQ.
    mov dx, 0x3f6
write_ready:
    in al, dx
    mov [0x512], al
    ; ATA-2 §8.4: DRQ and ERR are meaningful only after BSY clears.
    test al, 0x80
    jnz write_ready
    test al, 1
    jnz driver_fail
    test al, 8
    jz write_ready
    mov esi, 0x3000
    call write_block
    call wait_irq
    call write_block
    call wait_irq
    mov al, 8
    call address
    mov al, 0xc4
    call command
    call wait_irq
    mov edi, 0x4000
    call read_block
    call wait_irq
    call read_block
    mov dx, 0x1f6
    mov al, 0xaf
    out dx, al
    mov dx, 0x1f2
    mov al, 63
    out dx, al
    mov al, 0x91
    call command
    call wait_irq
    mov dx, 0x1f1
    mov al, 3
    out dx, al
    inc dx
    mov al, 0x0c
    out dx, al
    mov al, 0xef
    call command
    call wait_irq
    mov al, 0x10
    call command
    call wait_irq
    mov dx, 0x1f6
    mov al, 0xb0
    out dx, al
    inc dx
    in al, dx
    mov [0x508], al
    mov dx, 0x1f1
    in al, dx
    mov [0x509], al
    mov al, 0xec
    call command
    mov dx, 0x1f7
    in al, dx
    mov [0x50a], al
    mov dx, 0x177
    in al, dx
    mov [0x50b], al
    mov dx, 0x376
    in al, dx
    mov [0x510], al
    mov dx, 0x1f6
    mov al, 0xa0
    out dx, al
    mov al, 0xa1
    call command
    call wait_irq
    mov dx, 0x1f1
    in al, dx
    mov [0x511], al
    mov eax, cr0
    mov [0x50c], eax
    mov word [0x500], 0xcafe
driver_fail:
    cli
    hlt
    jmp $
command:
    mov dx, 0x1f7
    out dx, al
    ret
address:
    mov dx, 0x1f3
    out dx, al
    inc dx
    xor al, al
    out dx, al
    inc dx
    out dx, al
    inc dx
    mov al, 0xe0
    out dx, al
    mov dx, 0x1f2
    mov al, 4
    out dx, al
    ret
wait_irq:
    inc ebx
wait_loop:
    cmp [0x504], ebx
    jb wait_loop
    ret
read_block:
    mov ecx, 512
    mov dx, 0x1f0
    rep insw
    ret
write_block:
    mov ecx, 512
    mov dx, 0x1f0
    rep outsw
    ret
irq14:
    push eax
    push edx
    push edi
    mov dx, 0x1f7
    in al, dx
    mov edi, [0x504]
    mov [edi+0x600], al
    inc dword [0x504]
    mov al, 0x20
    out 0xa0, al
    out 0x20, al
    pop edi
    pop edx
    pop eax
    iretd
align 8
gdt: dq 0, 0x00cf9a000000ffff, 0x00cf92000000ffff
gdtr: dw $-gdt-1
    dd gdt
idtr: dw 2047
    dd 0x2000
    times 2048-($-$$) db 0
