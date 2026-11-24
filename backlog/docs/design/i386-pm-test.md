# Open 80386 protected-mode integration ROM

The project-owned `vendor/i386-pm-test/i386-pm-test.asm` exercises the unpaged
386 machine independently of firmware boot. SeaBIOS remains blocked on an
instruction refused by the pinned CPU. This fixture gives a smaller oracle
for the working machine/CPU seams without changing the CPU or substituting
machine-side instruction emulation.

The offline build and licence are described in
[the fixture README](../../../vendor/i386-pm-test/README.md).
The golden is `crates/pc-harness/tests/i386_pm_test_golden.rs`.

## Hardware contract and layout

References: Intel 80386 Programmer's Reference Manual §6 Protection
(descriptors, limits and privilege), §7 Multitasking (386 TSS SS0:ESP0),
§9 Interrupts and Exceptions (32-bit interrupt/trap gates and IRET),
§10.1 Processor State After Reset and §14.5 Switching to Real-Address Mode;
Intel 8259A datasheet, Initialization Command Words and End of Interrupt;
Intel 8254 datasheet, Mode 2; IBM Personal Computer AT Technical Reference,
System Board, Keyboard Controller Output Port (D1h, A20).
Port 92h and the 402h debug console follow the generic board's emulator policy,
not a claim about an IBM 5170.

The harness builds a `MachineConfig` on `Generic386Isa` with I80386, no FPU,
640 KiB conventional RAM, 15 MiB extended RAM and the board's required devices
plus COM1. It explicitly permits its unlisted, project-owned ROM. No shipped
machine file or catalogue rule is changed. Assembly installs the image in
E0000h–FFFFFh shadow RAM and the pristine FFFE0000h–FFFFFFFFh copy. The reset
vector at FFFFFFF0h takes a near jump without changing the reset CS cache,
saves EDX and far jumps to F000:0000. The program then executes from shadow
RAM; descriptor accessed/busy bits can be written there.

| Region | Purpose |
|---|---|
| 01000h–017FFh | 256-entry IDT, default handler catches unexpected vectors |
| 05000h–0501Fh | Failure, delivery and frame counters |
| 06000h–06067h | 386 TSS; ESP0=88000h, SS0=10h, I/O map offset beyond limit |
| 07000h / 107000h | Real-mode A20 alias pair |
| 08000h | Real-mode and 16-bit exit stack top |
| 20000h / 120000h | Low guard and extended-memory dword |
| 86000h / 88000h / 90000h | User, TSS kernel-entry and initial kernel stack tops |
| F0000h onward | Program, GDT, messages and pseudo-descriptors in shadow RAM |

The GDT has flat 4 GiB USE32 ring-0 code/data, 16-bit code at F0000h,
16-bit data, a 256-byte data segment at 20000h, flat USE32 ring-3 code/data
and an available 386 TSS. Paging stays disabled. Ring 3 runs with IOPL zero
and uses a DPL-3 INT gate; it performs no port I/O. No TSS I/O bitmap is used.

## Transcript contract

Each numbered check emits its label followed by `ok` or `FAIL` and LF on port
402h. Failures increment a word printed as four hexadecimal digits after the
return to real mode. An unexpected exception prints its own line and halts,
which cannot match the success golden.

