; SPDX-License-Identifier: AGPL-3.0-or-later
;
; BIOS-independent 80386 integration ROM. Intel 80386 Programmer's Reference
; Manual §10 Initialization, §6 Protection, §7 Multitasking, §9 Interrupts
; and Exceptions and §14.5 Switching to Real-Address Mode; Intel 8259A
; datasheet, Initialization Command Words and End of Interrupt; Intel 8254
; datasheet, Mode 2. See backlog/docs/design/i386-pm-test.md.
;
; Tables live in writable shadow RAM because the processor sets descriptor
; accessed and TSS busy bits. No BIOS services or CPU identification opcode
; are needed. Polling loops bound missing-IRQ failures in retired instructions.
cpu 386
bits 16
org 0xE0000

%define RM(x) (x - $$ - 0x10000)
FAILURES equ 0x5000
GP_COUNT equ 0x5004
INT_COUNT equ 0x5008
TRAP_COUNT equ 0x500C
IRQ_COUNT equ 0x5010
RING_INT equ 0x5014
RING_IRQ equ 0x5018
FRAME_BAD equ 0x501C
TSS equ 0x6000
IDT equ 0x1000
KSTACK equ 0x90000
ISTACK equ 0x88000
USTACK equ 0x86000
TICKS equ 3

%macro REPORT16 1
    mov si, RM(%1)
    call puts16
    jz %%ok
    inc word [es:FAILURES]
    mov si, RM(fail)
    jmp %%print
%%ok:
    mov si, RM(ok)
%%print:
    call puts16
%endmacro

%macro REPORT32 1
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
    mov di, FAILURES
    mov cx, 16
    rep stosw
    mov si, RM(title)
    call puts16
    mov edx, ebp
    cmp dh, 3
    REPORT16 reset_line

    ; A 286 cannot set IOPL/NT in real mode; a 386 can, with bit 15 clear.
    push word 0
    popf
    pushf
    pop bx
    push word 0x7000
    popf
    pushf
    pop cx
    push word 2
    popf
    and bx, 0xF000
    and cx, 0xF000
    xor cx, 0x7000
    or bx, cx
    REPORT16 flags_line

    ; Disable both OR-wired sources before proving that port 92h opens A20.
    mov al, 0xD1
    out 0x64, al
    mov al, 0xDD
    out 0x60, al
    xor al, al
    out 0x92, al
    mov ax, 0xFFFF
    mov fs, ax
    mov word [es:0x7000], 0x1111
    mov word [fs:0x7010], 0x2222
    cmp word [es:0x7000], 0x2222
    REPORT16 a20_off_line
    mov al, 2
    out 0x92, al
    in al, 0x92
    and al, 2
    xor al, 2
    mov bl, al
    mov word [es:0x7000], 0x1111
    mov word [fs:0x7010], 0x2222
    mov ax, [es:0x7000]
    xor ax, 0x1111
    or bl, al
    or bl, ah
    mov ax, [fs:0x7010]
    xor ax, 0x2222
    or al, ah
    or al, bl
    REPORT16 a20_on_line

    mov al, 0xFF
    out 0x21, al
    out 0xA1, al
    lgdt [RM(gdtr)]
    mov eax, cr0
    or eax, 1
    mov cr0, eax
    jmp dword 0x08:protected_start

puts16:
    pushf
    pusha
.loop:
    lodsb
    test al, al
    jz .done
    mov dx, 0x402
    out dx, al
    jmp .loop
.done:
    popa
    popf
    ret

bits 32
protected_start:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax
    mov esp, KSTACK
    xor eax, eax
    mov ax, cs
    xor eax, 0x08
    push dword 0x1234ABCD
    mov ebx, esp
    xor ebx, KSTACK-4
    or eax, ebx
    mov ebx, [KSTACK-4]
    xor ebx, 0x1234ABCD
    or eax, ebx
    pop ebx
    REPORT32 protected_line

    mov word [0x20000], 0x1357
    mov dword [0x120000], 0x89ABCDEF
    mov eax, [0x120000]
    xor eax, 0x89ABCDEF
    movzx ebx, word [0x20000]
    xor ebx, 0x1357
    or eax, ebx
    REPORT32 memory_line

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
    GATE 13, gp_handler, 0x8E
    GATE 0x20, irq_handler, 0x8E
    GATE 0x30, int_handler, 0x8E
    GATE 0x31, trap_handler, 0x8F
    GATE 0x40, ring_handler, 0xEE
    lidt [idtr]
    mov ax, 0x28
    mov fs, ax
