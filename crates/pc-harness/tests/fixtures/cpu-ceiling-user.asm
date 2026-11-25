; SPDX-License-Identifier: AGPL-3.0-or-later
; Intel 80386 Programmer's Reference Manual §5.1/§5.2 (segments/pages),
; §6.3 (privilege transitions), §7 (LDT/TSS), §15 (V86 and I/O permission).
; See windows95-on-486.md §3. Assemble with nasm -f bin and one of
; -DFLAT, -DFLAT -DPAGED, -DLDT -DPAGED, -DV86 -DPAGED,
; -DV86 -DPAGED -DTRAPPED, -DFLAT -DPAGED -DFARCALL. Tables are writable
; so accessed bits can be set.
cpu 386
bits 16
org 0xf8000
%define RM(x) (x - $$)
start:
    cli
    cld
    mov ax, cs
    mov ds, ax
    xor ax, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x8000
    mov si, RM(tables)
    mov di, 0x800
    mov cx, (tables_end-tables)/2
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
    mov esp, 0x8000
    xor eax, eax
    mov edi, 0x1000
    mov ecx, 0x1800
    rep stosd
    mov dword [0x1000], 0x2007
    mov edi, 0x2000
    mov eax, 3
    mov ecx, 256
.pages:
    stosd
    add eax, 0x1000
    loop .pages
    ; Only the loop, callee, data and user stacks are user accessible.
    mov dword [0x2000+0xfa*4], 0xfa007
    mov dword [0x2000+0xf9*4], 0xf9007
    mov dword [0x2000+0x40*4], 0x40007
    mov dword [0x2000+0x47*4], 0x47007
    mov dword [0x2000+8*4], 0x8007
    mov dword [0x5004], 0x7000
    mov word [0x5008], 0x10
    ; A bitmap beyond the TSS limit denies every port at IOPL zero.
    mov word [0x5066], 0x68
    mov ax, 0x30
    ltr ax
    mov ax, 0x28
    lldt ax
    mov word [0x3000+13*8], (0xf8000+RM(gp_handler)) & 0xffff
    mov word [0x3000+13*8+2], 8
    mov dword [0x3000+13*8+4], ((0xf8000+RM(gp_handler)) & 0xffff0000) | 0x8e00
    lidt [idtr]
%ifdef PAGED
    mov eax, 0x1000
    mov cr3, eax
    mov eax, cr0
    or eax, 0x80000000
    mov cr0, eax
%endif
    mov dword [0x40100], 0
    mov dword [0x40104], 0
    xor eax, eax
    xor ebx, ebx
    xor ecx, ecx
    xor edx, edx
    xor ebp, ebp
    xor esi, esi
    xor edi, edi
%ifdef V86
    push dword 0x4000
    push dword 0x4000
    push dword 0x4000
    push dword 0x4000
    push dword 0x4000
    push dword 0x8000
%ifdef TRAPPED
    push dword 0x20002
%else
    push dword 0x23002
%endif
    push dword 0xfa00
    push dword 0
%elifdef LDT
    push dword 0x1f
    push dword 0x8000
    push dword 2
    push dword 0x0f
    push dword 0x2000
%else
    mov bx, 0x23
    mov ds, bx
    mov es, bx
    push dword 0x23
    push dword 0x8800
    push dword 2
    push dword 0x1b
    push dword user_loop
%endif
    iretd

; The faulting IN is two bytes. Emulate its result and advance the saved IP;
; IRETD restores all six V86 segments and the original user stack.
gp_handler:
    push eax
    mov ax, 0x10
    mov ds, ax
    inc dword [0x6000]
    add dword [esp+8], 2
    pop eax
    mov al, 0x5a
    add esp, 4
    iretd

align 8
tables:
    dq 0
    dq 0x00cf9a000000ffff
    dq 0x00cf92000000ffff
    dq 0x00cffa000000ffff
    dq 0x00cff2000000ffff
    dq 0x000082000900001f
    dq 0x0000890050000067
    times 0x100-($-tables) db 0
    dq 0
    dq 0x0000fa0f8000ffff
    dq 0x0000fa0f9000ffff
    dq 0x0000f2040000ffff
tables_end:
gdtr: dw 7*8-1
    dd 0x800
idtr: dw 0x7ff
    dd 0x3000

    times 0x1000-($-$$) db 0
%ifdef FLAT
bits 32
%else
bits 16
%endif
xor_callee:
%ifdef FLAT
    xor edx, eax
%else
    xor dx, ax
%endif
%ifndef FLAT
    retf
%elifdef FARCALL
    retf
%else
    ret
%endif

    times 0x2000-($-$$) db 0
user_loop:
%macro BODY 0
%ifdef FLAT
    ; Per 100 retired: representative Win32 has 8 near CALL/RET pairs and no
    ; far transfers or segment reloads. FARCALL stress has 4 near pairs and
    ; 4 far CALL/RETF pairs, retaining the same 25-instruction period.
    mov ebx, 0x40100
    mov eax, [ebx]
    add eax, 3
    mov [ebx], eax
    push eax
    xor eax, 0x55aa55aa
    pop eax
    call near_callee
%ifdef FARCALL
    call 0x1b:xor_callee
%else
    call xor_callee
%endif
    mov [ebx+4], eax
    inc ebp
    add esi, eax
    xor edi, esi
    rol edx, 1
    add ecx, 7
    xor ecx, 0x1234
    add esi, ecx
    xor edi, edx
    ror ecx, 1
    inc ebx
%else
    ; Protected16 per 100 retired: 4 near pairs, 4 far pairs, 4 DS and 4 ES
    ; reloads. V86 per 100: 4 of each pair/reload (ten bodies per 250 retired).
    ; IOPL 0 adds 0.4 monitor DS loads and 0.4 IRETD segment restores per 100.
%ifdef V86
    mov bx, 0x3000
    add bx, 0x1000
%else
    mov bx, 0x1f
    add bx, 0
%endif
    mov ds, bx
    mov es, bx
    mov ax, [0x100]
    add ax, 3
    mov [0x100], ax
    push ax
    xor ax, 0x55aa
    pop ax
    call near_callee
%ifdef V86
    call 0xf900:0
%else
    call 0x17:0
%endif
    mov [es:0x102], ax
    inc ebp
    add si, ax
    xor di, si
    rol dx, 1
    add cx, 7
    xor cx, 0x1234
    inc bx
%endif
%endmacro
%ifdef V86
%rep 10
    BODY
%endrep
%ifdef TRAPPED
    in al, 0x80
%else
    ; Match the monitor's nine retired instructions and result so both V86
    ; samples perform identical user work with the same loop branches.
    mov al, 0x5a
    times 8 nop
%endif
%else
    BODY
%endif
    jmp user_loop
near_callee:
%ifdef FLAT
    add eax, 1
%else
    add ax, 1
%endif
    ret
bits 16
    times 0x7ff0-($-$$) db 0
    jmp 0xf800:0
    times 0x8000-($-$$) db 0
