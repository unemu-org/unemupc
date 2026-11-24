; SPDX-License-Identifier: AGPL-3.0-or-later
; IBM OS/2 Linear Executable Module Format, Linear Executable Header,
; Object Table and Object Page Table. DOS/32A LOADER.ASM loads LE objects
; at arbitrary addresses, so call/pop addresses avoid relocation records.
; DPMI 0.9, functions 0501h/0502h; Intel 80386 PRM, §10.1 (CR0).
bits 32
org 0

db 'LE', 0, 0
dd 0                           ; format level
dw 2, 1                        ; 386, OS/2
dd 0, 0, 1                     ; version, flags, module pages
dd 1, 0                        ; entry object, EIP
dd 2, 4096                     ; stack object, ESP
dd 4096, 4096                  ; page size, bytes in last page
dd 8, 0                        ; fixup section size, checksum
dd data_pages - objects, 0      ; loader section size, checksum
dd objects - $$, 2, page_table - $$       ; object table, count, page map
times 0x68 - ($-$$) db 0
dd fixup_pages - $$, fixup_records - $$
times 0x80 - ($-$$) db 0
dd data_pages - $$             ; absolute file offset for unbound LE
times 0xAC - ($-$$) db 0
objects:
dd 4096, 0, 0x2007, 1, 1, 0   ; readable/writable/executable, use32
dd 4096, 0, 0x2003, 2, 0, 0   ; zero-filled data/stack, no file pages
page_table:
db 0, 0, 1, 0                  ; big-endian 24-bit page index, valid
fixup_pages:
dd 0, 0                        ; empty relocation interval
fixup_records:
align 16, db 0
data_pages:
    call .base
.base:
    pop ebp
    sub ebp, .base - data_pages
    push ss
    pop ds
    push ss
    pop es
    lea edx, [ebp + hello - data_pages]
    mov ah, 9
    int 0x21

    ; DOS/32A runs its client at ring zero; record the live paging state.
    mov eax, cr0
    lea edi, [ebp + cr0_hex - data_pages]
    call hex32
    lea edx, [ebp + cr0_line - data_pages]
    mov ah, 9
    int 0x21

    mov ax, 0x0501
    mov ebx, 0x10               ; BX:CX = 1 MiB
    xor ecx, ecx
    int 0x31
    jc failed
    push esi
    push edi
    shl ebx, 16
    mov bx, cx
    cmp ebx, 0x100000
    jb failed
    mov edi, ebx
    xor ecx, ecx
.fill:
    mov [edi + ecx*4], ecx
    inc ecx
    cmp ecx, 0x40000
    jne .fill
    xor eax, eax
    xor ecx, ecx
.sum:
    add eax, [edi + ecx*4]
    inc ecx
    cmp ecx, 0x40000
    jne .sum
    cmp eax, 0xFFFE0000
    jne failed
    lea edi, [ebp + sum_hex - data_pages]
    call hex32
    pop edi
    pop esi
    mov ax, 0x0502
    int 0x31
    jc failed
    lea edx, [ebp + sum_line - data_pages]
    mov ah, 9
    int 0x21
    mov ax, 0x4C00
    int 0x21
failed:
    lea edx, [ebp + failure - data_pages]
    mov ah, 9
    int 0x21
    mov ax, 0x4C01
    int 0x21

hex32:
    mov ecx, 8
.next:
    rol eax, 4
    mov edx, eax
    and edx, 15
    mov dl, [ebp + digits - data_pages + edx]
    mov [edi], dl
    inc edi
    loop .next
    ret
hello: db 'DOS32-PM32-BEGIN', 13, 10, '$'
cr0_line: db 'DOS32-CR0='
cr0_hex: times 8 db '0'
    db 13, 10, '$'
sum_line: db 'DOS32-1M-SUM='
sum_hex: times 8 db '0'
    db 13, 10, '$'
failure: db 'DOS32-FAIL', 13, 10, '$'
digits: db '0123456789ABCDEF'
times 4096 - ($-data_pages) db 0