limit_fault:
    mov eax, [fs:0x100]
limit_resume:
    cmp dword [GP_COUNT], 1
    REPORT32 limit_line
    mov ax, 0x10
    mov fs, ax

    sti
    int 0x30
    int 0x31
    cli
    mov eax, [INT_COUNT]
    xor eax, 1
    mov ebx, [TRAP_COUNT]
    xor ebx, 1
    or eax, ebx
    or eax, [FRAME_BAD]
    REPORT32 gates_line

    ; Both PICs get complete ICW sequences; only IRQ0 is unmasked.
    mov al, 0x11
    out 0x20, al
    out 0xA0, al
    mov al, 0x20
    out 0x21, al
    mov al, 0x28
    out 0xA1, al
    mov al, 4
    out 0x21, al
    mov al, 2
    out 0xA1, al
    mov al, 1
    out 0x21, al
    out 0xA1, al
    mov al, 0xFE
    out 0x21, al
    mov al, 0xFF
    out 0xA1, al
    mov al, 0x34
    out 0x43, al
    mov ax, 1000
    out 0x40, al
    mov al, ah
    out 0x40, al
    mov ecx, 100000
    sti
.wait_irq:
    cmp dword [IRQ_COUNT], TICKS
    jae .irq_done
    loop .wait_irq
.irq_done:
    cli
    cmp dword [IRQ_COUNT], TICKS
    REPORT32 timer_line

    mov edi, TSS
    xor eax, eax
    mov ecx, 26
    rep stosd
    mov dword [TSS+4], ISTACK
    mov word [TSS+8], 0x10
    mov word [TSS+102], 104
    mov ax, 0x40
    ltr ax
    push dword 0x3B
    push dword USTACK
    push dword 2
    push dword 0x33
    push dword ring3_start
    iretd

ring3_start:
    mov ax, 0x3B
    mov ds, ax
    mov es, ax
    mov ax, cs
    cmp ax, 0x33
    jne .bad
    cmp esp, USTACK
    je .int
.bad:
    inc dword [FRAME_BAD]
.int:
    int 0x40
ring3_after_int:
    ; The first INT returns with IF set and IOPL zero. The IRQ must use SS0.
    mov ecx, 100000
.wait:
    cmp dword [RING_IRQ], 1
    jae .done
    loop .wait
.done:
    int 0x40
    jmp $

ring0_return:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov esp, KSTACK
    cmp dword [RING_INT], 2
    REPORT32 ring_int_line
    mov eax, [RING_IRQ]
    xor eax, 1
    or eax, [FRAME_BAD]
    REPORT32 ring_irq_line
    mov al, 0xFF
    out 0x21, al
    out 0xA1, al
    jmp 0x18:RM(protected16)

; Interrupt entry preserves all general registers. Frame offsets below refer
; to the hardware frame before PUSHAD, so stack-size errors cannot pass.
gp_handler:
    cmp dword [esp], 0
    jne unexpected
    cmp dword [esp+4], limit_fault
    jne unexpected
    cmp dword [esp+8], 8
    jne unexpected
    inc dword [GP_COUNT]
    mov dword [esp+4], limit_resume
    add esp, 4
    iretd

int_handler:
    pushfd
    test dword [esp], 0x200
    jz .ok
    inc dword [FRAME_BAD]
.ok:
    add esp, 4
    inc dword [INT_COUNT]
    iretd

trap_handler:
    pushfd
    test dword [esp], 0x200
    jnz .ok
    inc dword [FRAME_BAD]
.ok:
    add esp, 4
    inc dword [TRAP_COUNT]
    iretd

irq_handler:
    pushad
    mov eax, [esp+36]
    cmp eax, 0x33
    je .user
    cmp eax, 8
    jne .bad
    inc dword [IRQ_COUNT]
    jmp .eoi
