; SPDX-License-Identifier: AGPL-3.0-or-later
; VESA BIOS Extension (VBE) Core Functions Standard 3.0: functions 00h
; (controller information), 01h (mode information), 02h (set mode, with
; the linear-framebuffer bit 14), 03h (current mode), 05h (display window
; control) and 08h (DAC palette format), through SeaVGABIOS on a Bochs VBE
; card. IBM VGA/XGA Technical Reference, Video BIOS: INT 10h AH=0Eh teletype.
; Rebuild: nasm -f bin seabios-vbe.asm -o seabios-vbe.bin
;
; Word 0500h marks each stage; a bounded delay after each one lets the host
; capture the frame. Results are words from 0502h on (see the RES_ names);
; the controller block is at 1000h and the mode blocks at 1200h, 1300h and
; 1400h. Pixels reach the framebuffer through FS with a 4 GiB limit (big
; real mode: Intel486 Programmer's Reference Manual, §22.6 segment limits
; survive the return to real mode), with interrupts off while it is in use.
bits 16
org 0x7c00

MARKER          equ 0x500
RES_INFO        equ 0x502   ; 4F00h AX
RES_MODE8       equ 0x504   ; 4F01h AX for 101h
RES_MODE16      equ 0x506   ; 4F01h AX for 111h
RES_MODE32      equ 0x508   ; 4F01h AX for 142h
RES_SET8        equ 0x50a   ; 4F02h AX for 4101h
RES_DAC_SET     equ 0x50c   ; 4F08h BL=0 AX
RES_DAC_SET_BH  equ 0x50e   ; 4F08h BL=0 BH
RES_DAC_GET     equ 0x510   ; 4F08h BL=1 AX
RES_DAC_GET_BH  equ 0x512   ; 4F08h BL=1 BH
RES_SET16       equ 0x514   ; 4F02h AX for 4111h
RES_SET32       equ 0x516   ; 4F02h AX for 4142h
RES_SET_BANKED  equ 0x518   ; 4F02h AX for 0101h
RES_GET_BANK    equ 0x51a   ; 4F05h BH=1 AX
RES_GET_BANK_DX equ 0x51c   ; 4F05h BH=1 DX
RES_SET_TTY     equ 0x51e   ; 4F02h AX for 4111h before the teletype
RES_CURRENT     equ 0x520   ; 4F03h AX
RES_CURRENT_BX  equ 0x522   ; 4F03h BX
RES_SET_PLANAR  equ 0x524   ; 4F02h AX for 102h (4 bpp planar)
RES_DISPI_BPP   equ 0x526   ; DISPI BPP register after it
RES_SET_BANK    equ 0x530   ; 4F05h BH=0 AX, one word per bank 0..4
INFO            equ 0x1000
MODE8           equ 0x1200
MODE16          equ 0x1300
MODE32          equ 0x1400
PIXELS          equ 640 * 480
LFB             equ 0xe0000000

    jmp 0:boot
boot:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00
    cld
    sti
    ; The test continues past the boot sector.
    mov ax, 0x0202
    mov cx, 2
    xor dh, dh
    mov bx, 0x7e00
    int 0x13
    jc fail
    jmp start
fail:
    cli
    hlt
    jmp fail
    times 510-($-$$) db 0
    dw 0xaa55

start:
    ; Asking with "VBE2" returns the full 512-byte block with the mode list.
    mov dword [INFO], 'VBE2'
    mov di, INFO
    mov ax, 0x4f00
    int 0x10
    mov [RES_INFO], ax
    mov cx, 0x101
    mov di, MODE8
    call mode_info
    mov [RES_MODE8], ax
    mov cx, 0x111
    mov di, MODE16
    call mode_info
    mov [RES_MODE16], ax
    mov cx, 0x142
    mov di, MODE32
    call mode_info
    mov [RES_MODE32], ax

    ; Stage 1: 640x480 at 8 bpp through the framebuffer, eight-bit DAC.
    mov bx, 0x4101
    call set_mode
    mov [RES_SET8], ax
    mov ax, 0x4f08
    xor bl, bl
    mov bh, 8
    int 0x10
    mov [RES_DAC_SET], ax
    mov [RES_DAC_SET_BH], bh
    mov ax, 0x4f08
    mov bl, 1
    int 0x10
    mov [RES_DAC_GET], ax
    mov [RES_DAC_GET_BH], bh
    ; Values a six-bit DAC would truncate.
    mov dx, 0x3c8
    mov al, 1
    out dx, al
    inc dx
    mov al, 0x81
    out dx, al
    mov al, 0x42
    out dx, al
    mov al, 0x03
    out dx, al
    dec dx
    mov al, 0xff
    out dx, al
    inc dx
    mov al, 0xff
    out dx, al
    mov al, 0xfe
    out dx, al
    mov al, 0xfd
    out dx, al
    call unreal
    mov edi, LFB
    xor eax, eax
