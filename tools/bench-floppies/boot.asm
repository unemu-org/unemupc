; SPDX-License-Identifier: AGPL-3.0-or-later
; A complete 1.44 MB floppy image: boot sector, payload, empty FAT12 volume.
; Assemble with nasm -f bin -DPAYLOAD='<payload.bin>'.
;
; The BIOS loads sector 1 to 0000:7C00h and jumps to it with DL = boot drive
; (IBM Personal Computer AT Technical Reference, BIOS, INT 19h bootstrap).
; The boot sector reads the payload with INT 13h function 02h, one sector per
; call, so no call crosses a track or a 64 KiB DMA page; each failed read
; resets the controller (function 00h) and retries. It then jumps to the
; payload at 1000:0000h in real mode with DL unchanged. IBM Personal Computer
; AT Technical Reference, BIOS listing, Diskette I/O (INT 13h): function 00h
; (reset the diskette system) and function 02h (read sectors, CH cylinder,
; CL sector, DH head, DL drive, ES:BX buffer, CF set on error).
;
; The geometry is the medium's own (18 sectors, 2 heads), not asked of the
; BIOS: INT 13h function 08h reports the drive's largest format, which for a
; 2.88 MB drive holding this 1.44 MB disk would be 36 sectors per track.
;
; The payload sits in the volume's reserved sectors, after the boot sector,
; so the disk is also a valid empty FAT12 floppy (Microsoft FAT32 File System
; Specification, BPB and FAT12 layout): 80 cylinders, 2 heads, 18 sectors.
bits 16
org 0x7c00

%define LOAD_SEGMENT 0x1000
%define PAYLOAD_SECTORS 32
%define SECTORS_PER_TRACK 18
%define HEADS 2
%define ATTEMPTS 5

boot_sector:
    jmp short boot
    nop
    db "UNEMUBNC"
    dw 512                      ; bytes per sector
    db 1                        ; sectors per cluster
    dw 1 + PAYLOAD_SECTORS      ; reserved sectors: boot sector and payload
    db 2                        ; FATs
    dw 224                      ; root directory entries
    dw 2880                     ; total sectors
    db 0xf0                     ; media descriptor: 3.5-inch 1.44 MB
    dw 9                        ; sectors per FAT
    dw SECTORS_PER_TRACK
    dw HEADS
    dd 0                        ; hidden sectors
    dd 0                        ; total sectors (32-bit field)
    db 0                        ; drive number
    db 0
    db 0x29                     ; extended boot signature
    dd 0x0bec4c40               ; fixed serial number, for reproducible images
    db "UNEMU BENCH"
    db "FAT12   "

boot:
    cli
    xor ax, ax
    mov ds, ax
    mov ss, ax
    mov sp, 0x7c00
    ; Some BIOSes enter at 07C0:0000h; use one form of the address.
    jmp 0:normalised
normalised:
    sti
    cld
    mov [drive], dl
    mov ax, LOAD_SEGMENT
    mov es, ax

read_sector:
    mov byte [attempts], ATTEMPTS
retry:
    ; Track = LBA / 18, sector = LBA % 18 + 1, head = track & 1,
    ; cylinder = track / 2 (at most 79, so CL bits 6-7 stay clear).
    mov ax, [lba]
    xor dx, dx
    mov cx, SECTORS_PER_TRACK
    div cx
    mov cl, dl
    inc cl
    mov dh, al
    and dh, HEADS - 1
    shr ax, 1
    mov ch, al
    mov dl, [drive]
    xor bx, bx
    mov ax, 0x0201
    int 0x13
    jnc sector_read
    xor ax, ax
    mov dl, [drive]
    int 0x13
    dec byte [attempts]
    jnz retry
    mov si, disk_error
print:
    lodsb
    test al, al
    jz stop
    push si
    mov ah, 0x0e
    mov bx, 0x0007
    int 0x10
    pop si
    jmp print
stop:
    cli
    hlt
    jmp stop

sector_read:
    mov ax, es
    add ax, 512 / 16
    mov es, ax
    inc word [lba]
    cmp word [lba], PAYLOAD_SECTORS
    jbe read_sector
    mov dl, [drive]
    jmp LOAD_SEGMENT:0

lba: dw 1
drive: db 0
attempts: db 0
disk_error: db "UNEMU-BENCH disk read error", 0

    times 510-($-$$) db 0
    dw 0xaa55

payload:
    incbin PAYLOAD
    times PAYLOAD_SECTORS*512-($-payload) db 0

; Two copies of an empty FAT: media byte, then the end-of-chain marker that
; FAT12 reserves for cluster 1.
first_fat:
    db 0xf0, 0xff, 0xff
    times 9*512-($-first_fat) db 0
second_fat:
    db 0xf0, 0xff, 0xff
    times 9*512-($-second_fat) db 0

    times 2880*512-($-$$) db 0
