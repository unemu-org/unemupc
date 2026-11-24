# DOS extender test client

`dpmitest.asm` is project-owned AGPL-3.0-or-later source. Run
`bash vendor/dos-extender-test/build.sh` inside the devShell (NASM 3.01).
The deterministic output is **4,336 bytes**, SHA-256
`91a5b15f1a47e7d5651663e36b66e56190d79b7fd553d775e60b9dd60113a105`.
No linker, C runtime, proprietary compiler or downloaded build input is used.
The generated `dpmitest.exe` is ignored; the fixtures lane builds it.

## Executable format

The binary is an unbound **LE** executable, passed explicitly to
`DOS32A DPMITEST.EXE`; it is not a directly executable DOS MZ program.
The **IBM OS/2 Linear Executable Module Format** sections **Linear Executable
Header**, **Object Table** and **Fixup Page Table** describe the shared
structures. The LE-specific page map follows the pinned DOS/32A loader source
in `SOURCE/DOS32A/SOURCES.ZIP`: `DOS32A/LOADER.ASM:235–245`
(`relocate_le_object`) indexes and reads four-byte entries; lines **246–248**
read the page index's low two bytes in big-endian order. The client's single
page uses index 1 (`00 00 01 00`, including the valid-page flag).
The LX specification's eight-byte page-map layout does not apply here.
The remaining implementation is checked against the same source archive:
`DOS32A/TEXT/CLIENT/MISC.ASM`, `load_extrn_exec_header` / `search_for_le`,
and `DOS32A/LOADER.ASM`, `load_header`, `load_object`, `create_selector` and
`relocate_le_object`.

The header starts at offset zero. Its object table (relative offset 40h),
page map (48h), fixup page/record tables (68h/6Ch), and absolute data offset
(80h) address NASM labels. Object 1 is one 4 KiB readable/writable/executable
32-bit page. Object 2 is a zero-filled 4 KiB data/stack object with no file
pages. EIP is object 1 offset zero; ESP is object 2 offset 4096. The page
map uses LE's three-byte big-endian page index 1 plus a valid-page flag.
The two zero fixup-page offsets describe an empty relocation interval.
Call/pop discovers the loaded code address, so there are no absolute code
addresses or relocation records. DOS/32A supplies flat 32-bit selectors;
DS and ES are explicitly loaded from the data/stack selector.

## Runtime contract

The client prints `DOS32-PM32-BEGIN` using the extender's INT 21h AH=09h
translation, reads the live CR0 with MOV EAX,CR0 and prints `DOS32-CR0=`
followed by eight hexadecimal digits. DOS/32A's own host runs clients at
ring 0; the probe does not target an arbitrary ring-3 DPMI host. CR0.PE
must be set; CR0.PG must be set in the JEMMEX/VCPI run.

DPMI 0.9 function **0501h Allocate Memory Block** allocates 1 MiB using
BX:CX=0010:0000. The returned BX:CX linear address must be at least 1 MiB.
The client fills 262,144 dwords with their indexes, then reads them back
and sums them modulo 2^32: **FFFE0000**. Function **0502h Free Memory Block**
releases SI:DI's handle, and `DOS32-1M-SUM=FFFE0000` is printed before
INT 21h AX=4C00h returns to DOS. Allocation, address, checksum or release
failure prints `DOS32-FAIL` and exits with status 1.

The golden AUTOEXEC checks ERRORLEVEL, prints `DOS32-RETURNED`, executes
VER, prints `DOS32-VER-DONE`, and returns to the prompt. The test then types `echo dos32-ok` through
the keyboard and requires its output and a new prompt. The raw variant
has no JEMMEX/XMS manager; DOS/32A selects its own host. The VCPI variant
first runs `JEMMEX LOAD NOEMS NORAM NOVMW`. Runtime qualification and any
CPU boundary are recorded in
[protected-mode gaps](../../backlog/docs/design/protected-mode-gaps.md#dos-extender-qualification).

**MEASURED, 2026-11-24**, CPU pin
`fc07c81f580689c31f182f4421fb9cc5f07374d1`: the unchanged client completes
on 80386DX and both 80486DX variants, raw and with JemmEx/VCPI. All six
print the expected checksum, return with status zero, execute VER, and
reach `A:\>`. CPU-side samples in both long loops observe PE=1 and VM=0;
VCPI samples also observe PG=1. CR0 is 00000001 / 80000001 on 386 and
00000011 / 80000011 on 486 (raw / VCPI), matching the printed line.
The golden identifies the fill and sum loops at code offsets 52h–5Dh and
62h–6Dh relative to EBP, the call/pop code base; changing this pinned
client requires reviewing those offsets too. DOS/32A's mouse warning 9004
is non-fatal. No client or extender binary patch is needed.