| Line | What it checks and why |
|---|---|
| `reset dh=03` | Saved reset EDX has DH=03h; DL stepping is deliberately unconstrained. |
| `flags 12-15 0000/7000` | Real-mode POPF can clear and set IOPL/NT, while bit 15 stays clear: the 386 identification behaviour, without CPUID. |
| `a20 off alias` | Clear the 8042 and 92h sources, write distinct values to the alias pair, and observe wrapping. |
| `a20 port92 on` | Set/read back 92h bit 1 and observe independent low/high values with the 8042 source still off. |
| `protected32 code stack` | Set CR0.PE, far jump to USE32 CS=08h, load SS=10h and ESP=90000h. Push 1234ABCDh, require ESP=8FFFCh and that dword at linear 8FFFCh, then pop it. |
| `extended memory` | Write/read 89ABCDEFh at 120000h with an unchanged 1357h guard at 20000h. |
| `limit gp(0) resume` | Read a dword at offset 100h of the 256-byte segment. The vector-13 handler requires error code zero, the exact fault EIP and CS=08h, counts once, rewrites saved EIP and resumes with IRETD. |
| `interrupt trap gates` | INT 30h and INT 31h round trips; interrupt gate clears handler IF, trap gate preserves IF. Both increment exactly once. PIC lines are masked during this check. |
| `timer ring0 count=3` | Remap master/slave to 20h/28h with complete ICWs, unmask only IRQ0, set channel 0 Mode 2 divisor 1000, STI and poll for three IRQ0 deliveries. Every handler requires saved CS=08h and EOIs the master. CLI freezes an exact count of three. |
| `ring3 int tss stack` | LTR, IRETD to CS=33h/SS=3Bh/ESP=86000h, then INT 40h to ring 0. Handler checks live SS=10h (TSS.SS0), ESP=ESP0−20 and saved CS/SS/ESP, including the first return EIP, and IRETD returns to ring 3 with IF set. The second INT completes at ring 0. |
| `ring3 timer tss stack` | IRQ0 at ring 3 must enter with live SS=10h (TSS.SS0) and ESP=ESP0−20, save CS=33h/SS=3Bh/ESP=86000h and an EIP inside the polling loop. It EOIs, clears IF in the saved frame to prevent a second tick, and IRETD resumes ring 3. Exactly one such IRQ and no bad frame are required. |
| `real mode return` | Enter the 16-bit code segment, reload 16-bit data/stack caches, clear PE, far jump, reload all real-mode segments and the IVT, then check PE is clear. |

Both polling loops allow 100,000 iterations, and the host imposes a 500,000
retired-instruction cap. Missing IRQs, a failure to return, unexpected faults
and CPU refusals fail the golden instead of silently skipping a check. The
program avoids protected-mode bit-test, decimal-adjust, WAIT/x87 and MOV DR/TR
instructions, as required by the pinned core's measured instruction census.

## Coverage boundary

This covers the unpaged part of gap-doc implementation item 1 and the
protected timer/PIC INTA path of item 7. It does not qualify all of either
original acceptance list: paging and recoverable #PF, injection during
incomplete MMIO, shutdown after failed delivery, and exhaustive interrupt
shadows remain separate. Ring-3 INT and timer delivery measure the 32-bit
TSS stack switch; call gates and task switches are not exercised here.
Gap-doc item 8's full paged/REP/deferred-I/O scheduler matrix remains pending.
No DOS extender, DPMI host, SeaBIOS boot or I/O bitmap support is claimed.

## Measured golden and regression evidence

**MEASURED** at x86jit `99ed385ea653bbc7e66b16cd1de0dfd42366fa75`, nasm
3.01. The ROM is 131,072 bytes, SHA-256
`39592698161d868e8d8173f4e31251d0fdd44eb5f770336c473c0b8df9b38eeb`.
The golden pins both the ROM hash and CPU revision and asserts exactly 8,120
retired instructions, below the 500,000 cap. Two complete runs agree.
No instruction was refused and no requested stage was omitted.

```text
i386-pm-test
reset dh=03 ok
flags 12-15 0000/7000 ok
a20 off alias ok
a20 port92 on ok
protected32 code stack ok
extended memory ok
limit gp(0) resume ok
interrupt trap gates ok
timer ring0 count=3 ok
ring3 int tss stack ok
ring3 timer tss stack ok
real mode return ok
failures 0000
```

The host's delivery log requires exactly nine events with no loss: one
fault (vector 13, error zero), four software interrupts, and four hardware
interrupts at vector 20h (three CPL 0→0 and one CPL 3→0). Two software INTs
must transition CPL 3→0 through vector 40h. The guest checks the exact #GP
EIP and ring-entry frames; the host checks the architectural event class,
so an INT substituted for a hardware timer cannot satisfy this oracle.

Both tests use the same fixture loader, which requires the 128 KiB size,
the build-script SHA-256 pin and the CPU revision before execution.
A second test compares the entire transcript, retired count, virtual time
and full delivery records with Deadline unchunked and split into 1- and
17-instruction runs, and PerClock split into 257-instruction runs. This is
bounded unpaged scheduler evidence, not the full gap-doc item 8 matrix.

**MEASURED mutations:** each source mutation is assembled with a temporary
matching build-script SHA-256 pin, so fixture validation passes and both tests
reach the transcript assertion. Each mutation makes both tests fail:

