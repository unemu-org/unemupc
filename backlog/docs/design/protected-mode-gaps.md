# 386 protected-mode gaps

## Current unpaged machine qualification (2026-11-24)

**MEASURED**, CPU pin `b7c10b6fdda8f84d3eda9c5d7751fcda1c5893ed`:
[the project-owned 386 ROM](i386-pm-test.md) runs on `generic-386-isa`
from the high reset copy and low shadow, enters USE32 protected mode,
uses flat and limited segments, handles #GP(0) and software interrupt/trap
gates, takes three ring-0 and one ring-3 PIT IRQ through the remapped PIC,
uses a 386 TSS for ring transitions, and returns to real mode. All checks
pass at 8,120 retired instructions. Deadline/PerClock and run chunks agree.
This supplies the unpaged coverage of implementation items 1, 7 and 8 below;
it does not cover paging, a DOS extender, or their full acceptance matrices.
The investigation verdict and probes below describe their historical pin,
not the current unpaged CPU capabilities.

**MEASURED at the same pin:** SeaBIOS completes diskless POST and prints
`No bootable device.`. The probe stops at 5,885,053 retired instructions.
A synthetic floppy
checks INT 15h AH=88h, E801h and all E820 entries at 16 MiB and 4 MiB.
After complete POST, CMOS 0Fh=0 and 8042 FEh reach the resume marker.
The former REP NOP stop is passed: F3 90 executes as NOP on every model. See
[machine-file qualification](machine-files.md#generic-386-isa-board).

## 486 board coverage and gaps

**READ**, CPU pin `274d48a8`: `generic-486-isa` selects either `I80486Dx`
or `I80486DxCpuid`, always with the integrated FPU. The separate
[486 ROM](i386-pm-test.md#486-extension) covers unpaged instructions and
identification without changing the existing 386 ROM.

| Gap | Current boundary |
| --- | --- |
| Paging | CR0.PG is refused by x86jit (xj-srh.7, x86jit). CR0.WP can be set/read, but supervisor write protection needs paging and is not qualified. |
| Alignment check | CR0.AM and EFLAGS.AC can be read back at CPL 0. CPL-3 #AC is refused; no alignment-delivery test is claimed. |
| Numeric errors | External routing returns `UnsupportedModel` for both 486 entries until CR0.NE-driven FERR#/IGNNE# routing exists (xj-31k.4, x86jit). Internal routing is provisional. The board must not select the AT IRQ13/F0h error latch. |
| Cache test registers | TR3–TR5 are refused with typed CPU errors. TR6/TR7 semantics are outside the new ROM's coverage. |
| Cache effects | INVD/WBINVD and unpaged INVLPG execute, but the interpreter has no emulated cache or paged TLB to invalidate. |
| Word BSWAP | Refused by the CPU; only 32-bit BSWAP is tested. |

Architectural references: Intel486 DX Programmer's Reference Manual, §4 System
Architecture, §8 Exceptions, §10 Initialization, Chapter 26 instruction
reference; Embedded Intel486 Processor Family Developer's Manual, Appendix D
CPUID. No Windows, DOS extender or paged operating-system qualification is
implied by POST or the unpaged ROM.

## Context

**Historical verdict: the investigated core cannot execute a DOS extender's protected-mode
entry or a guest-paged 32-bit program.** The decode groundwork is useful,
but `Compat32` is a flat application execution context, not an emulated
80386 system. Legacy protected mode needs changes to the CPU-state, mode,
memory-translation and event-delivery seams. Adding opcodes or selecting
`Compat32` alone will not work.

This is a bounded investigation for the protected-mode scope in
[decision-16](../../decisions/decision-16%20-%20Scope%20a%20period%20PC%20from%20DOS%20to%20Windows%2098%20SE%20with%203dfx%20Voodoo.md).
It proposes work; it does not create or claim implementation beads. Firmware
selection is now recorded in decision-19 and
[firmware-and-machine-classes.md §3.6](firmware-and-machine-classes.md#36-phase-b-firmware-requirements-seabios-on-generic-386-isa).
No repository code, dependency pin, guest fixture or CPU checkout was changed.

Evidence vocabulary throughout:

- **MEASURED:** executed against unmodified code in this investigation.
- **READ:** source inspection or an architectural requirement from the named
  reference; not a runtime compatibility result.
- **ASSUMED:** a proposed scope choice or inference requiring later acceptance
  tests. In particular, no complete DOS extender was run.

The investigation CPU pin was `6cb817b86c1fbf463f7aab8c0da1ce0eec3db0b7`.
`Vm::set_cpu_model(CpuModel::I80286)` selects the machine profile on
`ibm-5170`, fixes the model for the VM lifetime and defaults to
`ExceptionDelivery::InGuest`. The separate I80386 core model is not selected
by this board. The former `LegacySystem` profile/context names and
`set_legacy_cpu_model` API are deprecated compatibility interfaces.
READ: the pin provides 286 real entry, descriptor caches, LGDT/LIDT, LMSW,
16-bit protected code/stack and same-CPL far transfers. At CPL 0, interrupt
and trap gates deliver INT n, INT 3, INTO, BOUND, exceptions, IRQs and
single-step through the IDT; IRET returns at CPL 0. Fault escalation reaches
#DF and `Exit::Shutdown` if delivery of #DF faults. LLDT/SLDT, LDT selectors,
LAR/LSL/VERR/VERW, ARPL and LTR/STR are available, with atomic descriptor
busy/accessed-bit writes; descriptor tables must reside in RAM. CPL-0 hardware
task switches (far JMP/CALL to a TSS or task gate, IRET
with NT, and IDT task gates), busy bits and MSW.TS are supported by
`xj-srh.18 (x86jit)` under `decision-24 (x86jit)` (READ). Since
`b4e64dff`, `xj-srh.9 (x86jit)` runs protected-mode code at CPL 1–3:
RETF, RETF n and IRET to an outer ring, interrupts, exceptions, IRQs and NMI
through a gate to an inner ring on the TSS's ring stack, task switches into
rings 1–3, and the CPL and IOPL checks of CLI/STI, POPF/IRET, port I/O, the
system instructions and (on the 286) LOCK (READ; the open privilege-level
test, [`at-privilege-test.md`](at-privilege-test.md), MEASURED). Since
`a11353e6` (the second landing of that x86jit work), the core runs far CALL and JMP through a call gate in the
GDT or LDT: 286 gates on both models (a 386 gate is #GP(selector) on the
80286), an inward CALL onto the target ring's TSS stack that pushes the old
SS:SP, copies the gate's parameter words and pushes CS:IP, RETF n back,
same-level calls and conforming targets without a stack switch, and JMP
through a gate to the same level only (READ; case (e) of the open
privilege-level test, MEASURED). `LegacyGateOrTaskTransfer` is no longer
produced. Still refused: an inward call-gate CALL whose frame copy would need
more than the core's 64 retry-ledger entries (parameters and frame on a
device), as `LegacyInstruction`, and LOADALL with a CS DPL of 1–3 as
`I286LoadallTable`. V86 and paging remain outside this machine's scope.
Real-mode exceptions (#DE, #UD, CS fetch/word limit faults, BOUND and INTO)
are delivered through the IVT. Real-mode TF delivers vector 1 after the
successor of a TF-setting POPF/IRET; only SS loads inhibit one instruction.
See Intel 80286 Programmer's Reference Manual §9.7.1 and Appendix B MOV/POP.
Explicit outward exceptions retain `CpuError::Fault` with vector, optional
error code and CS:IP; unsupported machine contexts retain typed diagnostics.
Bare assembly rigs without an A20 source and the separate `new_real16_bare`
constructor retain Application(Real16). Bare rigs with an 8042 or port 92h use
I80286 Machine with in-guest processor exceptions and a low reset fetch.
IBM 5170 reset abandons pending execution, then restores the 286 register/cache
image, including MSW FFF0h, IVT limit 03FFh and reset-origin CS base FF0000h.
The assembler maps the single FE0000h–FFFFFFh board ROM alias. Bare A20 rigs
restore the same registers but retain their synthetic low FFFF0h fetch.
CPU shutdown uses the same `ResetRequested` boundary and CPU-only reset
protocol as the 8042: RAM, CMOS shutdown byte, A20 latch, device state and
instruction/time counters survive. READ: IBM Personal Computer AT Technical
Reference, System Board, Logic Diagrams (shutdown/reset decoding) and Keyboard
Controller Output Port (processor reset). Pulse timing is not modelled.
The `ibm-5170` FPU defaults to `None`; `cpu.fpu = "80287"` selects `Present`,
with emulation disabled. Bare rigs keep their present-FPU behaviour. READ:
the current core supports the Application Real16 instruction set in legacy
real mode, including AAD/AAM immediate bases, BCD/SALC, x87 under MSW.EM/TS
checks and resumable REP INS/OUTS. This resolves the AAD, FNINIT and REP INSB
refusals measured during initial integration at `a92b676`. FNINIT/FNSTCW
stores 037Fh with the present model. Under `decision-25 (x86jit)`, absent
I286 FNSTCW m16 and FNSTSW m16/AX return FFFFh, other output stores preserve
memory and WAIT completes, subject to the ordinary EM/TS/MP #NM gates.
I386 absent outputs retain refusal. Intel 80286 and 80287 Programmer's
Reference Manual, Part I Appendix A and Part II Appendix B, item 10, ground
the chosen all-high bus default; it is not a universal electrical value.
Derived CMOS equipment bit 1 follows the selected FPU; explicit images do not.
Present AT coprocessors route ERROR# through IRQ13, with F0h acknowledging the
board BUSY latch without clearing numeric status. This does not extend the
protected-mode x87 forms accepted by the CPU; see
[coprocessor controls](machine-config.md#coprocessor-controls).
See [machine configuration](machine-config.md#cpu-model-selection).
The deliberate generation differences remain: I286 rejects 386-only 66h/67h
forms with #UD, and legacy follows the architectural operand/stack rules.
Synthetic tests exercise real-mode compatibility, protected entry, outward
#GP, FFFFh word-limit delivery through the IVT, shutdown after invalid #DF
delivery, and reset/resume through retained RAM and CMOS.
User-ROM measurements await the supervisor; these changes do not establish
IBM BIOS POST completion or a Landmark protected-mode pass.
The investigation below retains its original revision and conclusions.
INS/OUTS are now implemented, including Real16 REP port handshakes and
per-element accounting. This resolves the opcode/handshake portion of the
historical string-I/O gap below; protected-mode privilege checks and the
broader system-mode work remain separate requirements.

Historical READ: at investigation time root `Cargo.toml:36` pinned `x86jit-core` to
`96a49a9063cdf0a2bdb2d916f01b381fbad0176e`. All CPU paths below are relative
to that revision's `x86jit-core/src/`; line numbers refer to that checkout,
whose HEAD was also verified. Machine paths are relative to this repository.
The probe used the cached core by path; the machine adapter resolved its
normal git dependency to the same revision. These are two Cargo package
instances with the same source revision, not a patched dependency.

READ: `lift/mod.rs:66` defines `CpuMode` as a decode/lift context;
`lift/mod.rs:87` has only `Long64`, `Compat32` and `Real16`.
`vm.rs:1367` explicitly makes the vCPU's mode constant for its lifetime.
`Vm::set_cpu_mode` (`vm.rs:928`) selects the mode inherited by **new** vCPUs;
it is not a guest-driven mode switch. `state.rs:349` explicitly says that
descriptor tables are not modeled. The existing selector fields and
`rflags_system` image (`state.rs:428`) must not be mistaken for implemented
descriptor or privilege state: that path assumes CPL 3.

ASSUMED scope for the first executable result: an interpreter-only 386
legacy-system path, initially CPL 0, with USE16/USE32 code, guest-owned
GDT/IDT, 4 KiB two-level paging, and transitions back to real mode for DOS
services. Then qualify an open extender on a clean DOS boot without an
existing DPMI/VCPI server. This does not assume that every extender runs its
client at CPL 0. Ring transitions and TSS support remain explicit follow-up
requirements where the selected host/client needs them. V86, hardware task
switching, debugging and an FPU are not prerequisites for the first integer
probe, and are not silently promised by it.

Architectural reference: **Intel 80386 Programmer's Reference Manual (1986)**,
abbreviated **PRM** below. Links point to a transcription of that Intel
manual, not to the behavior of a modern x86 processor. In particular, the
386 paging target has no CR4, PAE, PSE, NX, CR0.WP or INVLPG requirement.

## Probe results

### P1 — complete entry and paging attempt

MEASURED: the scratch Cargo executable loaded a NASM flat binary at
`CS:IP=0000:1000`, with DS/ES/SS zero, SP `9000h`, IF initially set, and
8 MiB of mapped RWX RAM. The interpreter was selected through
`Vm::set_cpu_mode(CpuMode::Real16)` before creating the vCPU. All writes
used `Vm::write_bytes`. The hand-written program was:

```asm
bits 16
org 0x1000
cli
lgdt [gdtr]
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
mov fs, ax
mov gs, ax
mov esp, 0x9000
mov eax, 0x3000
mov cr3, eax
mov eax, cr0
or eax, 0x80000000
mov cr0, eax
jmp short paged
paged:
mov eax, [0x00400000]
mov [0x5000], eax
hlt
align 8
gdt:
dq 0
dq 0x00cf9a000000ffff
dq 0x00cf92000000ffff
gdtr:
dw 23
dd gdt
```

The harness seeded these guest tables/data, rather than requiring the entry
stub to build them:

| Physical location | Contents / intended translation |
| --- | --- |
| `3000h` | PDE[0] = `6003h`; PDE[1] = `7003h` |
| `6000h` | 1024 PTEs: `i * 1000h + 3`, identity-map the first 4 MiB |
| `7000h` | PTE[0] = `2003h`: linear `00400000h` maps to physical `2000h` |
| `2000h` | Dword `12345678h` |
| `00400000h` | Dword `DEADBEEFh`, a sentinel for untranslated access |
| `5000h` | Initially zero; expected final marker `12345678h` |

MEASURED, `run_steps(&vm, 100, StepOptions::default())` returned:

```text
Ok(StepRun {
  steps: 1,
  stop: Exit(UnknownInstruction {
    addr: 4097, bytes: [15, 1, 22, 104, 16, ...], len: 5
  })
})
RIP=1001h CS=0000h IF=false retired_instructions=1 marker=[0,0,0,0]
```

This is `LGDT [1068h]`, encoding `0F 01 16 68 10`, immediately after CLI.
Neither CR0 write, the far jump, segment loads nor paging enable was reached.
There is **no measured paged execution** and no measured protected-mode
fault delivery. `UnknownInstruction` here is an emulator implementation
exit, not an architectural claim that LGDT is illegal on a 386.

MEASURED: the assembled image is 110 bytes; SHA-256:
`22d7b2a194a46a411f1b7426efd9b2d0896575fea942f0c633fc36372f3e0bb4`.
The GDT is at `1050h`, protected entry at `1018h`, CR3 write at `1030h`,
PG-setting CR0 write at `103Bh`, and marker load at `1040h`.

### P2 — isolated instruction coverage

MEASURED: each row was also tested from a fresh Real16 VM at `1000h`
using `Vcpu::step_instruction`, with AX initially `0010h`. Unsupported
cases kept RIP at `1000h`. The 33 cases produced 28 `UnknownInstruction`
exits and five `Continue` results. Grouped encodings below list **every**
case; `m` denotes the direct memory operand `[0800h]` used for table ops.

LAR/LSL, VERR/VERW and LLDT/SLDT/LTR/STR are protected-mode operations:
their Real16 probes measure the current emulator exit, not a legal real-mode
use or execution in protected mode. A 386 should report the specified #UD
for their real-mode use (PRM Chapter 17, named instruction entries). Their
missing protected-mode implementation is established by the source audit,
not by treating these Real16 cases as protected-mode tests.

| Instruction(s) | Bytes (hex) | Measured result |
| --- | --- | --- |
| CLI | `FA` | Continue, RIP=`1001h` |
| LGDT m / LIDT m / SGDT m / SIDT m | `0F 01 16 00 08` / `0F 01 1E 00 08` / `0F 01 06 00 08` / `0F 01 0E 00 08` | UnknownInstruction, length 5 each |
| SMSW AX / LMSW AX | `0F 01 E0` / `0F 01 F0` | UnknownInstruction, length 3 each |
| MOV EAX,CR0 / MOV CR0,EAX | `0F 20 C0` / `0F 22 C0` | UnknownInstruction, length 3 each |
| MOV CR3,EAX | `0F 22 D8` | UnknownInstruction, length 3 |
| MOV EAX,CR2 / MOV CR2,EAX | `0F 20 D0` / `0F 22 D0` | UnknownInstruction, length 3 each |
| CLTS | `0F 06` | UnknownInstruction, length 2 |
| LLDT AX / LTR AX / SLDT AX / STR AX | `0F 00 D0` / `0F 00 D8` / `0F 00 C0` / `0F 00 C8` | UnknownInstruction, length 3 each |
| LAR AX,AX / LSL AX,AX | `0F 02 C0` / `0F 03 C0` | UnknownInstruction, length 3 each |
| VERR AX / VERW AX | `0F 00 E0` / `0F 00 E8` | UnknownInstruction, length 3 each |
| MOV EAX,12345678h | `66 B8 78 56 34 12` | Continue, EAX=`12345678h`, RIP=`1006h` |
| JMP 0008:1100 (16:16) | `EA 00 11 08 00` | Continue, CS=`0008h`, IP=`1100h`; real-mode selector update |
| JMP 0008:00001100 (16:32) | `66 EA 00 11 00 00 08 00` | UnknownInstruction, length 8 |
| MOV FS,AX / MOV GS,AX | `8E E0` / `8E E8` | UnknownInstruction, length 2 each |
| MOV DS,AX | `8E D8` | Continue, DS=`0010h`; no descriptor lookup |
| PUSHFD / POPFD / IRETD | `66 9C` / `66 9D` / `66 CF` | UnknownInstruction, length 2 each |
| PUSHAD / POPAD | `66 60` / `66 61` | UnknownInstruction, length 2 each |
| MOV EAX,[00400000h], address-size 32 | `67 66 A1 00 00 40 00` | Continue, EAX=`DEADBEEFh`, RIP=`1007h` |

READ: `lift/control.rs:839` accepts only `FarBranch16` for the immediate
real-mode far target; line 868 deliberately rejects the 386 16:32 form.
`lift/mod.rs:4227` implements Real16 CLI/STI, but lines 4235–4248 select
only 16-bit FLAGS/IRET forms. `lift/mod.rs:4360` selects only PUSHA/POPA,
not PUSHAD/POPAD. These are specific gaps despite successful 32-bit MOV
decoding.

The address-size-32 case is also a correctness warning, not a successful
386 memory test. READ: `interp/mod.rs:4463` truncates the limit-check
operand to 16 bits, whereas `lift/mod.rs:4980` adds the full address to
the segment base. With a normal 64 KiB real-mode DS cache, this reference
should not access 4 MiB. The probe shows that prefix acceptance does not
establish correct 386 segment-limit behavior. PRM §14.6 and §16.1 govern
the required behavior.

### P3 — Compat32 is not a protected-mode substitute

MEASURED: fresh Compat32 VMs, IF=true, same mapped RAM:

| Operation | Result |
| --- | --- |
| `run_steps(..., 1, ...)` | `Err(UnsupportedMode)` in each of four independent cases |
| `irq_ready()` | false in each case, despite IF=true |
| Step `MOV DS,AX` (`8E D8`) | UnknownInstruction at `1000h` |
| Step CLI (`FA`) | UnknownInstruction at `1000h` |
| Step `INT 31h` (`CD 31`) | `Exit::Exception`, vector 49, saved RIP=`1002h`; software trap metadata, no error code; no guest handler |
| Step `MOV EAX,[00400000h]` (`A1 00 00 40 00`) | Continue, EAX=`DEADBEEFh`; seeded PDE/PTEs have no effect |

The last row is a flat-map control, **not** a test with PG=1: there is no
CR0 state to seed. READ: `vm.rs:1860` refuses step-bounded execution
outside Real16; `vm.rs:1502` excludes other modes from readiness.
`lift/control.rs:1152` delegates user-mode segment loads as unsupported;
`lift/mod.rs:4211` emits an outward trap for INT n (and a host syscall
convention for INT 80h). These behaviors cannot serve a guest DPMI host.

### P4 — manual IRQ injection can accidentally use real-mode delivery

MEASURED: in a Compat32 VM, set IF=true, ESP=`9000h`, install IVT vector
20h at physical `80h` as `0000:1800`, put HLT at `1800h`, queue
`inject_irq(20h)`, and call `run(&vm, Some(3))`. The result was:

```text
Hlt; queued=false; RIP=1801h; ESP=8FFAh
bytes at 8FFAh = [00,10,00,00,02,02]
```

Those six bytes are a 16-bit IP/CS/FLAGS frame (`1000h`, `0000h`,
`0202h`). Three scratch assertions verified HLT, RIP and ESP. READ:
`vm.rs:2336` checks the pending IRQ and completion/shadow gate without
a mode guard, and calls `interp::deliver_interrupt`; the latter uses
`vector*4` and 16-bit pushes (`interp/control.rs:614`). Comments describe
this as real-mode-only caller usage. The machine's readiness check normally
prevents this misuse. It is neither IDT support nor a reason to enable
Compat32 IRQ readiness before protected-mode delivery exists.

### P5 — memory above 1 MiB and board A20 integration

Historical MEASURED (before board integration): a scratch test using the
repository's `X86JitCpu` mapped 2 MiB
with `new_real16_bare`/`map_ram`, wrote `11h` at `20h` and `77h` at
`100020h`, then ran `MOV AX,FFFFh; MOV DS,AX; MOV AL,[0030h]; HLT`.
It halted after four steps with AL=`77h`. Separately, the repository's
8042 received command D1h followed by data `01h`, and reported
`a20_enabled()=false`; this did not change the CPU result.

That adapter/device seam probe predates the assembled board wiring. The shared
8042/92h line now drives `Cpu::set_a20_enabled`: assembly applies the initial
state, and the machine forwards effective transitions after port writes and
before runs. IBM 5170 and bare rigs with an 8042 or port 92h source use the
I80286 Machine profile, with in-guest processor exceptions; bare rigs without
either source retain flat Application(Real16).
The baseline is 0 for a fresh line, 1 after attaching the enabled 8042. CPU-only
reset preserves source latches and effective/pending gate state. Host physical
memory access and ISA DMA bypass the CPU gate. See machine-config.md §A20 line
and System Control Port A. The original 1 MiB BIOS VM size was never an
adapter-wide address-space limit.
Assembly now uses the AT's fixed 24-bit decode and 16 MiB VM, requires a
matching CPU width, and fills holes after RAM, ROM, MMIO and existing
adapter-gap mappings. Wider CPUs require a separate sparse-mapping design.
Empty-bus reads return FFh and guest writes are discarded using immutable
backend ROM mappings. **ASSUMED:** FFh as the floating value. **READ:** IBM
Personal Computer AT Technical Reference, September 1985, Section 1 "System
Board", System Memory Map (p. 1-8), assigns the high expansion-memory range
and ROM aliases; it does not establish the absent-memory electrical value.
The board's FE0000h–FFFFFFh alias is a read-only copy of the resolved low
E0000h–FFFFFh map, including ROM and zero-filled gaps; `map_at_high_alias` is
its single owner. I286 reset-origin fetch at FFFFF0h reads that alias;
bare Application Real16 retains its low FFFF0h fetch. MMIO in
the low alias source is rejected because a static copy cannot mirror a device.
Bare rigs retain their prior VM/map.
Configured extended memory remains RAM, with empty bus above its end;
reduced conventional memory also leaves empty-bus holes below 1 MiB.
See [MemoryConfig](machine-config.md#memoryconfig--a-semantic-memory-map).
With A20 enabled, 100000h reads installed extended RAM or empty bus. With
A20 disabled, CPU reads and writes at that address alias low RAM at 000000h.

### P6 — open extender inspection and deliberate runtime scope cut

READ: inspected the public DOS/32A 9.12 source at
[amindlost/dos32a](https://github.com/amindlost/dos32a), specifically:

- `src/dos32a/text/kernel/mode.asm`, blob
  `a444de6344974d74e86fe5ede971e31445295a7d`, lines 132–157:
  the raw/XMS entry saves EFLAGS with PUSHFD, uses CLI, LIDT, LGDT,
  MOV CR0, a 16:16 far jump into protected USE16 code, protected
  segment reloads including FS/GS, an ESP load, then IRETD to its target.
  Lines 170–199 clear PE and return to real mode. This source requires
  mixed USE16/USE32 execution; a direct Real16-to-flat-Compat32 switch
  would miss its actual sequence.
- `detect.asm`, blob `766593291e5c78712019e992b482dbb5ce0d1a5f`,
  line 122 uses SMSW and lines 328–361 use PUSHFD/POPFD for CPU
  identification. Lines 167–205 probe optional DPMI/VCPI hosts.
- `init.asm`, blob `358c31368458020fa9d15f6cb8ef44aa3ea2b204`,
  lines 536–613 enable A20 and try E820h/E801h before AH=88h.
  Lines 775–797 touch CR3, CR0, CR2 and CLTS after entry. The CR4/FXSR
  branch at 784–795 is conditional on a newer CPU being reported.
  Line 149 uses LAR to inspect CS.

MEASURED evidence applicable to this source: P2's PUSHFD, SMSW, LIDT,
LGDT, MOV CR0/CR2/CR3 and CLTS forms all exit as unsupported at this
pin. These are isolated instruction measurements, not a replay of the
extender's assembled entry. ASSUMED: its raw path should be the first
extender qualification target, avoiding an EMS/V86 host and its extra
contracts. Rebuilding its DOS loader and creating a bootable client fixture
would not resolve the earlier core blockers, so no binary extender run was
attempted within this spike. Its complete dynamic instruction footprint and
client privilege requirements remain unmeasured.

### Reproduction and validation record

MEASURED: all harness sources, NASM listing, binary, Cargo outputs and
logs are confined to the assigned `o0j1-probe` scratch directory. The
executable uses `Vm::new(VmConfig::flat(...))`, `set_cpu_mode`, `map`,
`write_bytes`, `new_vcpu`, `set_reg`, `run_steps` and `step_instruction`;
it injects no CPUID, paging or mode behavior. Fresh VMs isolate instruction
cases; it uses no guest execution clock, randomness or threads.
The scratch artifacts are `Cargo.toml`, `Cargo.lock`, `src/main.rs`,
`entry.asm`, `entry.bin`, `entry.lst`, `probe.log`, `build.log`, `test.log`,
`clippy.log` and `fmt.log`, with build products under `target` and
`workspace-target`.

To reproduce, supply the assigned scratch directory as `PROBE_DIR` and
the provided environment file as `DEVSHELL_ENV`, then run from this
worktree (the environment is sourced before every Cargo invocation):

```sh
source "$DEVSHELL_ENV" && which cc cargo
source "$DEVSHELL_ENV" && nasm -f bin "$PROBE_DIR/entry.asm" -o "$PROBE_DIR/entry.bin" -l "$PROBE_DIR/entry.lst"
source "$DEVSHELL_ENV" && CARGO_TARGET_DIR="$PROBE_DIR/target" cargo run --offline --manifest-path "$PROBE_DIR/Cargo.toml"
source "$DEVSHELL_ENV" && CARGO_TARGET_DIR="$PROBE_DIR/workspace-target" cargo build --offline --all-targets
source "$DEVSHELL_ENV" && CARGO_TARGET_DIR="$PROBE_DIR/workspace-target" cargo test --offline --workspace
source "$DEVSHELL_ENV" && CARGO_TARGET_DIR="$PROBE_DIR/workspace-target" cargo clippy --offline --all-targets --all-features -- -D warnings
source "$DEVSHELL_ENV" && cargo fmt --check
```

Both `cc` and `cargo` resolved under `/nix/store`. Final command results:

| Command | Measured result |
| --- | --- |
| NASM assembly | Exit 0; binary/hash recorded in P1 |
| Scratch `cargo run --offline --manifest-path …` | Exit 0; 33 Real16 cases, four Compat32 cases, full-entry attempt, IRQ frame and adapter/A20 probe; three IRQ assertions pass |
| `cargo build --offline --all-targets` | Exit 0 |
| `cargo test --offline --workspace` | Exit 0; 48 result groups, 739 passed, 0 failed, 14 ignored, 0 measured, 2 filtered out |
| `cargo clippy --offline --all-targets --all-features -- -D warnings` | Exit 0 |
| `cargo fmt --check` | Exit 0 |

An initial scratch-only build failed with two E0277 diagnostics because
`StepResult` has no Debug implementation. The harness was corrected to
match Continue/Exit explicitly; the final run above passes. No Cargo
sandbox/registry permission failure occurred. The ignored/slow set was
not run: the brief did not authorize it. No repository tests were added,
because this change is an investigation document; P1–P5 record behavior
of the unmodified implementation, rather than a claimed fix with a
red/green regression test.

## CPU-core gap list

Each entry names the current implementation seam it would touch, not a
claim that a future feature can fit entirely within one file. All entries
are READ unless a P-number additionally supplies MEASURED evidence.

### Segmentation

| Gap | Current seam / evidence | Intel 80386 PRM requirement |
| --- | --- | --- |
| GDTR, LDTR, selector decoding and cached descriptors for all six segment registers | `state.rs:296`, `state.rs:349`, `state.rs:433`: selectors and FS/GS bases exist, no table registers or hidden caches | [§5.1.1–§5.1.4, Segment Translation](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s05_01.htm): bases, effective limits, access rights, GDT/LDT selection, null selectors, hidden state |
| Protected data/stack segment loads and validation | `lift/mod.rs:2233`, `lift/control.rs:1147`; P2/P3. Real16 loads derive base from selector; user-context loads are unsupported | [§6.3.1–§6.3.2, Segment-Level Protection](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s06_03.htm): type, P, DPL/RPL/CPL checks; readable/writable and expand-down semantics; accessed-bit update |
| Segment checks on fetches, stack, scalar and string accesses | `lift/mod.rs:4938`, `interp/mod.rs:4451`, `interp/control.rs:146`; fixed real-mode check and fixed 16-bit string indexing are insufficient | PRM §6.3.1.2 (Limit Checking), §6.3.2 (Restricting Access to Data); P2 exposes the 32-bit-offset limit-check hole |
| Independent CS.D and SS.B; protected USE16 code and mixed-width transfers | `lift/mod.rs:94`, `lift/mod.rs:5233`, `lift/control.rs:77`, `lift/control.rs:839`: one CpuMode currently couples decode, PC and stack rules | [§16.1, How the 80386 Implements 16-Bit and 32-Bit Features](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s16_01.htm); CS.D selects defaults, SS.B selects SP/ESP, gate width selects frame width |
| Protected far JMP/CALL/RETF/IRET and call gates, including changed CPL | `lift/mod.rs:4014`, `lift/control.rs:1217`, `interp/mod.rs:4359`: existing user IRET only accepts configured CS/SS; P2's far16 merely changes a real selector | PRM §6.3.3 (Restricting Control Transfers), §6.3.4 (Gate Descriptors Guard Procedure Entry Points), §16.4 (Transferring Control Among Mixed Code Segments), §17 CALL/JMP/RET/IRET |

Hidden descriptor caches must survive guest edits to their backing table
until an architectural reload. Recomputing a base from the current GDT on
every access would also be incorrect. Existing fixed real-mode limit
checks and same-selector user IRET are useful code, not descriptor support.

### Mode switch

| Gap | Current seam / evidence | Intel 80386 PRM requirement |
| --- | --- | --- |
| Architectural CR0 and mutable execution regime separate from decoder width | `state.rs:296`, `vm.rs:1367`, `vm.rs:932`, `lift/mod.rs:2254`; P1/P2 stop before PE can change | [§4.1.3, Control Registers](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s04_01.htm), [§10.3, Switching to Protected Mode](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s10_03.htm): PE starts protected execution at CPL 0 while old segment caches initially remain |
| PE transition, far CS reload, PG transition and return to real mode | `lift/mod.rs:4014`, `vm.rs:2376`, `lift/mod.rs:276`: fetch formation knows only real selector-shift or flat RIP; P1 | [§14.5, Switching Back to Real-Address Mode](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s14_05.htm), §10.3; CR0 writes and far transfers must end execution under stale context |
| Context-sensitive cache identity and invalidation | `vm.rs:3047`, `lift/mod.rs:241`: fetch physical address and CpuMode are currently sufficient only for existing regimes | PRM §5.1, §5.2.5, §16.1. ASSUMED implementation: include/validate segment context, decode defaults and translated code-page identity; invalidate relevant cached IR after descriptor-context, CR3, PG or A20 changes |
| 386 model and reset state distinct from the 286 real-mode contract | `lift/mod.rs:284`, `state.rs:428`, `vm.rs:1367`; Real16 has 286-specific policies and no CPU-generation state governing the transition | [§10.1–§10.2, Initialization](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s10_02.htm), §14.8; do not change existing 286 goldens by enabling all 386 semantics indiscriminately |

### Paging

| Gap | Current seam / evidence | Intel 80386 PRM requirement |
| --- | --- | --- |
| CPU-local linear-to-physical translation using CR3 and CR0.PG | `memory.rs:84`, `memory.rs:2152`, `memory.rs:3732`, `jit_abi.rs:89`: current accesses resolve guest address directly into backing; neither CR state nor access privilege is supplied to a page walk | [§5.2.1–§5.2.4, Page Translation](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s05_02.htm): 10/10/12-bit split, 1024-entry directory/table, 4 KiB frames; P3 is a flat-access control |
| Translation for instruction fetch and implicit system accesses as well as data | `lift/mod.rs:276`, `memory.rs:2174`, `interp/control.rs:614`; currently no page walk before decode, IVT fetch or stack access | PRM §5.3 (Combining Segment and Page Translation), [§6.4.3, Overrides to Page Protection](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s06_04.htm): descriptor/TSS/IDT and inner-stack accesses have supervisor rules |
| PDE/PTE P/U/S/R/W checks and A/D side effects | `memory.rs:2157`, `memory.rs:2265`; host map Prot/RegionKind is not a guest PTE | PRM §5.2.4.2–§5.2.4.4, §6.4.1–§6.4.2. Preserve 386 supervisor-write behavior; do not import later CR0.WP semantics |
| Guest #PF with CR2, complete architectural error code and restart state | `fault.rs:260`, `fault.rs:497`, `fault.rs:572`, `memory.rs:28`; existing flat-map projection deliberately cannot claim a complete architectural #PF | PRM §9.8.14 (Page Fault): record faulting linear address, P/W/U code and restart EIP; distinguish guest paging faults from host physical-map/MMIO failures |
| CR3/TLB semantics and physical alias coherence | `vm.rs:3047`, `memory.rs:1379`, `memory.rs:3732`; no architectural guest TLB exists | PRM §5.2.5 (Page Translation Cache). A software TLB is optional initially; CR3 reload must invalidate any cached translation, and stores through aliases must invalidate translated code by physical backing |

READ: the enum name `MemoryModel::SoftMmu` is not evidence of guest
paging. Its `map` arm is a `todo!` at `memory.rs:1761`. Existing SMC
code-page bitmaps, fetch-policy caches and host alias bookkeeping are
also not PDE/PTE walkers or architectural TLBs.

The initial page walker can be uncached. It must still split page-crossing
fetches and operands, preserve defined fault/restart state, route translated
MMIO to physical device addresses, and distinguish a guest not-present
PTE from an unmapped physical bus address. Do not emulate paging by
changing the shared Vm map per address space: CR3 belongs to each CPU.

### Interrupts/exceptions

| Gap | Current seam / evidence | Intel 80386 PRM requirement |
| --- | --- | --- |
| IDTR, 286/386 interrupt/trap gates and IDT limit/type/presence checks | `state.rs:296`, `interp/control.rs:614`, `vm.rs:2336`; P2/P4 | [§9.4, Interrupt Descriptor Table](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s09_04.htm), [§9.5, IDT Descriptors](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s09_05.htm): eight-byte entries, descriptor-selected target and gate width |
| Protected INT/INTR delivery and IRET, including IF/TF rules and software gate DPL | `lift/mod.rs:4158`, `lift/mod.rs:4211`, `lift/mod.rs:4260`, `vm.rs:1502`; P3/P4 | [§9.6.1, Interrupt Procedures](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s09_06.htm), §9.2: interrupt gate clears IF, trap gate preserves it; hardware IRQs do not use software-INT gate-DPL admission |
| Privilege-changing event entry and outer-level IRET | `lift/control.rs:1217`, `interp/mod.rs:4359`; configured-selector user return cannot restore a real protected privilege frame | PRM §9.6.1.1–§9.6.1.2, §6.3.4; stack switch through the TSS, correct saved SS:ESP and return validation |
| Precise fault/trap/abort delivery, architectural error codes and fault-on-delivery escalation | `vm.rs:1791`, `interp/control.rs:621`, `fault.rs:497`; Real16 re-vectors faults through the IVT, while Compat32 exits outward | [§9.7, Error Code](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s09_07.htm), [§9.8, Exception Conditions](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s09_08.htm): selector EXT/IDT/TI fields, restart RIP, #DF(0), shutdown if #DF delivery fails |

Required fault coverage, distinguished from an enum containing a vector:

- **#GP(13), #SS(12), #NP(11):** new descriptor, limit, privilege and
  gate checks, with the correct selector code or zero; PRM §9.8.11–§9.8.13.
- **#PF(14):** genuine guest paging violations, CR2 and code; §9.8.14.
- **#TS(10):** invalid task/stack-transition state when TSS/task machinery
  is enabled; §9.8.10. A flat CPL 0 probe does not establish this coverage.
- **#DF(8) and shutdown:** faults during event delivery require the
  exception-class combination rules, not unconditional recursive delivery;
  §9.8.8. A machine reset policy follows a reported shutdown.
- **#DE(0), #BP(3), #OF(4), #BR(5), #UD(6):** several producers already
  exist, but need protected delivery and applicable 32-bit forms. READ:
  `lift/mod.rs:4143` and `lift/mod.rs:4175` provide Real16-specialized
  producers; this is not a claim that all exception generation is absent.
- **#DB(1):** TF is stored without single-step delivery (`state.rs:428`);
  DR0–DR3/DR6/DR7 and precise debug conditions are absent from CpuState.
  PRM [§12.3, Debug Exceptions](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s12_03.htm).
  Defer debugger qualification unless the chosen extender requires it.
- **#NM(7) / coprocessor integration:** CR0.EM/MP/TS and CLTS need a
  coherent policy before claiming 386/387 software support; PRM §9.8.7
  and §11.1. Existing x87 execution does not establish those control bits
  or PC coprocessor error wiring. The first probe is integer-only.

### Task state

| Gap | Current seam / evidence | Intel 80386 PRM requirement |
| --- | --- | --- |
| TR, cached TSS descriptor, LTR/STR; TSS privilege stacks | `state.rs:296`, `lift/mod.rs:2221`, `interp/control.rs:614`; P2's LTR/STR unsupported | [§7.1, Task State Segment](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s07_01.htm), [§7.3, Task Register](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s07_03.htm): TR cache, descriptor validation/busy bit, SS0–SS2/ESP0–ESP2 for inward transitions |
| Dynamic CPL/IOPL and TSS I/O permission bitmap | `state.rs:607` implements fixed CPL 3 flag restore; `lift/mod.rs:4227` only provides real-mode CLI/STI | [§8.3.1–§8.3.2, Protection and I/O](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s08_03.htm): check permission before any port side effect; POPF/IRET IF and IOPL changes are privilege-sensitive |
| Hardware task switch, task gates, backlink, busy/NT/TS behavior | `state.rs:428`, `interp/mod.rs:4350`: NT is stored/rejected for the user-context IRET, not used for a task return | [§7.5–§7.6, Task Switching / Task Linking](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s07_05.htm), §9.6.2. Conditional scope; no evidence that the initial raw extender path needs task switching |
| V86 execution and transitions | `lift/mod.rs:87`, `state.rs:428`: a stored VM flag does not select V86 semantics | [§15.3, Entering and Leaving V86 Mode](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s15_03.htm), §15.4–§15.5. Needed for qualifying V86 memory-manager paths, not implied by running raw DOS/32A |

Do not equate loading TR with implementing a hardware task scheduler.
TSS privilege stacks and port permissions can be needed with software
task management, while a single-CPL entry/paging probe needs neither.

### Instructions

The following is a bounded minimum/audit list, not a complete 80386 opcode
conformance claim. PRM [§4.2, Systems Instructions](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s04_02.htm)
and the named entries in **PRM Chapter 17, 80386 Instruction Set** define
the encodings, privilege rules and faults.

| Family | Gap and current source seam | Evidence / PRM sections |
| --- | --- | --- |
| MOV CR0/CR2/CR3, SMSW/LMSW, CLTS | Add system-register operands and effects, not generic GPR MOV lowering (`lift/mod.rs:2254`, `state.rs:296`) | P2 MEASURED for listed forms; §4.1.3, §4.2, Chapter 17 MOV/SMSW/LMSW/CLTS |
| LGDT/SGDT, LIDT/SIDT, LLDT/SLDT | Add table-register state and operand-size-aware loads/stores (`lift/mod.rs:2221`, `state.rs:296`) | P2 MEASURED; §5.1.2, §9.4, Chapter 17 LGDT/LIDT/LLDT/SGDT/SIDT/SLDT |
| LTR/STR, LAR/LSL, VERR/VERW, ARPL | Descriptor/TSS queries and privilege semantics (`lift/mod.rs:2221`, `lift/control.rs:1147`) | P2 MEASURED except ARPL; ARPL absence READ from dispatch; §4.2, §6.3, §7.3, Chapter 17 named entries |
| MOV/POP Sreg, LDS/LES/LSS/LFS/LGS; PUSH/POP FS/GS | Complete real386 and protected forms, with caches/limits and SS inhibition (`lift/mod.rs:2233`, `lift/mod.rs:4381`) | P2/P3 measure MOV DS/FS/GS differences. Remaining forms READ: LSS/LFS/LGS absent, LDS/LES specialized to real mode; §3.10, §5.1.4, Chapter 17 MOV/POP/LDS/LSS |
| Far JMP/CALL/RETF; IRET/IRETD | 16:32 targets, mixed descriptor sizes, same/inter-level returns (`lift/control.rs:839`, `lift/control.rs:1217`) | P2 MEASURED far32 and Real16 IRETD rejection; §6.3.4, §9.6, §16.4, Chapter 17 named entries |
| PUSHFD/POPFD, PUSHAD/POPAD; CPU identification flags | Add 386 real/USE16/USE32 forms without fixed CPL 3 restore rules (`lift/mod.rs:4235`, `lift/mod.rs:4252`, `lift/mod.rs:4360`, `state.rs:607`) | P2 MEASURED Real16 rejection; §3.1, §3.8, §4.1.1, §16.1, Chapter 17 PUSHF/POPF/PUSHA/POPA |
| CLI/STI, HLT, IN/OUT/INS/OUTS | Correct CPL/IOPL/TSS gating as applicable (HLT requires CPL 0) and 16/32-bit string port handshakes (`lift/mod.rs:4110`, `lift/mod.rs:4227`, `interp/control.rs:104`) | P3 measures missing Compat32 CLI. HLT and scalar ports already exist; INS/OUTS have no dispatch arms (READ). §6.3.5.1, §8.2–§8.3, §9.2, Chapter 17 named entries |
| 32-bit addressing, REP and stack/loop forms | Audit all implicit registers and segments; accepting 66h/67h on MOV is insufficient (`interp/control.rs:141`, `lift/mod.rs:4316`, `lift/mod.rs:5233`) | P2 MEASURED partial support and limit-check problem; §16.1–§16.3, Chapter 17 address/operand-size attributes |
| MOV DRn and debug behavior | New debug state/operands (`state.rs:296`, `lift/mod.rs:2254`); outside first integer qualification | READ; §12.1–§12.3, Chapter 17 MOV |

ASSUMED: configure the selected 386 personality so extender CPU-detection
code does not discover modern capabilities and enter a CR4/FXSR branch.
The legacy 286 path and existing Long64/Compat32 application contracts
must retain their existing behavior. Reusing common integer IR is sensible;
reusing fixed user-mode privilege assumptions is not.

## Machine gap list

The 2026-11-11 firmware update below is **READ** from SeaBIOS rel-1.17.0
(`b52ca86e094d19b58e2304417787e96b940e39c6`); its file:line references and
full requirement table are in
[firmware-and-machine-classes.md §3.6](firmware-and-machine-classes.md#36-phase-b-firmware-requirements-seabios-on-generic-386-isa).
QEMU file:line citations (including CMOS `pc.c:403–485`) refer to
`29760efaf96bbe5928f8111881eddbf5ac06439e` (`29760ef`); Bochs references
in the linked requirement list use `44104eeaff16107e478ea8bd9c55bd8601a80191`
(`44104ee`). These source findings are not a measured SeaBIOS boot. Beads
cited here are existing follow-up work filed by the supervisor.

1. **Extended RAM and its discovery.** READ:
   `crates/assembly/src/memory.rs:68` already has arbitrary address-space
   size and RAM regions; `assemble.rs:250` maps those regions; P5 measures
   a larger adapter map. Firmware-board assembly now covers the full 16 MiB
   286 physical extent with empty bus outside installed regions; the BIOS
   preset still installs no extended RAM. Add an extended
   RAM region starting at `100000h`, preserve the conventional-memory/MMIO/
   ROM holes (`upc-o0j.6`). `assemble.rs:448` already derives extended KB,
   so avoid inventing a second memory-size authority. Firmware-owned CMOS
   layout (`upc-o0j.11`) must also seed QEMU/Bochs 30h/31h, 34h/35h and
   5Bh–5Dh, not only current 17h/18h; include century 32h, boot order
   38h/3Dh, ATA translation 39h and CPU count 5Fh (SeaBIOS
   `paravirt.c:240–254,535,588–592`, `clock.c:48`, `boot.c:471–489`,
   `block.c:84–90`). Derive bytes from the description and test against QEMU
   `pc.c:403–485` without changing the 5170 seed. AH=88h must be compared
   against E820, not CMOS (item 3). ASSUMED first profile: 16 MiB total
   physical extent, with 15 MiB contiguous extended RAM.

2. **A20 physical-address effect and board wiring.** READ:
   Device side complete: `crates/devices/src/a20.rs` exposes a shared line;
   `sys_ctrl_port_a.rs` implements port 92h, snapshot v1 and the alternate
   reset event. Assembly connects 8042 bit 1 and 92h bit 1 with an ASSUMED
   OR policy, preserving the existing 8042-enabled power-on default. Source
   changes are visible synchronously. Bare rigs admit the device; ibm-5170
   rejects it because the AT has no port 92h. Both reset sources share the
   existing machine reset line. IBM ThinkPad 560 Technical Reference, "System
   Control Port A (Hex 0092)", reproduces the PS/2 reset contract: a bit-0 0→1
   write pulses reset; system reset or writing zero clears the bit, and the
   latch stays set during reset for POST. Bit 1 is zero after system reset.
   Intel386 EX User's Manual, PORT92, CPURST, confirms that bit 0 remains one
   after CPU-RESET and must be cleared before another reset. Pulse timing and
   the model's ordinary security/light latches (bits 3, 6–7) remain ASSUMED.
   The real bit-3 write restriction (writable only while zero) is unmodelled.
   Bit 4 is read-only watchdog-timeout status, modelled as no timeout (Bochs
   PORTS.LST, port 0092); only bits 2 and 5 are reserved. See machine-config.md
   §A20 line and System Control Port A. CPU masking and its input are wired;
   source defaults retain 8042 enabled and 92h disabled. Attachment of the
   enabled 8042 counts one transition (initial count 1); no source reports a
   disabled unattached board line while CPU addressing stays flat.
   A disabled gate clears **physical address bit 20**, not every high
   address bit and not the linear address before paging. Cover instruction
   fetch, CPU data and implicit page-table accesses, crossing transfers,
   and code-cache invalidation through aliases. Firmware/loader physical
   access and DMA need explicitly documented paths: ISA DMA is a separate
   bus master and must not accidentally go through CPU segmentation/paging.
   SeaBIOS uses **only port 92h** for A20, never 8042 D1h
   (`x86.h:17–19,268–277`, `system.c:18–35`); an open-bus FFh read looks
   enabled without opening the gate (READ). With today's OR rule and the 8042
   source enabled at power-on, SeaBIOS cannot disable effective A20 through 92h
   alone. QEMU uses last-writer-wins in `pc.c`, `handle_a20_line_change`, with
   the comment "XXX: add logic to handle multiple A20 line sources". The
   `generic-386-isa` board bead (`upc-o0j.6`) must choose OR versus
   last-writer-wins and test SeaBIOS INT 15h AX=2400h/2402h, including effective
   disable and query behavior. Keep today's OR default ASSUMED; isolated tests
   of all four source combinations do not establish SeaBIOS compatibility.
   References: IBM PC AT Technical Reference, Keyboard Controller,
   Output Port; IBM Personal System/2 Hardware Interface Technical Reference,
   "System Control Port A (Hex 0092)". This wiring is board policy, not an
   Intel PRM CPU feature. Additional reference: **Ralf
   Brown's Interrupt List, Release 61**, PORT 0064h output-port commands
   and PORT 0092h, table P0415 ([port
   reference](https://github.com/cirosantilli/ralf-brown-interrupt-list/blob/master/inter61d/PORTS.A)).

3. **Firmware INT 15h contract.** READ:
   `vendor/kiselev-bios/README.md:1` identifies the current 8088/Xi8088
   firmware. Its absence of these AT services is an input from the worker
   brief, not a new runtime firmware measurement. Decision-19 selects
   unmodified SeaBIOS `CONFIG_QEMU` (`upc-o0j.9`). Keep BIOS
   instructions running in the guest; do not add host INT 15h shims to the
   CPU adapter. Required service expectations, from **Ralf Brown's Interrupt
   List, Release 61**, INT 15/AH=87h/88h/89h, tables 00498–00501
   ([service reference](https://github.com/cirosantilli/ralf-brown-interrupt-list/blob/master/inter61a/INTERRUP.C)):

   | Service | Contract to qualify |
   | --- | --- |
   | AH=88h | Qualify CF/status and AX against SeaBIOS's highest E820 RAM-entry end (`malloc.c:476–491`), not CMOS. `upc-o0j.6` acceptance is amended accordingly; test zero/configured extended RAM and the E820-derived result |
   | AH=87h | ES:SI describes source/destination through a BIOS GDT work area; CX counts words (up to 8000h); test real copies across 1 MiB, CF/AH success/failure and A20 handling. This is a BIOS operation with protected-mode machinery, not merely a high-memory host memcpy |
   | AH=89h | ES:SI supplies the switch GDT including IDT/data/stack/code descriptors; BL/BH select PIC vector bases in multiples of eight. Success continues after the INT in protected mode with changed segments; test entry state and PIC/IDT consistency, plus A20 failure. It is not itself a DPMI implementation |

   ASSUMED: raw DOS/32A qualification uses its own PE switch; it does not
   require AH=89h merely because it is a DOS extender. Its inspected source
   needs memory discovery and A20. READ: the chosen SeaBIOS **supports
   E801h and E820h** (`system.c:260,290`), as well as AH=87h/89h
   (`system.c:72–165,185`); qualify those discovery paths rather than
   assuming an unsupported-service fallback. AH=88h derives its answer
   from E820 (`malloc.c:476–491`), so CMOS alone is not its oracle.
   Qualify AH=87h/89h separately for the advertised BIOS compatibility
   contract; do not conflate those tests with the first extender milestone.

4. **A real 386 profile and reset choice.** READ:
   `crates/assembly/src/cpu.rs:13` only has an I80286 marker; `assemble.rs:251`
   always constructs Real16; `cpu.rs:47` selects the low `F000:FFF0` reset
   convention. Add a genuine core capability/model choice and a declarative
   profile with suitable RAM, firmware, cascaded PIC, PIT, keyboard/A20 and
   CMOS. [Intel 80386 PRM §10.2.3, First Instructions](https://pdos.csail.mit.edu/6.828/2018/readings/i386/s10_02.htm)
   starts reset fetch at physical `FFFFFFF0h` until the first far transfer.
   Proper reset needs the hidden CS/reset state and board BIOS mapping at
   the high address, or a clearly documented diagnostic-entry profile that
   bypasses hardware reset. `MemoryModel::Flat` would otherwise reserve a
   dense 4 GiB span; SoftMmu is unfinished. The first probe uses direct
   entry; it proves nothing about 386 reset/BIOS boot fidelity. Resolve
   sparse physical backing/high ROM decode with `xj-srh.12 (x86jit)` before
   claiming a reset-accurate 386 profile. `upc-o0j.12` will also supply
   writable C0000h–FFFFFh shadow RAM (128 KiB BIOS preloaded at E0000h) and
   a **separate read-only pristine copy** at FFFE0000h–FFFFFFFFh. This is
   our board design (decision-19 rule 3), unlike QEMU `isapc`'s high alias
   of the same writable BIOS region
   (`x86-common.c:1005–1009,1063,1117–1120`, `pc_sysfw.c:222`). SeaBIOS
   finds no pristine copy on that alias (`shadow.c:178–186`); with
   RAM-preserving CPU resets, a second reboot reaches `HaveRunPost == 3`
   shutdown (`shadow.c:169–208`). POST writes F-segment state and clears
   C0000h (`post.c:290–299`, `optionroms.c:467`); shadow unlock knows only
   i440FX/Q35 (`shadow.c:117–147`). Restrict adapter-RAM validation's
   exception to `generic-386-isa`; keep `ibm-5170` unchanged. `upc-o0j.14`
   will turn typed shutdown (`xj-srh.6 (x86jit)`) and port 92h bit 0 into a
   386 CPU reset retaining RAM. With a pristine copy available, SeaBIOS
   restores shadow RAM from the high image, writes CF9h, then INT3 with a
   one-byte IDT triple-faults (`shadow.c:169–208`, `misc.c:134–145`); CMOS
   0Fh resume codes and the 8042 path must work with the 386 reset state
   (`resume.c:23–84,146–157`).

5. **Protected interrupts through existing INTA.** READ:
   `crates/machine/src/lib.rs:440` already polls readiness, acknowledges the
   PIC and injects one vector, and `crates/cpu/src/x86jit_backend.rs:336`
   delegates readiness. Preserve that seam. The core must use IDTR/gates,
   not the machine decode an IDT itself. Add tests for guest PIC remapping,
   a timer IRQ into USE32 code, IRETD, IF and STI/SS inhibition, pending
   completion blocking and one acknowledgement per accepted request.
   **MEASURED now:** `i386_pm_test_golden.rs` qualifies a remapped PIC IRQ0
   in USE32 at CPL 0 and CPL 3, with EOI, IRETD, exact counts and TSS stack
   frames; see [i386-pm-test.md](i386-pm-test.md). Incomplete-access and
   failed-delivery cases retain their separate acceptance tests.
   A handler must explicitly EOI the actual PIC. Shutdown from failed #DF
   delivery needs a defined typed stop/reset policy, not a host panic.

6. **Deadline engine and 32-bit step accounting.** READ:
   `crates/assembly/src/assemble.rs:238` probes step support even for a
   zero-step run. `crates/machine/src/lib.rs:489` budgets to deadlines and
   charges completed steps, including error prefixes. Core Compat32 cannot
   currently pass this contract (P3). Extend the legacy-system interpreter's
   step API across mode switches, including 32-bit REP iteration boundaries,
   port/MMIO retry handoffs and IRQ readiness. Do not charge JIT blocks as
   instructions or substitute its optional execution counter. Preserve the
   existing distinction: whole retired instructions for diagnostics; completed
   instructions/REP iterations for scheduling; event delivery itself retires
   no instruction. Device time and halted PIT clocks stay under the existing
   deterministic engine. No new machine scheduler is needed.

7. **Snapshot of the new CPU state and replay boundaries.** READ:
   `CpuState` is Clone (`state.rs:295`), but vCPU mode, pending IRQ,
   retirement and shadows live outside it (`vm.rs:1367`, `:1415`, `:1432`).
   `crates/cpu/src/lib.rs:179` has no complete CPU snapshot API;
   `backlog/docs/architecture.md:151` explicitly describes a future machine
   snapshot. Add a versioned CPU image including CR0/2/3, GDTR/IDTR,
   LDTR/TR and their caches, six segment caches/selectors, CPL/EFLAGS and
   execution defaults, plus pending vector, inhibition, completion/retry
   state and counters. Include CPU model and effective A20 input; latch
   sources remain in device state. Page tables are guest RAM. Flush derived
   translations/IR caches on restore instead of serializing host pointers.
   Specify whether snapshots are allowed with an outstanding I/O completion;
   an initial explicit quiescent-boundary restriction is acceptable. Full
   machine replay also needs RAM, devices/PIC, last sampled IRQ levels,
   global/fractional clock, halted/idle state and config/firmware/core identity.
   This spike does not introduce a general disk-persistence framework.

8. **8254 timing read-back (`upc-o0j.10`) — implemented.**
   READ: SeaBIOS's PIT fallback uses read-back command D2h (`timer.c:171–183`).
   `pit8253.rs` now implements count/status latches, status-first reads,
   programmed mode bits, OUT and NULL COUNT, with isolated mock-bus tests.
   Channel 0 is programmed to mode 2, count 0
   (`timer.c:274–283`). CPUID-free 386 timing relies on this path
   (`timer.c:101–116`), not just on the existing 8253 count interface.

9. **Emulator-only fw_cfg and console/ROM delivery (`upc-o0j.13`).** READ:
   no-fw_cfg RAM sizing works, but serial-console configuration comes only
   from `etc/sercon-port` (`sercon.c:519–521`); video/option ROMs come from
   PCI or romfiles, not a plain ISA scan (`optionroms.c:364–386,451–489`).
   Decision-19 adds 510h/511h without DMA only to `generic-386-isa`, serving
   signature/ID, file directory, legacy keys (including 0Eh = 0),
   `etc/sercon-port`, `vgaroms/*`, `genroms/*` and `bootorder` from the
   machine description. Catalogue placement must support firmware-loaded
   ROMs. Qualify the protocol on MockBus and serial-console goldens first;
   register-level VGA plus a SeaVGABIOS entry is later `upc-o0j.15`, and
   diagnostic debug port 402h is optional `upc-o0j.16`. The fixture
   `upc-o0j.9` disables MPTABLE/PIRTABLE and other unused platform features
   as listed in firmware-and-machine-classes.md §3.6.

## Proposed order

1. **Agree the CPU extension contract first.** Keep the pinned dependency
   model and append-only CpuState/jit_abi offsets. Introduce a 386 personality
   and mutable legacy architectural regime without changing the existing
   Long64/Compat32 application semantics. Plan a CPU-aware memory-access path;
   do not try to force protected USE16 through `Code16 == real-mode` checks.
2. **Make unpaged entry and return real.** Implement the needed 386 real
   forms, table registers, descriptor cache/validation, PE switching and
   same-CPL USE16/USE32 transfers. Extend exact step accounting at this stage,
   before enabling the machine profile. Acceptance: P1 without its paging
   suffix writes an expected marker and returns to a real-mode stub.
3. **Deliver protected faults and interrupts at one CPL.** Build IDT/gate/
   IRET handling and fault escalation before claiming restartable paging.
   Keep unsupported-feature exits distinct from architectural #UD.
4. **Add the guest page walker and precise #PF.** Qualify the complete P1
   remap, non-present and permission cases, cross-page fetch/data accesses,
   CR3 reload and physical alias SMC/MMIO. A TLB can wait.
5. **Integrate machine RAM/A20/profile and firmware in parallel as their
   dependencies arrive.** The larger RAM map, CMOS derivation and isolated
   port-92h tests can be prepared earlier. Hardware-reset/high-ROM qualification
   and firmware INT 15h tests have their own gate. Keep 286 goldens pinned.
6. **Qualify an open raw extender/client end to end.** First use a clean
   DOS boot, no EMS/V86 host, integer client, deterministic marker/exit back
   to DOS. Add TSS/ring/IOPL support where the selected client requires it;
   do not represent that choice as general DPMI compatibility. Test timer
   delivery and a real guest paging case as separate explicit obligations.
7. **Round-trip protected CPU state and replay.** Agree state extraction
   earlier, then test at protected and paged boundaries. Defer V86, task-gate
   switches, hardware debug and FPU qualification unless a measured guest
   dependency promotes them into the current scope.

ASSUMED engineering judgment: this is a substantial extension of the mode
and memory architecture, not a small instruction patch. The current design
**cannot host legacy protected mode without redesigning those seams**:
immutable vCPU mode, CPU-independent memory addressing, decode-width-based
segmentation, coupled stack width and user-only privilege semantics are
concrete evidence. There is no evidence that the entire core must be
replaced. An interpreter-first system path can reuse decoding, integer IR,
memory-region routing, SMC tracking and the existing machine traits while
keeping current JIT-generated application code unchanged. Performance of
that future path is unmeasured; this spike makes no schedule or throughput
promise for implementing it.

## x86jit requests

The original numbered list describes the CPU extension seams; the firmware
spike's R1–R3 were sent and filed on 2026-11-11 as noted in entries 2 and 4.
Cross-repository ids below name existing work, not changes filed by this
documentation worker. **Every request** requires isolation tests,
full x86jit gates, append-only existing CpuState/jit_abi layout, unchanged
existing-mode generated code, and a tested seam merged whole before this
repository re-pins. Dependencies refer to this numbered list.

1. **Define 386 legacy-system state and execution-context contract.** Separate
   CPU generation, architectural real/protected regime, CS.D and SS.B from
   fixed application decode contexts; append system/descriptor state. Test
   context derivation and old ABI offsets. Dependency: none; establish the
   CPU-aware translation boundary for request 7 without adding paging yet.
2. **Complete 386 real-mode entry instruction forms.** Add PUSHFD/POPFD,
   PUSHAD/POPAD, FS/GS and 16:32 far forms; correct 67h limit/index behavior
   and CPU identification flags. Test the P2 forms against a 386 contract
   while retaining the 286 personality. Dependency: 1. Baseline entry work
   is `xj-srh.2 (x86jit)`; the firmware spike extends it through two filed
   requests, gated on the 386 model (READ source need, not measured support):
   - **R1, `xj-srh.16 (x86jit)`:** full 386 real-mode integer ISA, 66h/67h
     including SIB and REP with 67h, 32-bit `calll`/`retl`, `pushl`/`pushfl`
     in 16-bit code, and the 15-byte limit. Audit the 386 0Fh map:
     MOVZX/MOVSX, SETcc, Jcc rel16, BT*, BSF/BSR, SHLD/SHRD, two-operand
     IMUL, LSS/LFS/LGS and PUSH/POP FS/GS. SeaBIOS uses gcc
     `-m16 -march=i386` (`Makefile:64,83–84`); the existing Real16 286
     policies leave this incomplete. Proposed oracles: test386.asm
     (`upc-fro.1`) and a SeaBIOS/SeaVGABIOS opcode census. R1 follows
     `xj-srh.4 (x86jit)`, beyond the baseline entry forms needed by entry 4.
   - **R3, `xj-srh.17 (x86jit)`:** EFLAGS.AC/ID fixed zero; CPUID, RDTSC,
     RDMSR/WRMSR, WBINVD/INVD/INVLPG, BSWAP, CMPXCHG and XADD raise #UD
     under the 386 model. SeaBIOS gates CPUID on ID toggling
     (`x86.c:10–23`); without CPUID it skips TSC/MTRR/APIC setup and uses
     PIT timing (`timer.c:101–116`, `mtrr.c:41–44`, `smp.c:104–114`).
3. **Implement descriptor-table and control-register primitives.** Add
   LGDT/SGDT/LIDT/SIDT, SMSW/LMSW, MOV CR0/CR2/CR3 and CLTS with correct
   operand sizes and legal/illegal uses. Test state round trips and rejected
   PG-without-PE writes. Dependencies: 1–2; PE/PG execution effects must land
   with their consuming seams (4/7), not as successful no-ops.
4. **Execute same-CPL protected segmentation and PE transitions.** Add
   descriptor-backed segment loads, limits/types/presence and accessed bits,
   USE16/USE32 far JMP/CALL/RET/IRET, entry and return to real mode. Test
   hidden-cache persistence and DOS/32A-style mixed-width entry. Dependencies:
   1–3 (baseline entry forms); fault generation included, guest IDT routing
   follows in 6. **R2 is added to `xj-srh.4 (x86jit)` acceptance:** on PE
   1→0 the cached segment limit survives; real-mode segment loads change
   only the base and accesses check the cached limit. SeaBIOS returns through
   `transition16big` and runs VGA ROM code in big real mode
   (`romlayout.S:87–117`, `stacks.c:291–292`, READ). PE transitions must
   not flush or relift the other mode's blocks wholesale.
5. **Extend exact step execution across legacy 16/32-bit modes.** Preserve
   counters, 32-bit REP iteration boundaries, fault prefixes, I/O completion
   and STI/SS inhibition across PE/CS changes. Test exact caps and no duplicated
   port/MMIO effects. Dependency: 4; keep the new system path interpreted.
6. **Deliver same-CPL protected IDT events and fault escalation.** Implement
   LIDT-based 16/32-bit interrupt/trap gates, INT/INTR/IRET, architectural
   error codes, #DF and typed shutdown (`xj-srh.6 (x86jit)`); machine reset
   policy is `upc-o0j.14`. Remove the possibility of accidental
   IVT delivery in a legacy protected context. Dependencies: 4–5; test P3/P4
   counterparts and faults during gate/stack delivery.
7. **Implement 386 two-level guest paging and restartable page faults.** Add
   CPU-local CR3 translation for fetch/data/system accesses, P/U/S/R/W and A/D,
   CR2/#PF code, CR3 invalidation and physical alias/MMIO/SMC correctness.
   Dependencies: 1, 3–6; verify P1 remaps to `12345678h` and a handler can
   repair a non-present PTE then retry. No software TLB optimization required.
8. **Implement LDT and descriptor-query instructions.** Add LLDT/SLDT and
   LAR/LSL/VERR/VERW/ARPL, with table/privilege rules and descriptor faults.
   Dependencies: 4, 6; test an LDT-based client segment and DOS/32A's LAR use.
9. **Implement TSS privilege stacks and protected privilege transitions.**
   Add LTR/STR, cached TR/TSS and busy-bit handling, inward interrupt/call-gate
   stacks and outer IRET. Dependencies: 6, 8; test ring 3 to ring 0 and return,
   malformed TSS/stack descriptors and #TS/#SS/#NP. Hardware task switching is
   outside this request.
10. **Enforce legacy CPL/IOPL and TSS port permissions.** Add protected
    CLI/STI/HLT/POPF/IRET policies and scalar/string I/O permission checks
    before exits/side effects. Dependencies: 5, 9; test allowed/denied byte
    and wider ports and 32-bit INS/OUTS iteration accounting.
11. **Expose coherent physical A20 gating to the embedder.** Supply a
    core-facing gate control applied to CPU physical fetch/data/page-walk
    cycles, with split-access and cache/alias coherence; keep 8042/92h board
    policy in the machine. Dependencies: 1, 7; test both real and paged aliases.
12. **Support 386 reset fetch and sparse high-ROM mapping.** Represent
    reset CS hidden state through the first far transfer and offer a bounded
    physical backing/alias strategy for `FFFFFFF0h` (`xj-srh.12 (x86jit)`).
    The board's pristine high image/shadow RAM is `upc-o0j.12`. Dependency: 1;
    test high reset fetch and low-ROM continuation without allocating 4 GiB of RAM.
13. **Export and restore complete legacy CPU checkpoint state.** Include
    architectural caches and vCPU event/shadow/counter state, define completion
    boundary restrictions and flush derived caches on restore. Dependencies:
    5–11 for their final fields; test continuation equivalence across PE/PG
    boundaries. Persistent machine-file format remains the embedder's concern.

The agreed upstream order for the firmware additions is
`xj-srh.4 (x86jit)`, `xj-srh.15 (x86jit)`, then
`xj-srh.16 (x86jit)` / `xj-srh.17 (x86jit)`; the original numbered list
is a seam/dependency description, not a replacement for that tracker order.

Conditional follow-up requests, **not initial-entry blockers**: hardware
task switches/task gates/NT returns (PRM §7.5–§7.6), V86 transitions and
sensitive operations (§15), DR/TF debugging (§12), and 386/387 control/error
integration (§11.1). Promote each separately only when guest evidence or an
explicit compatibility target requires it; do not bundle them into request 4.

## Implementation beads

Proposed machine-side beads only; no tracker was accessed or changed.
All implementation starts after the foundation/consolidation gates required
by the accepted project workflow. Each CPU dependency below means **landed
upstream, pinned here**, not a local patch under the Cargo cache. Firmware
selection is settled by decision-19; its implementation requirements and
existing follow-up beads are in firmware-and-machine-classes.md §3.6.

1. **Add a protected-mode CPU integration fixture and re-pin the core.**
   Dependencies: x86jit requests 1–7 and their upstream gates. Acceptance:
   an in-tree, reproducibly assembled entry fixture performs PE entry,
   protected segment loads, USE32 execution, P1's non-identity page mapping,
   a recoverable #PF and a return to real mode. Assert marker, CR2/error code,
   state and exact completed steps through an appropriate test API. Record
   CPU revision in golden identity; rerun existing slow goldens when the pin
   moves under supervisor authorization. Before evidence: P1/P2/P3 fail these
   requirements on the investigation pin.
   **Partial coverage now:** `i386-pm-test` measures PE/USE32 entry, segments,
   extended RAM, a recoverable #GP and real-mode return at pin `99ed385e`.
   Non-identity paging and recoverable #PF remain blocked on `xj-srh.7 (x86jit)`.
2. **Add extended RAM to the 386 diagnostic configuration.** Dependencies:
   none for the data/model work; 1 for protected execution tests. Acceptance:
   configurable contiguous RAM above 1 MiB, preserved VGA/ROM holes, matching
   CMOS extended KB, no change to the existing 286 preset. An integration
   guest writes/reads extended RAM and a paging alias; host loader and DMA
   retain physical semantics. Before evidence: P5 proves mapping already
   works, while the current BIOS preset still provides no extended region.
3. **Device side complete: port 92h and board-owned A20 source policy.** Dependencies:
   none for isolated latch/device work; existing 8042 remains reused.
   Acceptance: MockBus tests cover bit-1 enable/readback, bit-0 reset request
   semantics, reset values and snapshots. Test all combinations of 8042 and
   fast-A20 sources under the chosen OR policy; controls use declarative
   device/config wiring. No CPU masking is claimed by latch tests alone.
4. **Partially implemented: wire effective A20 into CPU physical accesses.** Dependencies: 2–3,
   x86jit request 11. Real-mode board wiring and alias/reset tests are implemented.
   The paging case remains unimplemented: assembly does not select a paging-capable
   CPU/profile, so the linear-page-to-A20-sensitive-physical-page test still needs
   the later 386 integration. Full acceptance: assembled guest toggles D1h and 92h;
   `FFFF:0030` distinguishes/aliases `100020h` and `20h` correctly; repeat
   with paging mapping a linear page onto an A20-sensitive physical page.
   Test split fetch/read/write, code-alias invalidation, gate changes at
   instruction boundaries and documented DMA/reset behavior. Before evidence:
   P5 reads `77h` after clearing the 8042 latch.
5. **Expose the 386 model and reset profile through assembly.** Dependencies:
   1–4, x86jit request 12, and the decision-19 firmware fixture
   `upc-o0j.9`. Acceptance: validated CPU/model choice rather than an I80286
   label, 16 MiB reference map and expected device roles, reset at the
   correct high ROM address then far transfer to low ROM. Include the
   shadow/high-copy and triple-fault requirements in Machine gap list item 4
   (`upc-o0j.12`, `upc-o0j.14`).
   Missing/mismatched firmware is rejected clearly.
   Diagnostic direct entry remains explicit. Existing 286 reset and goldens
   remain unchanged.
6. **Qualify the selected BIOS INT 15h services.** Dependencies: 4–5 and
   the decision-19 firmware fixture `upc-o0j.9`. Acceptance: guest
   integration tests for AH=88h memory size/error reporting, AH=87h
   descriptor-based high-memory copy and
   AH=89h entry/PIC/segment state; capture CF/AH and A20 outcomes. SeaBIOS
   supports E801h/E820h; qualify their results and compare AH=88h against
   E820, not CMOS (`upc-o0j.6`, amended; Machine gap list item 3). These
   tests execute firmware and produce pinned golden evidence; no adapter
   interrupt shim.
7. **Qualify protected timer IRQs through PIC INTA.** Dependencies: 1,
   x86jit request 6; request 9 if the chosen fixture changes CPL. Acceptance:
   guest remaps PICs, loads IDT, handles a PIT IRQ in USE32 code, EOIs and
   IRETDs. Assert one acknowledgement, correct frame, resume IP, IF/shadows,
   no premature interrupt during incomplete port/MMIO access, and a defined
   shutdown result when delivery fails irrecoverably. Before evidence:
   P3 readiness refuses the mode and P4 injects a real-mode frame.
   **Protected timer IRQ covered:** the new ROM programs both PICs and PIT
   Mode 2, takes exactly three IRQ0s at CPL 0 and one at CPL 3, checks the
   privilege frame, EOIs and resumes. Host delivery records require four
   hardware vector-20h events. This does not close the incomplete-access,
   interrupt-shadow or failed-delivery matrix above.
8. **Extend scheduler equivalence fixtures to mixed and paged 32-bit code.**
   Dependencies: 1, 7, x86jit request 5; request 10 for INS/OUTS fixtures.
   Acceptance: existing Deadline and test-only PerClock engines agree on
   guest registers/RAM/device state, steps, PIT clocks and IRQ order across
   PE/PG changes, 32-bit REP, deferred I/O, HLT and bounded caps. Vary run
   chunk sizes; completed stopping accesses are charged once and faults
   preserve the completed prefix. No wall clock or block-count timing.
   **Partial coverage now:** the unpaged ROM agrees in transcript, exact
   retired count, virtual time and full delivery records between Deadline
   and PerClock, and with Deadline chunks of 1 and 17 and PerClock chunks
   of 257 instructions. Its mode switches, REP table initialization and
   port I/O run through the existing machine loop. Paging, mixed-width
   deferred MMIO/INS/OUTS, HLT wakeups and the full state matrix remain open.
9. **Add protected CPU checkpoint capture and restore qualification.**
   Dependencies: 1, 4, 7–8, x86jit request 13. Acceptance: versioned CPU
   image round-trips hidden caches, CR/table/task state, mode/default sizes,
   flags, A20 and pending/shadow/counter state. At the documented legal
   boundary, snapshot/restore plus the test-owned RAM/device/clock image
   reproduces continuation exactly; changed GDT bytes do not overwrite a
   cached segment during restore. Reject incompatible core/profile versions
   and illegal in-flight completion boundaries atomically. Do not expand
   this bead into frontend snapshot UI or disk persistence.
10. **Run an open DOS extender integer client end to end.** Dependencies:
    5–8, firmware memory-discovery qualification from 6, and x86jit requests
    8–10 if required by the selected raw host/client. Acceptance: reproducible,
    license-documented extender/client fixture, clean DOS boot without an
    EMS/V86 host, measured PE/USE32 entry, extended-RAM operation, timer IRQ,
    deterministic output/exit back to DOS and golden identity pinned to CPU,
    firmware, media and config. Explicitly verify whether this extender path
    enables paging; if not, the separate paged integration fixture from 1
    remains mandatory. List any remaining guest-triggered unsupported
    instructions/faults rather than calling one successful client general
    DPMI/386 compatibility. Full V86/task/FPU qualification is separate work.
