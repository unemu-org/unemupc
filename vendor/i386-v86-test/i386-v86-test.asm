; SPDX-License-Identifier: AGPL-3.0-or-later
;
; Intel 80386 Programmer's Reference Manual, Chapter 15 Virtual 8086 Mode,
; sections 15.3 Entering and Leaving V86 Mode, 15.4 IOPL-Sensitive
; Instructions and 15.5 I/O Permission Map. See backlog/docs/design/protected-mode-gaps.md §V86.
; Writable tables live in RAM because the CPU sets descriptor accessed bits.
cpu 386
bits 16
org 0xE0000
%define RM(x) (x - $$ - 0x10000)
TSS equ 0x5000
GP_COUNT equ 0x6000
REFLECTIONS equ 0x6004

times 0x10000-($-$$) db 0xFF
start:
    cli
    cld
    mov ax, cs
    mov ds, ax
    xor ax, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x8000
    mov word [es:0x100], RM(reflected)
    mov word [es:0x102], 0xF000
    mov si, RM(gdt)
    mov di, 0x800
    mov cx, (gdt_end-gdt)/2
    rep movsw
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
    mov esp, 0x9000
    xor eax, eax
    mov edi, TSS
    mov ecx, 0x201
    rep stosb
    mov dword [TSS+4], 0x9000
    mov word [TSS+8], 0x10
    mov word [TSS+0x66], 0x68
    mov al, 0xFF
    mov edi, TSS+0x68
    mov ecx, 0x201-0x68
    rep stosb
    ; OUT 402h must consult the bitmap even while IOPL is three.
    mov byte [TSS+0x68+(0x402/8)], 0xFB
    mov dword [GP_COUNT], 0
    mov dword [REFLECTIONS], 0
    mov ax, 0x18
    ltr ax
    mov edi, 0x1000+13*8
    mov eax, gp_handler
    mov dl, 0x8E
    call gate
    mov edi, 0x1000+0x40*8
    mov eax, reflect
    mov dl, 0xEE
    call gate
    mov edi, 0x1000+0x42*8
    mov eax, lower_iopl
    call gate
    mov edi, 0x1000+0x41*8
    mov eax, finished
    call gate
    lidt [idtr]
    mov esi, banner
    call puts32
    push dword 0x5000
    push dword 0x4000
    push dword 0xF000
    push dword 0x3000
    push dword 0x2000
    push dword 0x8000
    push dword 0x23002
    push dword 0xF000
    push dword RM(v86)
    iretd

gate:
    mov [edi], ax
    mov word [edi+2], 8
    mov byte [edi+4], 0
    mov [edi+5], dl
    shr eax, 16
    mov [edi+6], ax
    ret

; A monitor reflects INT 40h through its real-mode IVT entry. The protected
; frame stays nine dwords; a separate three-word frame lets the V86 handler
; return with the ordinary real-mode IRET instruction.
reflect:
    pushad
    mov ax, ds
    mov bx, es
    or ax, bx
    mov bx, fs
    or ax, bx
    mov bx, gs
    or ax, bx
    jnz failed
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    inc dword [REFLECTIONS]
    mov esi, [esp+32+12]
    sub si, 6
    mov edi, [esp+32+16]
    shl edi, 4
    movzx eax, si
    add edi, eax
    mov eax, [esp+32]
    mov [edi], ax
    mov eax, [esp+32+4]
    mov [edi+2], ax
    mov eax, [esp+32+8]
    mov [edi+4], ax
    mov [esp+32+12], esi
    movzx eax, word [0x100]
    mov [esp+32], eax
    movzx eax, word [0x102]
    mov [esp+32+4], eax
    popad
    iretd

lower_iopl:
    and dword [esp+8], ~0x3000
    iretd

gp_handler:
    pushad
    mov ax, 0x10
    mov ds, ax
    cmp dword [esp+32], 0
    jne failed
    mov edi, [esp+32+8]
    shl edi, 4
    add edi, [esp+32+4]
    cmp dword [GP_COUNT], 0
    jne .io
    cmp byte [edi], 0xFA
    jne failed
    inc dword [esp+32+4]
    jmp .resume
.io:
    cmp dword [GP_COUNT], 1
    jne failed
    cmp word [edi], 0x80E4
    jne failed
    add dword [esp+32+4], 2
    or dword [esp+32+12], 0x3000
.resume:
    inc dword [GP_COUNT]
    popad
    add esp, 4
    iretd

finished:
    mov ax, 0x10
    mov ds, ax
    cmp dword [GP_COUNT], 2
    jne failed
    cmp dword [REFLECTIONS], 1
    jne failed
    mov esi, done
    call puts32
    cli
    hlt
    jmp $
failed:
    mov ax, 0x10
    mov ds, ax
    mov esi, failure
    call puts32
    cli
    hlt
    jmp $
puts32:
    mov dx, 0x402
.next:
    lodsb
    test al, al
    jz .end
    out dx, al
    jmp .next
.end:
    ret

bits 16
v86:
    mov si, RM(real_code)
    call puts16
    int 0x40
    mov si, RM(resumed)
    call puts16
    int 0x42
    cli
    mov si, RM(iopl)
    call puts16
    in al, 0x80
    mov si, RM(bitmap)
    call puts16
    int 0x41
    jmp $
reflected:
    mov si, RM(reflected_text)
    call puts16
    iret
puts16:
    mov dx, 0x402
.next:
    lodsb
    test al, al
    jz .end
    out dx, al
    jmp .next
.end:
    ret

align 8
gdt:
    dq 0
    db 0xFF,0xFF,0,0,0,0x9A,0xCF,0
    db 0xFF,0xFF,0,0,0,0x92,0xCF,0
    db 0x00,0x02,0x00,0x50,0,0x89,0,0
gdt_end:
gdtr: dw gdt_end-gdt-1
    dd 0x800
idtr: dw 0x7FF
    dd 0x1000
banner: db 'i386-v86-test',10,0
real_code: db 'v86 real code ok',10,0
reflected_text: db 'reflected int ok',10,0
resumed: db 'v86 resumed ok',10,0
iopl: db 'iopl gp(0) ok',10,0
bitmap: db 'bitmap allow deny ok',10,0
done: db 'ring0 return ok',10,'failures 0000',10,0
failure: db 'FAIL',10,0

times 0x1FFF0-($-$$) db 0xFF
    jmp 0xF000:RM(start)
times 0x20000-($-$$) db 0xFF
