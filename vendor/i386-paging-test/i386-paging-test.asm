; SPDX-License-Identifier: AGPL-3.0-or-later
;
; Two-level paging on a complete ISA board. Intel 80386 Programmer's
; Reference Manual §5.2 Page Translation, §6.4 Page-Level Protection,
; §9.8.14 Page Fault; Intel486 Family Programmer's Reference Manual
; §4.1.3 CR0.WP and §26 INVLPG. See backlog/docs/design/i386-pm-test.md.
; All tables are RAM. The identity map deliberately grants user access to
; this test's code/data so page permissions can be tested independently.
cpu 486
bits 16
org 0xE0000
%define RM(x) (x - $$ - 0x10000)
PD equ 0x10000
PT equ 0x11000
PD2 equ 0x12000
ALIAS_PT equ 0x13000
IDT equ 0x1000
TSS equ 0x6000
FAILURES equ 0x5000
FAULTS equ 0x5004
EXPECT_ADDR equ 0x5008
EXPECT_ERROR equ 0x500C
EXPECT_IP equ 0x5010
RESUME equ 0x5014
MODEL equ 0x5018

%macro REPORT 1
    mov esi, %1
    call puts
    jz %%ok
    inc dword [FAILURES]
    mov esi, fail
    jmp %%print
%%ok:
    mov esi, ok
%%print:
    call puts
%endmacro
%macro EXPECT_PF 4
    mov dword [EXPECT_ADDR], %1
    mov dword [EXPECT_ERROR], %2
    mov dword [EXPECT_IP], %3
    mov dword [RESUME], %4
%endmacro

times 0x10000-($-$$) db 0xFF
start:
    cli
    cld
    xor ax, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x8000
    mov di, FAILURES
    mov cx, 16
    rep stosw
    mov [es:MODEL], ebp
    mov ax, cs
    mov ds, ax
    mov al, 2
    out 0x92, al
    mov al, 0xFF
    out 0x21, al
    out 0xA1, al
    lgdt [RM(gdtr)]
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
    mov esp, 0x90000
    mov esi, title
    call puts
    mov edi, PD
    mov ecx, 4096
    xor eax, eax
    rep stosd
    mov edi, PT
    mov ecx, 1024
    mov eax, 7
.identity:
    stosd
    add eax, 0x1000
    loop .identity
    mov dword [PD], PT|7
    mov dword [PD+4], ALIAS_PT|7
    mov dword [PD+8], ALIAS_PT|3
    mov dword [PD+12], ALIAS_PT|5
    mov dword [PD2], PT|7
    mov dword [PD2+4], ALIAS_PT|7
    mov dword [PD2+8], ALIAS_PT|3
    mov dword [PD2+12], ALIAS_PT|5
    mov dword [ALIAS_PT], 0x20007
    mov dword [ALIAS_PT+8], 0x22003
    mov dword [ALIAS_PT+12], 0x23005
    mov dword [ALIAS_PT+16], 0x24007
    mov dword [0x20000], 0x11223344
    mov dword [0x21000], 0x55667788
    mov edi, IDT
    mov ecx, 256
.idt:
    mov eax, unexpected
    mov word [edi], ax
    mov word [edi+2], 8
    mov word [edi+4], 0x8E00
    shr eax, 16
    mov word [edi+6], ax
    add edi, 8
    loop .idt
%macro GATE 3
    mov eax, %2
    mov word [IDT+%1*8], ax
    shr eax, 16
    mov word [IDT+%1*8+6], ax
    mov byte [IDT+%1*8+5], %3
%endmacro
    GATE 14, pf_handler, 0x8E
    GATE 0x40, ring_return, 0xEE
    lidt [idtr]
    mov eax, PD
    mov cr3, eax
    mov eax, cr0
    or eax, 0x80000000
    mov cr0, eax
    mov eax, [0x400000]
    cmp eax, 0x11223344
    REPORT identity_line
    mov eax, [PD+4]
    and eax, 0x20
    xor eax, 0x20
    mov ebx, [ALIAS_PT]
    and ebx, 0x60
    xor ebx, 0x20
    or eax, ebx
    REPORT accessed_line
    mov dword [0x400000], 0xAABBCCDD
    mov eax, [ALIAS_PT]
    and eax, 0x60
    xor eax, 0x60
    mov ebx, [0x20000]
    xor ebx, 0xAABBCCDD
    or eax, ebx
    REPORT dirty_line
    EXPECT_PF 0x401000, 0, absent_fault, absent_resume
absent_fault:
    mov eax, [0x401000]
absent_resume:
    cmp dword [FAULTS], 1
    REPORT absent_line

    mov dword [ALIAS_PT], 0x21007
    mov eax, cr3
    mov cr3, eax
    cmp dword [0x400000], 0x55667788
    REPORT reload_line
    mov dword [PD+4], 0
    mov eax, PD2
    mov cr3, eax
    cmp dword [0x400000], 0x55667788
    REPORT directory_line
    cmp byte [MODEL+1], 4
    jne .no_invlpg
    mov dword [ALIAS_PT], 0x20007
    invlpg [0x400000]
    cmp dword [0x400000], 0xAABBCCDD
    REPORT invlpg_line
    jmp .user
.no_invlpg:
    mov esi, invlpg_skip
    call puts
