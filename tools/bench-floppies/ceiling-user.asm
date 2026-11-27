; SPDX-License-Identifier: AGPL-3.0-or-later
; Bootable copy of the user-mode speed-ceiling loops, loaded to LOAD (10000h)
; by boot.asm. Intel 80386 Programmer's Reference Manual §5.1/§5.2 (segments
; and pages), §6.3 (privilege transitions), §7 (LDT/TSS), §9 (exceptions) and
; §15 (V86 and I/O permission).
;
; Assemble with nasm -f bin, -DPASSES=<n> and -DCASE_NAME='<name>', plus:
;   flat-paged32-cpl3          -DFLAT -DPAGED
;   flat-paged32-cpl3-farcall  -DFLAT -DPAGED -DFARCALL
;   paged-protected16-cpl3     -DLDT -DPAGED
;   paged-v86                  -DV86 -DPAGED
;   paged-v86-iopl0            -DV86 -DPAGED -DTRAPPED
;
; The loop bodies are the ROM fixture's. The ROM sits at F8000h with its
; callee at F9000h and its loop at FA000h; this copy keeps the same layout at
; the load address (setup at 10000h, callee at 11000h, loop at 12000h), so
; every segment base, page-table entry and V86 segment moves by the same
; amount. The low-memory tables (GDT 800h, LDT 900h, page directory 1000h,
; page table 2000h, IDT 3000h, TSS 5000h, monitor counter 6000h, ring-0 stack
; below 8000h) and the user data at 40000h are unchanged; a booted machine
; leaves them free once the BIOS is done.
;
; A ROM can loop forever, a timed boot cannot. EBP already counts bodies, so
; the closing JMP becomes CMP EBP / JB: one more retired instruction per pass.
; After the last pass the user code executes UD2; its invalid-opcode fault
; enters ring 0, which stores the END line into text memory and halts.
cpu 386
%ifdef V86
%assign BODIES 10
%else
%assign BODIES 1
%endif
%assign ITER PASSES
bits 16
%include "common.inc"
org LOAD
start:
    PRELUDE
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
    mov dword [0x2000+(LIN(user_loop)>>12)*4], LIN(user_loop) | 7
    mov dword [0x2000+(LIN(xor_callee)>>12)*4], LIN(xor_callee) | 7
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
    mov word [0x3000+13*8], LIN(gp_handler) & 0xffff
    mov word [0x3000+13*8+2], 8
    mov dword [0x3000+13*8+4], (LIN(gp_handler) & 0xffff0000) | 0x8e00
    mov word [0x3000+6*8], LIN(end_handler) & 0xffff
    mov word [0x3000+6*8+2], 8
    mov dword [0x3000+6*8+4], (LIN(end_handler) & 0xffff0000) | 0x8e00
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
    push dword LIN(user_loop) >> 4
    push dword 0
%elifdef LDT
    push dword 0x1f
    push dword 0x8000
    push dword 2
    push dword 0x0f
    push dword RM(user_loop)
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

; Reached through the UD2 after the last pass, from any user mode.
end_handler:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    WRITE_LINE end_text, END_ROW_ADDRESS
    HALT_FOREVER

end_text: db END_TEXT, 0

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
    ; LDT: user code at the load address and at the callee, user data at 40000h.
    dq 0
    dw 0xffff, LIN(start) & 0xffff
    db LIN(start) >> 16, 0xfa, 0, 0
    dw 0xffff, LIN(xor_callee) & 0xffff
    db LIN(xor_callee) >> 16, 0xfa, 0, 0
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
    call (LIN(xor_callee) >> 4):0
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
    cmp ebp, PASSES * BODIES
    jb user_loop
    ud2
near_callee:
%ifdef FLAT
    add eax, 1
%else
    add ax, 1
%endif
    ret