.user:
    mov ax, ss
    cmp ax, 0x10
    jne .bad
    cmp esp, ISTACK-20-32
    jne .bad
    cmp dword [esp+48], 0x3B
    jne .bad
    cmp dword [esp+44], USTACK
    jne .bad
    cmp dword [esp+32], ring3_after_int.wait
    jb .bad
    cmp dword [esp+32], ring3_after_int.done
    ja .bad
    inc dword [RING_IRQ]
    ; Prevent a second ring-3 tick before the final INT returns to ring 0.
    and dword [esp+40], ~0x200
    jmp .eoi
.bad:
    inc dword [FRAME_BAD]
.eoi:
    mov al, 0x20
    out 0x20, al
    popad
    iretd

ring_handler:
    cli
    mov ax, ss
    cmp ax, 0x10
    jne .bad
    cmp esp, ISTACK-20
    jne .bad
    cmp dword [esp+4], 0x33
    jne .bad
    cmp dword [esp+12], USTACK
    jne .bad
    cmp dword [esp+16], 0x3B
    jne .bad
    inc dword [RING_INT]
    cmp dword [RING_INT], 1
    jne .return
    cmp dword [esp], ring3_after_int
    jne .bad
    or dword [esp+8], 0x200
    iretd
.bad:
    inc dword [FRAME_BAD]
.return:
    ; The second INT deliberately finishes at CPL 0, abandoning its user frame.
    jmp ring0_return

unexpected:
    mov esi, unexpected_line
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

bits 16
protected16:
    mov ax, 0x20
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov esp, 0x8000
    mov eax, cr0
    and eax, ~1
    mov cr0, eax
    jmp 0xF000:RM(real_return)
real_return:
    mov ax, 0xF000
    mov ds, ax
    xor ax, ax
    mov es, ax
    mov fs, ax
    mov gs, ax
    mov ss, ax
    mov sp, 0x8000
    lidt [RM(real_idtr)]
    mov eax, cr0
    and eax, 1
    REPORT16 return_line
    mov si, RM(failures_line)
    call puts16
    mov bx, [es:FAILURES]
    mov cx, 4
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
    mov si, RM(newline)
    call puts16
    cli
    hlt
    jmp $

align 8
; Flat code/data, 16-bit exit code/data, limited data, user code/data, TSS.
gdt:
    dq 0
    dq 0x00CF9A000000FFFF
    dq 0x00CF92000000FFFF
    dq 0x00009A0F0000FFFF
    dq 0x000092000000FFFF
    dq 0x00009202000000FF
    dq 0x00CFFA000000FFFF
    dq 0x00CFF2000000FFFF
    dw 103, TSS
    db 0, 0x89, 0, 0
gdt_end:
gdtr: dw gdt_end-gdt-1
    dd gdt
idtr: dw 256*8-1
    dd IDT
real_idtr: dw 0x3FF
    dd 0

title: db 'i386-pm-test', 10, 0
reset_line: db 'reset dh=03 ', 0
flags_line: db 'flags 12-15 0000/7000 ', 0
a20_off_line: db 'a20 off alias ', 0
a20_on_line: db 'a20 port92 on ', 0
protected_line: db 'protected32 code stack ', 0
memory_line: db 'extended memory ', 0
limit_line: db 'limit gp(0) resume ', 0
gates_line: db 'interrupt trap gates ', 0
timer_line: db 'timer ring0 count=3 ', 0
ring_int_line: db 'ring3 int tss stack ', 0
ring_irq_line: db 'ring3 timer tss stack ', 0
return_line: db 'real mode return ', 0
failures_line: db 'failures ', 0
ok: db 'ok', 10, 0
fail: db 'FAIL', 10, 0
newline: db 10, 0
unexpected_line: db 'unexpected exception', 10, 0

; A near jump retains the reset-only hidden CS base until the far transfer.
times 0x1F000-($-$$) db 0xFF
high_start:
    mov ebp, edx
    jmp 0xF000:RM(real_start)
times 0x1FFF0-($-$$) db 0xFF
    jmp near high_start
    times 0x20000-($-$$) db 0xFF
