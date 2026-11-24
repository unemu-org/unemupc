; SPDX-License-Identifier: AGPL-3.0-or-later
;
; Intel486 DX Programmer's Reference Manual, §3 Basic Architecture,
; §4 System Architecture, §10 Initialization and Chapter 26 instruction
; definitions; Embedded Intel486 Processor Family Developer's Manual,
; Appendix D CPUID. See backlog/docs/design/i386-pm-test.md §486 extension.
; Shadow RAM holds the GDT because loading descriptors sets accessed bits.
; NASM groups the documented CPUID and CMPXCHG encodings under Pentium.
; This source uses only the Intel486 instructions listed above.
cpu pentium
bits 16
org 0xE0000
%define RM(x) (x - $$ - 0x10000)
FAILURES equ 0x5000
HAS_CPUID equ 0x5004
SCRATCH equ 0x5010

%macro REPORT 1
    mov esi, %1
    call puts32
    jz %%ok
    inc word [FAILURES]
    mov esi, fail
    jmp %%print
%%ok:
    mov esi, ok
%%print:
    call puts32
%endmacro

times 0x10000-($-$$) db 0xFF
real_start:
    cli
    cld
    mov ax, cs
    mov ds, ax
    xor ax, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x8000
    mov word [es:FAILURES], 0
    mov dword [es:HAS_CPUID], 0
    ; EFLAGS.ID is the architectural CPUID presence test, even on early DX.
    pushfd
    pop eax
    mov ebx, eax
    xor eax, 0x200000
    push eax
    popfd
    pushfd
    pop eax
    push ebx
    popfd
    xor eax, ebx
    and eax, 0x200000
    jz .identified
    mov dword [es:HAS_CPUID], 1
.identified:
    mov al, 2
    out 0x92, al
    lgdt [RM(gdtr)]
    mov eax, cr0
    mov edi, eax
    or eax, 1
    mov cr0, eax
    jmp dword 0x08:protected_start

bits 32
protected_start:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax
    mov esp, 0x90000
    mov esi, title
    call puts32
    mov eax, 0x0403
    cmp dword [HAS_CPUID], 0
    je .signature
    mov eax, 0x0410
.signature:
    cmp ebp, eax
    REPORT reset_line
    cmp edi, 0x60000010
    REPORT cr0_line

    mov eax, 0x12345678
    bswap eax
    cmp eax, 0x78563412
    REPORT bswap_line

    mov eax, 7
    mov ebx, 9
    xadd eax, ebx
    xor eax, 16
    xor ebx, 7
    or eax, ebx
    mov dword [SCRATCH], 11
    mov ecx, 4
    xadd [SCRATCH], ecx
    xor ecx, 11
    or eax, ecx
    mov ecx, [SCRATCH]
    xor ecx, 15
    or eax, ecx
    REPORT xadd_line

    mov eax, 15
    mov ebx, 42
    cmpxchg [SCRATCH], ebx
    setnz dl
    mov ecx, [SCRATCH]
    xor ecx, 42
    movzx edx, dl
    or ecx, edx
    mov eax, 99
    cmpxchg [SCRATCH], ebx
    setz dl
    xor eax, 42
    or ecx, eax
    movzx edx, dl
    or ecx, edx
    mov edx, [SCRATCH]
    xor edx, 42
    or ecx, edx
    mov eax, 5
    mov edx, 5
    cmpxchg edx, ebx
    xor edx, 42
    or ecx, edx
    test ecx, ecx
    REPORT cmpxchg_line

    ; WP protection requires paging; this checks only writable control bits.
    mov ebx, cr0
    mov eax, ebx
    or eax, 0x50000
    mov cr0, eax
    mov eax, cr0
    xor eax, ebx
    xor eax, 0x50000
    REPORT control_line
    mov cr0, ebx
    pushfd
    pop ebx
    mov eax, ebx
    xor eax, 0x40000
    push eax
    popfd
    pushfd
    pop eax
    push ebx
    popfd
    xor eax, ebx
    cmp eax, 0x40000
    REPORT ac_line

    cmp dword [HAS_CPUID], 0
    jne .cpuid
    mov esi, absent_line
    call puts32
    jmp .cache
.cpuid:
    mov esi, present_line
    call puts32
    xor eax, eax
    cpuid
    xor eax, 1
    xor ebx, 0x756E6547
    xor edx, 0x49656E69
    xor ecx, 0x6C65746E
    or eax, ebx
    or eax, edx
    or eax, ecx
    REPORT leaf0_line
    mov eax, 1
    cpuid
    xor eax, 0x0410
    xor edx, 1
    or eax, ebx
    or eax, ecx
    or eax, edx
    REPORT leaf1_line
.cache:
    invd
    wbinvd
    invlpg [SCRATCH]
    xor eax, eax
    REPORT cache_line
    mov esi, failures_line
    call puts32
    mov bx, [FAILURES]
    mov ecx, 4
.hex:
    rol bx, 4
    mov al, bl
    and al, 15
    add al, '0'
    cmp al, '9'
    jbe .digit
    add al, 7
.digit:
    mov dx, 0x402
    out dx, al
    loop .hex
    mov esi, newline
    call puts32
    cli
    hlt
    jmp $

puts32:
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

align 8
gdt:
    dq 0
    dq 0x00CF9A000000FFFF
    dq 0x00CF92000000FFFF
gdt_end:
gdtr: dw gdt_end-gdt-1
    dd gdt

title: db 'i486-pm-test', 10, 0
reset_line: db 'reset signature ', 0
cr0_line: db 'reset cr0=60000010 ', 0
bswap_line: db 'bswap32 ', 0
xadd_line: db 'xadd register memory ', 0
cmpxchg_line: db 'cmpxchg success failure ', 0
control_line: db 'cr0 wp am readback ', 0
ac_line: db 'eflags ac readback ', 0
absent_line: db 'eflags id fixed; cpuid absent', 10, 0
present_line: db 'eflags id toggle; cpuid present', 10, 0
leaf0_line: db 'cpuid 0 GenuineIntel max=1 ', 0
leaf1_line: db 'cpuid 1 eax=0410 edx=0001 ', 0
cache_line: db 'invd wbinvd invlpg ', 0
failures_line: db 'failures ', 0
ok: db 'ok', 10, 0
fail: db 'FAIL', 10, 0
newline: db 10, 0

bits 16
times 0x1F000-($-$$) db 0xFF
high_start:
    mov ebp, edx
    jmp 0xF000:RM(real_start)
times 0x1FFF0-($-$$) db 0xFF
    jmp near high_start
    times 0x20000-($-$$) db 0xFF