.user:
    mov edi, TSS
    xor eax, eax
    mov ecx, 26
    rep stosd
    mov dword [TSS+4], 0x88000
    mov word [TSS+8], 0x10
    mov word [TSS+102], 104
    mov ax, 0x28
    ltr ax
    EXPECT_PF 0x402000, 5, user_super_fault, user_super_resume
    push dword 0x23
    push dword 0x86000
    push dword 2
    push dword 0x1B
    push dword user
    iretd
user:
    mov ax, 0x23
    mov ds, ax
    mov es, ax
user_super_fault:
    mov eax, [0x402000]
user_super_resume:
    EXPECT_PF 0x403000, 7, user_ro_fault, user_ro_resume
user_ro_fault:
    mov dword [0x403000], 0xBAD
user_ro_resume:
    EXPECT_PF 0x800000, 5, user_pde_super_fault, user_pde_super_resume
user_pde_super_fault:
    mov eax, [0x800000]
user_pde_super_resume:
    EXPECT_PF 0xC00000, 7, user_pde_ro_fault, user_pde_ro_resume
user_pde_ro_fault:
    mov dword [0xC00000], 0xBAD
user_pde_ro_resume:
    mov dword [0x404000], 0x13579BDF
    int 0x40
    jmp $
ring_return:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov esp, 0x90000
    cmp dword [FAULTS], 5
    REPORT user_line
    cmp dword [0x24000], 0x13579BDF
    REPORT user_rw_line
    ; WP=0 allows supervisor writes even to a read-only user mapping.
    mov dword [0x403000], 0x2468ACE0
    cmp dword [0x23000], 0x2468ACE0
    REPORT supervisor_line
    cmp byte [MODEL+1], 4
    jne no_wp
    mov eax, cr0
    or eax, 0x10000
    mov cr0, eax
    EXPECT_PF 0x403000, 3, wp_fault, wp_resume
wp_fault:
    mov dword [0x403000], 0xBAD
wp_resume:
    ; Also protect a supervisor-only read-only mapping.
    mov dword [ALIAS_PT+8], 0x22001
    invlpg [0x402000]
    EXPECT_PF 0x402000, 3, wp_super_fault, wp_super_resume
wp_super_fault:
    mov dword [0x402000], 0xBAD
wp_super_resume:
    cmp dword [FAULTS], 7
    REPORT wp_line
    jmp finish
no_wp:
    mov esi, wp_skip
    call puts
finish:
    mov esi, failures_line
    call puts
    mov eax, [FAILURES]
    call hex32
    mov esi, newline
    call puts
    cli
    hlt
    jmp $
pf_handler:
    pushad
    mov eax, cr2
    cmp eax, [EXPECT_ADDR]
    jne .bad
    mov eax, [esp+32]
    cmp eax, [EXPECT_ERROR]
    jne .bad
    mov eax, [esp+36]
    cmp eax, [EXPECT_IP]
    je .record
.bad:
    inc dword [FAILURES]
.record:
    inc dword [FAULTS]
    mov esi, pf_line
    call puts
    mov eax, cr2
    call hex32
    mov esi, error_line
    call puts
    mov eax, [esp+32]
    call hex32
    mov esi, newline
    call puts
    mov eax, [RESUME]
    mov [esp+36], eax
    popad
    add esp, 4
    iretd
unexpected:
    mov esi, unexpected_line
    call puts
    cli
    hlt
    jmp $
puts:
    pushfd
    pushad
.loop:
    lodsb
    test al, al
    jz .done
    mov dx, 0x402
    out dx, al
    jmp .loop
.done:
    popad
    popfd
    ret
hex32:
    pushad
    mov ebx, eax
    mov ecx, 8
.loop:
    rol ebx, 4
    mov al, bl
    and al, 15
    add al, '0'
    cmp al, '9'
    jbe .digit
    add al, 7
.digit:
    mov dx, 0x402
    out dx, al
    loop .loop
    popad
    ret
align 8
gdt:
    dq 0
    dq 0x00CF9A000000FFFF
    dq 0x00CF92000000FFFF
    dq 0x00CFFA000000FFFF
    dq 0x00CFF2000000FFFF
    dw 103, TSS
    db 0, 0x89, 0, 0
gdt_end:
gdtr: dw gdt_end-gdt-1
    dd gdt
idtr: dw 256*8-1
    dd IDT
title: db 'i386-paging-test', 10, 0
identity_line: db 'identity alias ', 0
accessed_line: db 'pde pte accessed ', 0
dirty_line: db 'alias write dirty ', 0
absent_line: db 'not present resume ', 0
reload_line: db 'cr3 equal reload ', 0
directory_line: db 'cr3 new directory ', 0
invlpg_line: db 'invlpg pte change ', 0
invlpg_skip: db 'invlpg unavailable on 386', 10, 0
user_line: db 'ring3 pte pde us rw ', 0
user_rw_line: db 'ring3 writable ', 0
supervisor_line: db 'supervisor wp=0 write ', 0
wp_line: db 'supervisor wp=1 user supervisor ro ', 0
wp_skip: db 'wp unavailable on 386', 10, 0
failures_line: db 'failures ', 0
pf_line: db '#pf cr2=', 0
error_line: db ' error=', 0
newline: db 10, 0
ok: db 'ok', 10, 0
fail: db 'FAIL', 10, 0
unexpected_line: db 'unexpected exception', 10, 0

bits 16
times 0x1F000-($-$$) db 0xFF
high_start:
    mov ebp, edx
    jmp 0xF000:RM(start)
times 0x1FFF0-($-$$) db 0xFF
    jmp near high_start
    times 0x20000-($-$$) db 0xFF
