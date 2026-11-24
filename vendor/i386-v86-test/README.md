# i386-v86-test — project-owned V86 monitor ROM

AGPL-3.0-or-later. Assemble offline with `bash vendor/i386-v86-test/build.sh`
inside the development shell. No BIOS, downloaded source or guest OS is needed.
The 128 KiB image has SHA-256
`c21066bfcd0a1d5a39458de7f0c8b656a578d16dd4db97e1672f7755220e348b`.
Build products are ignored; CI builds from the committed assembly source.

Intel 80386 Programmer's Reference Manual, Chapter 15, and Intel486 DX
Programmer's Reference Manual, Chapter 23, define the V86 transitions used.
The ROM enters protected 32-bit ring 0 with a RAM GDT/IDT and 32-bit TSS,
then enters V86 through a nine-dword IRETD frame. Its V86 code uses paragraph
segments and a 16-bit stack. INT 40h takes a DPL-3 386 interrupt gate to the
monitor. The monitor checks that DS/ES/FS/GS were nulled, builds a three-word
real-mode return frame on the interrupted stack and reflects the interrupt
through its IVT entry. The V86 handler returns with IRET. A second monitor
gate lowers IOPL, CLI traps with #GP(0), and IN 80h traps through the denied
TSS bitmap bit. OUT 402h remains allowed through the bitmap at both IOPLs.
The monitor skips the trapped instructions, restores IOPL and returns to V86;
a final INT returns to ring 0 and checks the reflection/fault counts.

The exact port-402h transcript is:

```text
i386-v86-test
v86 real code ok
reflected int ok
v86 resumed ok
iopl gp(0) ok
bitmap allow deny ok
ring0 return ok
failures 0000
```

`crates/pc-harness/tests/v86_golden.rs` runs it on the generic 386 and both
486 models: 867 retired instructions on each. Both scheduling engines and
17/257-unit bounded chunks agree on transcript, count, clock and five delivery
records, including VM=1→0 and CPL=3→0. These are bounded integration witnesses,
not qualification of all V86 monitor services, paging, hardware task gates,
16-bit interrupt gates, VME or 486 alignment checking.

The companion cache-backed FreeDOS/JemmEx tests exercise a real DOS monitor.
This ROM supplies a reproducible clean-CI golden because the core's fixture
recipe does not fetch the frontend's FreeDOS 1.4 source archive. See
[coverage and state audit](../../backlog/docs/design/protected-mode-gaps.md#v86-monitor-and-cpu-state-audit-2026-11-24).