fill8:
    mov [fs:edi], al
    inc edi
    inc eax
    cmp eax, PIXELS
    jb fill8
    sti
    mov ax, 1
    call stage

    ; Stage 2: 16 bpp, pixel n holds n.
    mov bx, 0x4111
    call set_mode
    mov [RES_SET16], ax
    call unreal
    mov edi, LFB
    xor eax, eax
fill16:
    mov [fs:edi], ax
    add edi, 2
    inc eax
    cmp eax, PIXELS
    jb fill16
    sti
    mov ax, 2
    call stage

    ; Stage 3: 32 bpp, pixel n holds n.
    mov bx, 0x4142
    call set_mode
    mov [RES_SET32], ax
    call unreal
    mov edi, LFB
    xor eax, eax
fill32:
    mov [fs:edi], eax
    add edi, 4
    inc eax
    cmp eax, PIXELS
    jb fill32
    sti
    mov ax, 3
    call stage

    ; Stage 4: 8 bpp banked through the A0000h window. Pixel n of bank b
    ; holds (n & FFh) ^ (b * 25h), so a byte in the wrong bank shows.
    mov bx, 0x0101
    call set_mode
    mov [RES_SET_BANKED], ax
    xor si, si
bank:
    mov ax, 0x4f05
    xor bx, bx
    mov dx, si
    int 0x10
    mov bx, si
    shl bx, 1
    mov [RES_SET_BANK + bx], ax
    mov ax, si
    mov ah, 0x25
    mul ah
    mov bl, al
    mov ax, 0xa000
    mov es, ax
    xor di, di
    xor cx, cx                  ; 65536 bytes
    cmp si, 4
    jne bank_pixels
    mov cx, PIXELS - 4 * 0x10000
bank_pixels:
    mov ax, di
    xor al, bl
    stosb
    loop bank_pixels
    xor ax, ax
    mov es, ax
    inc si
    cmp si, 5
    jb bank
    mov ax, 0x4f05
    mov bx, 0x0100
    int 0x10
    mov [RES_GET_BANK], ax
    mov [RES_GET_BANK_DX], dx
    mov ax, 4
    call stage

    ; Stage 5: teletype in a direct-colour mode; SeaVGABIOS draws the glyphs
    ; through INT 15h AH=87h, since the framebuffer is above 1 MiB.
    mov bx, 0x4111
    call set_mode
    mov [RES_SET_TTY], ax
    mov si, message
teletype:
    lodsb
    test al, al
    jz teletype_done
    mov ah, 0x0e
    mov bx, 0x000f
    int 0x10
    jmp teletype
teletype_done:
    mov ax, 0x4f03
    int 0x10
    mov [RES_CURRENT], ax
    mov [RES_CURRENT_BX], bx
    mov ax, 5
    call stage

    ; Stage 6: the 4 bpp planar mode 102h, which the card does not support.
    mov bx, 0x0102
    call set_mode
    mov [RES_SET_PLANAR], ax
    mov dx, 0x1ce
    mov ax, 3
    out dx, ax
    inc dx
    in ax, dx
    mov [RES_DISPI_BPP], ax
    mov word [MARKER], 6
halt:
    cli
    hlt
    jmp halt

; CX = mode, ES:DI = block. Returns AX.
mode_info:
    mov ax, 0x4f01
    int 0x10
    ret

; BX = mode and flags. Returns AX.
set_mode:
    mov ax, 0x4f02
    int 0x10
    ret

; Mark stage AX, then give the host time to capture it.
stage:
    mov [MARKER], ax
    mov ecx, 300000
stage_wait:
    dec ecx
    jnz stage_wait
    ret

; Load FS with a flat 4 GiB data segment and return to real mode with
; interrupts off, so no handler runs while FS addresses the framebuffer.
unreal:
    cli
    lgdt [gdtr]
    mov eax, cr0
    or al, 1
    mov cr0, eax
    jmp short .protected
.protected:
    mov bx, 8
    mov fs, bx
    and al, 0xfe
    mov cr0, eax
    jmp short .real
.real:
    xor bx, bx
    mov fs, bx
    ret

align 8
gdt:
    dq 0
    dq 0x00cf92000000ffff       ; flat data, 4 GiB
gdtr:
    dw gdtr - gdt - 1
    dd gdt
message:
    db 'VBE', 0
    times 3*512-($-$$) db 0