| Mutation | Guest evidence |
|---|---|
| INT handler expects live SS=18h instead of 10h | Both ring-3 lines `FAIL`, `failures 0002`, 7,335 retired instructions. |
| IRQ handler expects live SS=18h instead of 10h | Timer ring-3 line `FAIL`, `failures 0001`, 310,988 retired instructions. |
| Pushed ESP expected at KSTACK−8 instead of KSTACK−4 | Stack line `FAIL`, `failures 0001`, 8,134 retired instructions. |
| Sentinel read from linear KSTACK−8 instead of KSTACK−4 | Stack line `FAIL`, `failures 0001`, 8,134 retired instructions. |

Separately, truncate the image by one byte or change one padding byte without
updating the pin. Both tests fail at the shared loader's size or hash assertion,
respectively, before executing the ROM. Restore mutated files with plain copies
and touch them, then rerun the offline hash-checking build before the final
passing tests. No emulator or device implementation changed; these are negative
integration-test checks rather than device-fix before/after claims.

## 486 extension

The separate project-owned `vendor/i486-pm-test/` ROM keeps the existing 386
fixture and transcript unchanged. It shares the 128 KiB shadow/high-copy
layout and offline build pattern, but needs only a flat ring-0 USE32 segment
for the 486-specific checks. One image detects either DX variant through
EFLAGS.ID; the host selects the CPU explicitly. The build script pins the
image SHA-256 `981f23f80d98f19ec02528eb8da5f1e67442cad7481f7320b445ceb9f001aff8`.

The exact common transcript is:

```text
i486-pm-test
reset signature ok
reset cr0=60000010 ok
bswap32 ok
xadd register memory ok
cmpxchg success failure ok
cr0 wp am readback ok
eflags ac readback ok
```

The early DX then prints `eflags id fixed; cpuid absent`. The CPUID DX prints:

```text
eflags id toggle; cpuid present
cpuid 0 GenuineIntel max=1 ok
cpuid 1 eax=0410 edx=0001 ok
```

Both end with:

```text
invd wbinvd invlpg ok
failures 0000
```

The guest compares reset EDX against 0403h or 0410h according to the detected
ID policy, and CR0 against 60000010h before setting PE. BSWAP must reverse
12345678h to 78563412h. XADD checks both the sum and old destination in
register and memory forms. CMPXCHG checks successful replacement, failed
comparison's accumulator load and ZF, and the register form. WP/AM and AC
are set and read back at CPL 0, then restored; no supervisor write-protection
or CPL-3 #AC behavior is implied. CPUID compares all four registers for both
leaves, including zero leaf-1 EBX/ECX. Cache instructions must execute without
fault at CPL 0 with paging disabled.

The golden compares Deadline whole runs with chunks of 1 and 17 and PerClock
chunks of 257, including the transcript, retired count and virtual clock.
It requires zero delivery events. The 50,000-step cap bounds failures.
Intel486 DX Programmer's Reference Manual §4, §10 and Chapter 26 and Embedded
Intel486 Processor Family Developer's Manual Appendix D define these checks.
Paging, #AC delivery, cache contents and CR0.NE-driven FERR routing remain
outside this ROM's scope.

**MEASURED at CPU pin `274d48a8` (2026-11-24):** the early DX halts at
1,694 retired instructions; the CPUID DX at 2,119. Both report zero failures
and zero deliveries. Repeated runs and the complete scheduler/chunk matrix
agree. The 386 ROM still halts at 8,120 retired instructions; no existing
expected transcript or retired count was adjusted for the CPU repin.

**MEASURED regression mutations:** replacing only the adapter's 486 reset
image with `i386_reset` fails the CPU reset test, the all-source board reset
test and all three 486 ROM tests. The ROM reports both reset checks as `FAIL`
and `failures 0002`. Changing the BSWAP oracle to expect 78563413h (while
rebuilding with a temporary matching image hash) fails all three ROM tests
at the transcript assertion with `bswap32 FAIL` and `failures 0001`.
Both mutations were restored by plain copies followed by `touch`; the final
runs rebuild the restored sources. Removing the two 486 schema spellings
fails all twelve new machine-file tests. Disabling the integrated-FPU
validation fails its board validation regression.

## Two-level paging ROM

The sibling `vendor/i386-paging-test` fixture runs on 80386DX, early 80486DX
and CPUID 80486DX with CPU pin
`d8bf798ed05520b6b06bbb787a71f3906a0c65d9`. Intel 80386 Programmer's Reference
Manual §5.2 Page Translation, §6.4 Page-Level Protection and §9.8.14 Page
Fault define its checks; Intel486 Family Programmer's Reference Manual
§4.1.3 CR0.WP and §26 INVLPG define the 486 additions.

This is a separate BIOS-independent 128 KiB ROM, preserving the unpaged
ROM's bytes and transcript. The generic boards retain their ordinary devices,
COM1, 640 KiB low RAM and 15 MiB extended RAM. The reset copy at FFFE0000h
far-jumps to writable F0000h shadow; port 92h enables A20 before USE32 entry.
Build offline with `bash vendor/i386-paging-test/build.sh` in the devShell.
The image is SHA-256 pinned at
`a3d80f6e361719cb5b82da7fee983652f4a91b9fedee20f704b70f41f66881bf`.
Generated bytes are ignored. The fixtures CI lane builds it; the existing
vendor-selection rule includes its readers and slow goldens.

Layout: IDT=1000h, result/expected-fault fields=5000h, 386 TSS=6000h,
PD=10000h, identity PT=11000h, second PD=12000h, alias PT=13000h.
The first 4 MiB are identity mapped with user access intentionally enabled
for test code/data. Alias pages start at linear 00400000h and map physical
20000h–24000h: RW user alias, absent page, supervisor page, read-only user
page, writable user page. PDEs at linear 00800000h and 00C00000h impose
supervisor-only and read-only restrictions independently of PTE permissions.
Ring-0, transition and user stacks are 90000h, 88000h and 86000h.

The fault handler validates CR2, the hardware error-code dword and the saved
instruction address, then changes the saved EIP to a known resume label.
Every expected fault increments a counter; missing faults therefore fail too.
All fault addresses and error codes are printed, and mismatched frames
increment the final failure count. The new-CR3 test clears the old
directory's alias PDE before loading the second directory, so success proves
that execution uses the new directory. The equal reload and 486 INVLPG cases
change a PTE before invalidating and check the newly selected physical data.

The exact shared transcript prefix is:

```text
i386-paging-test
identity alias ok
pde pte accessed ok
alias write dirty ok
#pf cr2=00401000 error=00000000
not present resume ok
cr3 equal reload ok
cr3 new directory ok
```

The 386 then prints `invlpg unavailable on 386`; both 486 variants print
`invlpg pte change ok`. All continue with:

```text
#pf cr2=00402000 error=00000005
#pf cr2=00403000 error=00000007
#pf cr2=00800000 error=00000005
#pf cr2=00C00000 error=00000007
ring3 pte pde us rw ok
ring3 writable ok
supervisor wp=0 write ok
```

The 386 prints `wp unavailable on 386`. The 486 variants print:

```text
#pf cr2=00403000 error=00000003
#pf cr2=00402000 error=00000003
supervisor wp=1 user supervisor ro ok
```

All end with `failures 00000000` and CLI/HLT. The golden compares the full
transcript and exact retired count with whole runs and 17-step chunks under
a 500,000 virtual-time-unit cap. This qualifies basic two-level paging integration,
not V86, PSE/PAE, all task-switch/exception atomicity cases, paging through
A20-sensitive aliases, device page-table storage, or hardware TLB timing.
The pinned CPU uses an uncached walker with bounded private instruction
proofs; conservative invalidation remains required for host restores.

**MEASURED:** the 386 halts after **8,666** retired instructions; both 486
variants after **9,373**, identically with whole runs and 17-step chunks.
Test-profile wall times were 5.1–8.9 ms (386), 5.9–9.8 ms (early DX) and
6.0–9.9 ms (CPUID DX). Every transcript ends with zero failures. A regression
mutation making only the user read-only PTE writable, rebuilt with a temporary
matching hash, loses the expected `#PF(7)`, prints `ring3 pte pde us rw FAIL`
and `failures 00000001`, and fails the exact-transcript golden. Source and
builder were restored by plain copies followed by touch and rebuilt before
final verification.

**MEASURED repin audit:** the full affected-workspace run, including ignored
non-user-ROM goldens, passes with existing unpaged retired counts, hashes
and transcripts unchanged. Existing Rust revision guards/comments move from
`5a17667764080c2fe7a33d446584a3e175b7923d` to the pin above. The only existing
retired expectation advanced is test386: 793,775 to 796,780 (+3,005), because
its formerly refused PE|PG write now executes and reaches protected stack/ring
tests before the first V86 IRETD. This is a later stop, not altered counting
for the old path. The x86jit diff keeps PG=0 lowering and adds the separate
paged translator/continuations, model-gated WP/INVLPG and CR3 task handoff.
