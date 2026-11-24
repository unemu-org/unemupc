// SPDX-License-Identifier: AGPL-3.0-or-later
//! Machine runners, guest programs and key scripts for the PC/AT goldens and
//! the command line (§3.1).
//!
//! The command line's `--boot`, `--bios` and `run --machine` call the same
//! runners that this crate's golden integration tests call, so a golden pins
//! what the binary does. Every PC-family runner here builds a
//! declarative [`assembly::MachineConfig`] from a Rust preset or machine file, realises it with
//! [`machine_file::assemble`], and drives the resulting machine — addressing devices
//! by role through the assembler's device-index map, never a hardcoded bus index.
//!
//! Runners return their observables (COM1 bytes, POST stream, text screens)
//! through semantic byte routes for COM1 and POST. Text screens use the
//! card-state observation facade. All captures share assembly-owned routing.
//!
//! Front-end tests import the helpers in `test_support` with the
//! `test-support` feature. Core tests keep including `tests/support/` through
//! `#[path]`: `assembly` sits in a lower layer and may not depend on this
//! crate, even for its tests (`backlog/docs/design/core-contract.md` §7.3).

#[cfg(feature = "test-support")]
extern crate self as pc_harness;

#[cfg(feature = "test-support")]
pub mod test_support;

use anyhow::{Context, Result};
use assembly::{
    AssembledMachine, ConsoleProbe, DeviceId, InjectionSchedule, KeyboardStimulus, MachineConfig,
    Observation, ObservationPolicy, ObservationRequest, OutputKind, RunError, RunPolicy,
    StopCondition,
};
use cpu::{Cpu, CpuError, CpuReg, X86JitCpu};
use device::{ByteStreamKind, Device};
use devices::upd765::FLOPPY_IMAGE_BYTES;
use devices::vga_text::{COLS, ROWS, TEXT_BUFFER_LEN};
use machine::{Machine, MachineError, RunOutcome};
use machine_file::assemble_with_observation;
use std::collections::BTreeMap;
use video::{render_text_mode, Frame, TextGrid, TextModeState};

/// Consecutive halted PIT clocks with no interrupt before the boot machine
/// declares the timer quiescent and stops (§5). The boot guest masks IRQ0 after
/// the fifth marker; from then on no serviceable interrupt appears, so the
/// machine gives up after this many idle PIT clocks. Comfortably longer than one PIT
/// period (divisor 20 → 20 PIT clocks per period) so a live
/// timer is never mistaken for a dead one.
pub const BOOT_IDLE_LIMIT: u64 = 200;

/// Linear address of the IRQ0 handler (= its offset, since CS = 0). It is poked
/// into IVT entry 8 by the boot program; it must match the handler's position in
/// [`BOOT_PROGRAM`].
pub const BOOT_HANDLER_OFFSET: u16 = 0x7C30;

/// The built-in real-mode boot demo (§5, §7).
///
/// A hand-encoded 16-bit program (no assembler). The embedder seeds the segment
/// registers and the stack so
/// the program starts straight at device setup: it writes an IRQ0 handler address
/// into IVT entry 8, programs the 8259 PIC (ICW1–ICW4, vector base `0x08`, IRQ0
/// unmasked), programs PIT counter 0 (mode 2, divisor 20), `sti`, then loops on
/// `hlt`. The IRQ0 handler emits a marker byte `.` to COM1, and after five
/// markers masks all IRQs so the machine falls quiescent and terminates. Every
/// instruction's bytes are documented in the `boot_golden` integration test.
#[rustfmt::skip]
pub const BOOT_PROGRAM: &[u8] = &[
    // ---- main program @ 0x7C00 (segments + SP seeded by the embedder) ----
    0xC7, 0x06, 0x20, 0x00, 0x30, 0x7C, // mov word [0x0020],0x7C30  (IVT8 IP)
    0xC7, 0x06, 0x22, 0x00, 0x00, 0x00, // mov word [0x0022],0x0000  (IVT8 CS)
    0xB0, 0x11, 0xE6, 0x20,             // mov al,0x11 ; out 0x20,al  (ICW1)
    0xB0, 0x08, 0xE6, 0x21,             // mov al,0x08 ; out 0x21,al  (ICW2)
    0xB0, 0x04, 0xE6, 0x21,             // mov al,0x04 ; out 0x21,al  (ICW3)
    0xB0, 0x01, 0xE6, 0x21,             // mov al,0x01 ; out 0x21,al  (ICW4)
    0xB0, 0xFE, 0xE6, 0x21,             // mov al,0xFE ; out 0x21,al  (IMR: only IRQ0)
    0xB0, 0x34, 0xE6, 0x43,             // mov al,0x34 ; out 0x43,al  (PIT cmd)
    0xB0, 0x14, 0xE6, 0x40,             // mov al,0x14 ; out 0x40,al  (div lo = 20)
    0xB0, 0x00, 0xE6, 0x40,             // mov al,0x00 ; out 0x40,al  (div hi = 0)
    0xFB,                               // sti
    0xF4,                               // hlt          <- loop target @ 0x7C2D
    0xEB, 0xFD,                         // jmp $-3 (back to hlt)
    // ---- IRQ0 handler @ 0x7C30 ----
    0x50,                               // push ax
    0x52,                               // push dx
    0xFE, 0x06, 0x00, 0x05,             // inc byte [0x0500]        (++counter)
    0xB0, 0x2E,                         // mov al,0x2E ('.')
    0xBA, 0xF8, 0x03,                   // mov dx,0x03F8            (COM1 THR)
    0xEE,                               // out dx,al                (transmit)
    0x80, 0x3E, 0x00, 0x05, 0x05,       // cmp byte [0x0500],0x05
    0x75, 0x04,                         // jne +4                   (skip mask)
    0xB0, 0xFF, 0xE6, 0x21,             // mov al,0xFF ; out 0x21,al (mask all IRQs)
    0xB0, 0x20, 0xE6, 0x20,             // mov al,0x20 ; out 0x20,al (EOI)
    0x5A,                               // pop dx
    0x58,                               // pop ax
    0xCF,                               // iret
];

/// Linear address of the IRQ1 (keyboard) handler in [`KBD_PROGRAM`]. It is poked
/// into IVT entry 9 by the main program (CS = 0, so the linear address equals the
/// offset); it must match the handler's position after the 44-byte main program.
pub const KBD_HANDLER_OFFSET: u16 = 0x7C00 + 44;

/// The built-in real-mode keyboard-interrupt demo (§5, §7).
///
/// A hand-encoded 16-bit program (no assembler). The embedder seeds the segment
/// registers and stack so the
/// program starts straight at device setup: it writes an IRQ1 handler address
/// into IVT entry 9, programs the 8259 PIC (ICW1–ICW4, vector base `0x08`, IRQ1
/// unmasked), enables keyboard interrupts in the 8042 (command byte bit 0),
/// `sti`, then loops on `hlt`. A scancode queued into the keyboard before the run
/// raises IRQ1 once interrupts are enabled; the handler reads it from port `0x60`
/// and emits it to COM1, then masks all IRQs so the machine falls quiescent and
/// terminates. Every instruction's bytes are documented in the `keyboard_golden`
/// integration test.
#[rustfmt::skip]
pub const KBD_PROGRAM: &[u8] = &[
    // ---- main program @ 0x7C00 (segments + SP seeded by the embedder) ----
    0xC7, 0x06, 0x24, 0x00, 0x2C, 0x7C, // mov word [0x0024],0x7C2C  (IVT9 IP)
    0xC7, 0x06, 0x26, 0x00, 0x00, 0x00, // mov word [0x0026],0x0000  (IVT9 CS)
    0xB0, 0x11, 0xE6, 0x20,             // mov al,0x11 ; out 0x20,al  (ICW1)
    0xB0, 0x08, 0xE6, 0x21,             // mov al,0x08 ; out 0x21,al  (ICW2: base 0x08)
    0xB0, 0x04, 0xE6, 0x21,             // mov al,0x04 ; out 0x21,al  (ICW3)
    0xB0, 0x01, 0xE6, 0x21,             // mov al,0x01 ; out 0x21,al  (ICW4)
    0xB0, 0xFD, 0xE6, 0x21,             // mov al,0xFD ; out 0x21,al  (IMR: only IRQ1)
    0xB0, 0x60, 0xE6, 0x64,             // mov al,0x60 ; out 0x64,al  (8042: write cmd byte)
    0xB0, 0x01, 0xE6, 0x60,             // mov al,0x01 ; out 0x60,al  (cmd byte: KBD int on)
    0xFB,                               // sti
    0xF4,                               // hlt          <- loop target @ 0x7C29
    0xEB, 0xFD,                         // jmp $-3 (back to hlt)
    // ---- IRQ1 handler @ 0x7C2C ----
    0x50,                               // push ax
    0x52,                               // push dx
    0xBA, 0x60, 0x00,                   // mov dx,0x0060            (keyboard data port)
    0xEC,                               // in al,dx                 (read scancode)
    0xBA, 0xF8, 0x03,                   // mov dx,0x03F8            (COM1 THR)
    0xEE,                               // out dx,al                (transmit scancode)
    0xB0, 0xFF, 0xE6, 0x21,             // mov al,0xFF ; out 0x21,al (mask all IRQs)
    0xB0, 0x20, 0xE6, 0x20,             // mov al,0x20 ; out 0x20,al (EOI)
    0x5A,                               // pop dx
    0x58,                               // pop ax
    0xCF,                               // iret
];

/// The scancode the built-in keyboard demo queues (make code for `A`). The demo's
/// serial output is exactly this byte, emitted by the IRQ1 handler.
pub const KBD_SCANCODE: u8 = 0x1E;

/// Consecutive halted PIT clocks with no interrupt before the keyboard machine
/// declares the source quiescent and stops (§5). After the single keystroke the
/// handler masks all IRQs, so no further interrupt appears and the machine gives
/// up after this many idle PIT clocks.
pub const KBD_IDLE_LIMIT: u64 = 200;

/// Linear address of the IRQ8 (RTC) handler in [`RTC_PROGRAM`]. It is poked into
/// IVT entry `0x70` by the main program (CS = 0, so the linear address equals the
/// offset); it must match the handler's position after the 64-byte main program.
pub const RTC_HANDLER_OFFSET: u16 = 0x7C00 + 64;

/// The marker byte the RTC demo's IRQ8 handler transmits to COM1 (`'R'`). The
/// demo's serial output is exactly this one byte.
pub const RTC_MARKER: u8 = b'R';

/// Virtual clocks per RTC second for the demo. The real ratio is over a million
/// clocks; a bounded headless run never advances that far, so the RTC is built
/// with this small ratio (the `at_rtc` preset's fast-tick timing) and the
/// update-ended interrupt fires a few dozen PIT clocks into the `hlt` loop. It is
/// still virtual time (including PIT clocks while halted), so the run is
/// deterministic.
pub const RTC_SECOND_TICKS: u64 = 64;
/// Update-in-progress window for the demo RTC, in virtual clocks.
pub const RTC_UIP_TICKS: u64 = 8;

/// Consecutive halted PIT clocks with no interrupt before the RTC machine
/// declares the source quiescent and stops (§5). After the single update-ended
/// interrupt the handler masks the master PIC (the cascade line included), so no
/// further vector is serviceable and the machine gives up after this many idle
/// PIT clocks. Comfortably longer than one RTC second so a live clock is never
/// mistaken for a dead one before it first fires.
pub const RTC_IDLE_LIMIT: u64 = 200;

/// The built-in real-mode RTC update-ended interrupt demo (§5, §7).
///
/// A hand-encoded 16-bit program (no assembler). The embedder seeds the segment
/// registers and stack so the
/// program starts straight at device setup: it writes an IRQ8 handler address
/// into IVT entry `0x70` (`0x0000:0x01C0`), programs the master 8259 (ICW1–ICW4,
/// vector base `0x08`, IRQ2 unmasked for the cascade) and the slave 8259 (ICW1–
/// ICW4, vector base `0x70`, IRQ8 unmasked), enables the RTC's update-ended
/// interrupt (register B bit 4, UIE), `sti`, then loops on `hlt`. Once a full RTC
/// second ticks the RTC asserts IRQ8; the machine forwards its level to the
/// slave PIC, whose INT level drives master IRQ2, and injects vector `0x70`. The
/// handler emits a marker byte `R` to COM1, ends the cascaded interrupt with an
/// EOI to **both** the slave (`0xA0`) and the master (`0x20`), reads register C of
/// the RTC to deassert IRQ8, masks all master IRQs so the source falls quiescent,
/// and `iret`s. Every instruction's bytes are documented in the `rtc_golden`
/// integration test.
#[rustfmt::skip]
pub const RTC_PROGRAM: &[u8] = &[
    // ---- main program @ 0x7C00 (segments + SP seeded by the embedder) ----
    0xC7, 0x06, 0xC0, 0x01, 0x40, 0x7C, // mov word [0x01C0],0x7C40  (IVT0x70 IP)
    0xC7, 0x06, 0xC2, 0x01, 0x00, 0x00, // mov word [0x01C2],0x0000  (IVT0x70 CS)
    0xB0, 0x11, 0xE6, 0x20,             // mov al,0x11 ; out 0x20,al  (master ICW1)
    0xB0, 0x08, 0xE6, 0x21,             // mov al,0x08 ; out 0x21,al  (master ICW2: base 0x08)
    0xB0, 0x04, 0xE6, 0x21,             // mov al,0x04 ; out 0x21,al  (master ICW3: slave on IRQ2)
    0xB0, 0x01, 0xE6, 0x21,             // mov al,0x01 ; out 0x21,al  (master ICW4: 8086)
    0xB0, 0xFB, 0xE6, 0x21,             // mov al,0xFB ; out 0x21,al  (master IMR: unmask IRQ2)
    0xB0, 0x11, 0xE6, 0xA0,             // mov al,0x11 ; out 0xA0,al  (slave ICW1)
    0xB0, 0x70, 0xE6, 0xA1,             // mov al,0x70 ; out 0xA1,al  (slave ICW2: base 0x70)
    0xB0, 0x02, 0xE6, 0xA1,             // mov al,0x02 ; out 0xA1,al  (slave ICW3: identity 2)
    0xB0, 0x01, 0xE6, 0xA1,             // mov al,0x01 ; out 0xA1,al  (slave ICW4: 8086)
    0xB0, 0xFE, 0xE6, 0xA1,             // mov al,0xFE ; out 0xA1,al  (slave IMR: unmask IRQ8)
    0xB0, 0x0B, 0xE6, 0x70,             // mov al,0x0B ; out 0x70,al  (select RTC reg B)
    0xB0, 0x12, 0xE6, 0x71,             // mov al,0x12 ; out 0x71,al  (reg B = 24h | UIE)
    0xFB,                               // sti
    0xF4,                               // hlt          <- loop target @ 0x7C3D
    0xEB, 0xFD,                         // jmp $-3 (back to hlt)
    // ---- IRQ8 handler @ 0x7C40 ----
    0x50,                               // push ax
    0x52,                               // push dx
    0xB0, 0x52,                         // mov al,0x52 ('R')
    0xBA, 0xF8, 0x03,                   // mov dx,0x03F8            (COM1 THR)
    0xEE,                               // out dx,al                (transmit marker)
    0xB0, 0x20, 0xE6, 0xA0,             // mov al,0x20 ; out 0xA0,al (EOI slave)
    0xB0, 0x20, 0xE6, 0x20,             // mov al,0x20 ; out 0x20,al (EOI master)
    0xB0, 0x0C, 0xE6, 0x70,             // mov al,0x0C ; out 0x70,al (select RTC reg C)
    0xBA, 0x71, 0x00,                   // mov dx,0x0071            (RTC data port)
    0xEC,                               // in al,dx                 (read reg C -> deassert IRQ8)
    0xB0, 0xFF, 0xE6, 0x21,             // mov al,0xFF ; out 0x21,al (mask all master IRQs)
    0x5A,                               // pop dx
    0x58,                               // pop ax
    0xCF,                               // iret
];

/// Guest-physical base of the write demo's source buffer: the distinctive sector
/// the guest hands to the DMA controller for the floppy WRITE lives here, preloaded
/// by the preset. Below the read-back buffer and the program, all within 64 KiB.
pub const FDC_WRITE_SRC: u64 = 0x0600;

/// Guest-physical base of the write demo's read-back buffer: the READ DATA that
/// follows the write streams the just-written sector here, where the guest then
/// compares it against the pattern. Starts as cleared RAM (zeroes).
pub const FDC_WRITE_DST: u16 = 0x0A00;

/// The marker byte the write demo emits to COM1 when the sector it read back
/// matches the sector it wrote (`'W'`). The demo's serial output is exactly this
/// one byte; an empty capture means the read-back did not match the write.
pub const FDC_WRITE_MARKER: u8 = b'W';

/// Step cap for the write/readback demo (§6). At the pinned inputs its `W`
/// marker is written at instruction-start step 3,676 (completed at 3,677).
/// 5,000 leaves 1,323 completed steps of margin before stopping the idle loop.
pub const FDC_WRITE_RETIRED_CAP: u64 = 5_000;

/// The distinctive 512-byte sector the write demo commits to the blank floppy.
///
/// Preloaded into guest RAM at [`FDC_WRITE_SRC`] and, after the write→read cycle,
/// expected byte-for-byte in the read-back buffer. Byte `i` is `0xC0 ^ i`, so the
/// guest regenerates this value for every read-back byte. The sector contains
/// non-zero bytes throughout, so a blank floppy cannot spoof the match.
#[must_use]
pub fn fdc_write_pattern() -> Vec<u8> {
    (0..512u32).map(|i| 0xC0u8 ^ i as u8).collect()
}

/// The built-in real-mode floppy WRITE demo (§7, §7.7, §7.8).
///
/// A hand-encoded 16-bit program (no assembler). The embedder seeds the segment
/// registers and stack so the
/// program starts straight at device setup. With a blank writable floppy in drive
/// 0 it: brings the µPD765 out of reset (DOR: not-reset + DMA-enable + motor),
/// selects 250 kbps for the double-density medium, then programs the 8237 channel 2
/// for a memory→device transfer of one 512-byte sector from [`FDC_WRITE_SRC`]
/// and issues WRITE DATA (drive 0, C=0/H=0/R=1). The machine services the DMA
/// on the boundary of the last parameter byte, so the sector is committed and
/// the result phase is ready by the guest's next instruction; the
/// guest drains the seven result bytes. It then programs channel 2 the other way
/// (device→memory) into [`FDC_WRITE_DST`] and issues READ DATA for the same CHS,
/// pulling the just-written sector back. Finally it compares all 512 read-back
/// bytes against the pattern and, on a full match, emits [`FDC_WRITE_MARKER`]
/// to COM1, then busy-spins on a port read (`in al,dx ; jmp $-3`). The port read
/// yields to the machine each iteration, so the run stops cleanly at
/// [`FDC_WRITE_RETIRED_CAP`] (the busy-guest idiom) rather than looping forever
/// before the next CPU step. Because the floppy starts blank, a match proves the
/// write reached media. Every instruction's bytes are documented in the
/// `fdc_write_golden` integration test.
#[rustfmt::skip]
pub const FDC_WRITE_PROGRAM: &[u8] = &[
    // ---- bring the FDC out of reset (DOR @ 0x3F2) ----
    0xB0, 0x1C,                         // mov al,0x1C            (not-reset|DMA|motor0, drive 0)
    0xBA, 0xF2, 0x03,                   // mov dx,0x03F2          (DOR)
    0xEE,                               // out dx,al
    0xB0, 0x02,                         // mov al,0x02            (250 kbps for DD media)
    0xBA, 0xF7, 0x03,                   // mov dx,0x03F7          (CCR)
    0xEE,                               // out dx,al
    // ---- program 8237 ch2: memory -> device (read), src 0x0600, 512 bytes ----
    0xB0, 0x06, 0xE6, 0x0A,             // mov al,0x06 ; out 0x0A,al  (mask ch2)
    0xB0, 0x00, 0xE6, 0x0C,             // mov al,0x00 ; out 0x0C,al  (clear flip-flop)
    0xB0, 0x4A, 0xE6, 0x0B,             // mov al,0x4A ; out 0x0B,al  (mode: ch2 read single)
    0xB0, 0x00, 0xE6, 0x04,             // mov al,0x00 ; out 0x04,al  (addr low)
    0xB0, 0x06, 0xE6, 0x04,             // mov al,0x06 ; out 0x04,al  (addr high -> 0x0600)
    0xB0, 0x00, 0xE6, 0x81,             // mov al,0x00 ; out 0x81,al  (page ch2)
    0xB0, 0xFF, 0xE6, 0x05,             // mov al,0xFF ; out 0x05,al  (count low)
    0xB0, 0x01, 0xE6, 0x05,             // mov al,0x01 ; out 0x05,al  (count high -> 512)
    0xB0, 0x02, 0xE6, 0x0A,             // mov al,0x02 ; out 0x0A,al  (unmask ch2)
    // ---- WRITE DATA (0x05): drive 0, C=0 H=0 R=1 N=2 EOT=1 (DATA @ 0x3F5) ----
    0xBA, 0xF5, 0x03,                   // mov dx,0x03F5          (DATA)
    0xB0, 0x05, 0xEE,                   // mov al,0x05 ; out dx,al  (WRITE DATA)
    0xB0, 0x00, 0xEE,                   // mov al,0x00 ; out dx,al  (drive/head)
    0xB0, 0x00, 0xEE,                   // mov al,0x00 ; out dx,al  (cylinder)
    0xB0, 0x00, 0xEE,                   // mov al,0x00 ; out dx,al  (head)
    0xB0, 0x01, 0xEE,                   // mov al,0x01 ; out dx,al  (sector R=1)
    0xB0, 0x02, 0xEE,                   // mov al,0x02 ; out dx,al  (N=2, 512-byte sector)
    0xB0, 0x01, 0xEE,                   // mov al,0x01 ; out dx,al  (EOT=1)
    0xB0, 0x1B, 0xEE,                   // mov al,0x1B ; out dx,al  (GPL)
    0xB0, 0xFF, 0xEE,                   // mov al,0xFF ; out dx,al  (DTL)
    // ---- drain the 7 WRITE result bytes so the controller returns to idle ----
    0xEC, 0xEC, 0xEC, 0xEC, 0xEC, 0xEC, 0xEC, // in al,dx x7
    // ---- program 8237 ch2: device -> memory (write), dst 0x0A00, 512 bytes ----
    0xB0, 0x06, 0xE6, 0x0A,             // mov al,0x06 ; out 0x0A,al  (mask ch2)
    0xB0, 0x00, 0xE6, 0x0C,             // mov al,0x00 ; out 0x0C,al  (clear flip-flop)
    0xB0, 0x46, 0xE6, 0x0B,             // mov al,0x46 ; out 0x0B,al  (mode: ch2 write single)
    0xB0, 0x00, 0xE6, 0x04,             // mov al,0x00 ; out 0x04,al  (addr low)
    0xB0, 0x0A, 0xE6, 0x04,             // mov al,0x0A ; out 0x04,al  (addr high -> 0x0A00)
    0xB0, 0x00, 0xE6, 0x81,             // mov al,0x00 ; out 0x81,al  (page ch2)
    0xB0, 0xFF, 0xE6, 0x05,             // mov al,0xFF ; out 0x05,al  (count low)
    0xB0, 0x01, 0xE6, 0x05,             // mov al,0x01 ; out 0x05,al  (count high -> 512)
    0xB0, 0x02, 0xE6, 0x0A,             // mov al,0x02 ; out 0x0A,al  (unmask ch2)
    // ---- READ DATA (0x06): same CHS, streams the sector into 0x0A00 ----
    0xBA, 0xF5, 0x03,                   // mov dx,0x03F5          (DATA)
    0xB0, 0x06, 0xEE,                   // mov al,0x06 ; out dx,al  (READ DATA)
    0xB0, 0x00, 0xEE,                   // mov al,0x00 ; out dx,al  (drive/head)
    0xB0, 0x00, 0xEE,                   // mov al,0x00 ; out dx,al  (cylinder)
    0xB0, 0x00, 0xEE,                   // mov al,0x00 ; out dx,al  (head)
    0xB0, 0x01, 0xEE,                   // mov al,0x01 ; out dx,al  (sector R=1)
    0xB0, 0x02, 0xEE,                   // mov al,0x02 ; out dx,al  (N=2)
    0xB0, 0x01, 0xEE,                   // mov al,0x01 ; out dx,al  (EOT=1)
    0xB0, 0x1B, 0xEE,                   // mov al,0x1B ; out dx,al  (GPL)
    0xB0, 0xFF, 0xEE,                   // mov al,0xFF ; out dx,al  (DTL)
    // ---- regenerate the pattern and compare the entire read-back sector ----
    0xBE, 0x00, 0x0A,                   // mov si,0x0A00
    0xB9, 0x00, 0x02,                   // mov cx,512
    0x31, 0xDB,                         // xor bx,bx              (byte index)
    0x8A, 0xC3,                         // mov al,bl              <- compare loop
    0x34, 0xC0,                         // xor al,0xC0
    0x38, 0x04,                         // cmp [si],al
    0x75, 0x0A,                         // jne end (+10)
    0x46,                               // inc si
    0x43,                               // inc bx
    0xE2, 0xF4,                         // loop compare (-12)
    0xB0, 0x57,                         // mov al,0x57 ('W')
    0xBA, 0xF8, 0x03,                   // mov dx,0x03F8          (COM1 THR)
    0xEE,                               // out dx,al               (transmit marker)
    0xEC,                               // in al,dx     (I/O-yielding spin)  <- end
    0xEB, 0xFD,                         // jmp $-3 (back to the in)
];

/// The built-in real-mode VGA text demo (§9.2).
///
/// A hand-encoded 16-bit program (no assembler). The embedder seeds DS/ES to
/// `0xB800` so a near word store lands in the text buffer at `0xB8000`; the demo
/// writes two glyph+attribute cells ("Hi", bright white on black) into cells 0
/// and 1, then `hlt`s. It takes no interrupts. Every instruction's bytes are
/// documented in the `vga_text_golden` integration test.
#[rustfmt::skip]
pub const VGA_TEXT_PROGRAM: &[u8] = &[
    0xB8, b'H', 0x1F,       // mov ax,0x1F48   ('H', attr 0x1F)
    0x89, 0x06, 0x00, 0x00, // mov [0x0000],ax  -> cell 0 @ 0xB8000
    0xB8, b'i', 0x1F,       // mov ax,0x1F69   ('i', attr 0x1F)
    0x89, 0x06, 0x02, 0x00, // mov [0x0002],ax  -> cell 1 @ 0xB8002
    0xF4,                   // hlt
];

/// Step safety cap for the legacy bare test386 rig (§6). It keeps the short
/// scheduler comparison bounded; full-board validation uses a separate budget.
pub const TEST386_MAX_RETIRED: u64 = 1_024;

/// Why the test386 smoke run stopped (§3.1). Captured on **every** exit path so
/// the POST progress stream is never lost to an error return.
///
/// This is a diagnostic classification for the report, not a golden: assertions
/// pin the *variant*, never an address (NASM output shifts stop addresses with
/// the toolchain, so any address lives in `detail` for humans only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// Host output failed; the journal session was left unsealed.
    PersistenceFailed { detail: String },

    /// The guest executed `hlt` and the machine terminated.
    Halt,
    /// The motherboard requested CPU reset; the runner returns control.
    ResetRequested,
    /// The core hit a decodable instruction it does not implement. `detail` records the
    /// faulting address for the report.
    Unsupported {
        /// Human-readable location of the unlifted instruction.
        detail: String,
    },
    /// The guest touched an address or port the minimal machine does not map.
    Unmapped {
        /// Human-readable description of the unmapped access.
        detail: String,
    },
    /// The guest raised a CPU exception the machine escaped to the host.
    Exception {
        /// x86 exception vector.
        vector: u8,
    },
    /// A legacy-system fault with vector, error code and CS:IP, or a run of
    /// exception deliveries that retired no instruction.
    Fault {
        /// Backend fault description without guest bytes.
        detail: String,
    },
    /// Machine configuration or assembly failed before execution.
    ConfigFailed {
        /// Human-readable assembly error.
        detail: String,
    },
    /// The run policy could not grant its complete command budget.
    PolicyFailed {
        /// Human-readable budget error.
        detail: String,
    },
    /// The retired-instruction safety cap was reached before a natural stop.
    BudgetExhausted,
    /// Requested output is incomplete or unavailable; guest execution has stopped.
    ObservationFailed {
        /// Capture diagnostic, including the producing route and byte limit.
        detail: String,
    },
}

/// Finish every drive and report maintenance separately from session errors.
/// See persistent-media.md § Locking and retention.
pub fn finish_persistence(
    assembled: &mut AssembledMachine,
    warnings: &mut impl std::io::Write,
) -> std::result::Result<(), assembly::PersistenceError> {
    let result = assembled.finish_persistence();
    for handle in &assembled.persistence {
        if let Some(outcome) = handle.maintenance_failure() {
            let _ = writeln!(warnings, "warning: {}: {outcome}", handle.owner());
        }
    }
    result
}

/// The result of the test386 smoke run (§3.1): the captured POST progress bytes
/// and why execution stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Test386Run {
    /// Checkpoint bytes the guest wrote to the POST port `0x80`, in order.
    pub post: Vec<u8>,
    /// Why the run ended (for the report; assertions pin only the variant).
    pub stop: StopReason,
}

/// Boot the third-party test386 diagnostic image from RAM and record how far it
/// gets — an exploratory smoke test of the reset → real-mode-execution →
/// POST-capture plumbing, **not** full CPU validation (§3.1, §5).
///
/// The whole 1 MiB is RAM; the 64 KiB `blob` is loaded at `0xF0000` and execution
/// starts at the reset vector `F000:FFF0`, whose far jump enters the diagnostic's
/// POST sequence. The machine decodes just two devices — the POST checkpoint port
/// at `0x80` and a COM1 UART (in case a build variant enables serial) — behind a
/// [`chipset::NoInterruptController`]: the diagnostic runs with interrupts
/// disabled and needs no interrupt source. The step cap bounds execution.
///
/// The POST byte stream is captured through its semantic sink on **every** exit
/// path. A real-mode-only core is expected to stop early on a 386 diagnostic, and
/// that stop surfaces as an `Err` from the machine (an unlifted instruction) — so
/// the error is classified into [`StopReason`] rather than propagated, and the
/// captured checkpoints are drained regardless of how the run ended.
pub fn run_test386(blob: &[u8]) -> Test386Run {
    let config = assembly::test386(blob.to_vec());
    let mut assembled = match assemble_with_observation(&config, &post_policy()) {
        Ok(assembled) => assembled,
        Err(e) => {
            return Test386Run {
                post: Vec::new(),
                stop: StopReason::ConfigFailed {
                    detail: e.to_string(),
                },
            }
        }
    };

    let result = assembled.run_session(&RunPolicy::natural(Some(TEST386_MAX_RETIRED), None));
    let result = result.and_then(|outcome| {
        finish_persistence(&mut assembled, &mut std::io::stderr())?;
        Ok(outcome)
    });
    let rip = assembled.machine.cpu().reg(CpuReg::Rip);
    let mut stop = classify_test386_stop(rip, result);
    let post = stream_or_fail(capture_post(&assembled.observation), &mut stop);
    Test386Run { post, stop }
}

#[cfg(test)]
fn run_past_reset_requests<C: Cpu>(
    machine: &mut Machine<C>,
    cap: Option<u64>,
) -> Result<RunOutcome, MachineError> {
    loop {
        let outcome = machine.run_bounded(cap)?;
        if outcome != RunOutcome::ResetRequested {
            return Ok(outcome);
        }
        machine.reset_cpu()?;
    }
}

/// Classify how a POST-only (no video) run stopped, reading the RIP for the
/// human-readable `detail` (never an assertion target — stop addresses shift with
/// the assembler toolchain).
fn classify_test386_stop(rip: u64, result: Result<RunOutcome, RunError>) -> StopReason {
    match result {
        Ok(RunOutcome::Halted) => StopReason::Halt,
        Ok(RunOutcome::ResetRequested) => StopReason::ResetRequested,
        Ok(RunOutcome::RetiredCapReached) => StopReason::BudgetExhausted,
        Err(RunError::Machine(MachineError::Cpu(CpuError::UnsupportedInstruction {
            addr,
            ..
        }))) => StopReason::Unsupported {
            detail: format!("unsupported instruction at {addr:#x} (rip {rip:#x})"),
        },
        Err(RunError::Machine(MachineError::Cpu(CpuError::Unmapped { addr }))) => {
            StopReason::Unmapped {
                detail: format!("unmapped guest access at {addr:#x} (rip {rip:#x})"),
            }
        }
        Err(RunError::Machine(MachineError::Cpu(
            error @ (CpuError::Fault { .. }
            | CpuError::DeliveryStalled { .. }
            | CpuError::ResetStorm { .. }),
        ))) => StopReason::Fault {
            detail: error.to_string(),
        },
        Err(RunError::Machine(MachineError::Cpu(error @ CpuError::UnsupportedSystem { .. }))) => {
            StopReason::Unsupported {
                detail: error.to_string(),
            }
        }
        Err(RunError::Machine(MachineError::Cpu(CpuError::Memory(msg)))) => StopReason::Unmapped {
            detail: format!("guest memory error: {msg} (rip {rip:#x})"),
        },
        Err(RunError::Machine(MachineError::Cpu(e @ CpuError::UnexpectedExit { .. }))) => {
            StopReason::Unmapped {
                detail: format!("{e} (rip {rip:#x})"),
            }
        }
        Err(RunError::Machine(MachineError::Bus(e))) => StopReason::Unmapped {
            detail: format!("{e} (rip {rip:#x})"),
        },
        Err(error @ RunError::Persistence(_)) => StopReason::PersistenceFailed {
            detail: error.to_string(),
        },
        Err(error @ (RunError::ConsoleUnavailable { .. } | RunError::Audio(_))) => {
            StopReason::ObservationFailed {
                detail: error.to_string(),
            }
        }
        Err(error @ RunError::InsufficientCommandBudget { .. }) => StopReason::PolicyFailed {
            detail: error.to_string(),
        },
        Err(RunError::Machine(MachineError::Exception { vector })) => {
            StopReason::Exception { vector }
        }
    }
}

// ---------------------------------------------------------------------------
// Full BIOS machine: load a real 32 KiB PC BIOS ROM and boot it (§3.1, §5, §9).
// ---------------------------------------------------------------------------

/// Step-equivalent safety cap for BIOS/DOS runs (§6). MEASURED with the pinned
/// AT fixtures at 8/13: the date prompt first appears at 9,461,624 and is observed
/// by the 10,000,000-step poll. The write/DIR command completes at 11,167,482,
/// leaving 8,832,518 steps before this cap. The policy reserves the full
/// three-million-step command window before injecting keys.
pub const BIOS_MAX_RETIRED: u64 = 20_000_000;
/// Step cap for the historical no-controller POST rig (§3.1, §9), at Bare 1/1.
/// MEASURED: final POST OUT starts at 8,024,894 and completes at 8,024,895;
/// "Boot failed" appears at 12,750,391. This cap leaves 7,249,609 steps after
/// that visible failure/key-prompt milestone (11,975,105 after the final POST).
pub const BIOS_GOLDEN_MAX_RETIRED: u64 = 20_000_000;
/// Halted PIT-clock bound. The BIOS timer reload is 65,536 (Intel 8254 datasheet,
/// Mode 3); retaining 1,000,000 permits that full interval plus 934,464 clocks
/// of margin, so timer waits survive POST instead of ending at a short HLT.
pub const BIOS_IDLE_LIMIT: u64 = 1_000_000;

/// The deterministic execution boundary of a captured screen.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ScreenStamp {
    /// Virtual time in instruction equivalents, including halted PIT clocks.
    pub virtual_time: u64,
    /// Instructions retired by the CPU at capture.
    pub retired: u64,
}

/// The result of a full BIOS boot run (§3.1): the POST checkpoint stream, both
/// text windows, the serial stream, final floppy media, and why execution stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BiosRun {
    /// Final drive-0 media read from the controller after execution (§7.7).
    /// `None` means no floppy controller was mounted or media could not be read.
    pub floppy_image: Option<Vec<u8>>,
    /// Checkpoint bytes the BIOS wrote to POST port `0x80`, in order.
    pub post: Vec<u8>,
    /// The colour (CGA/VGA) text page at `0xB8000`, rendered from the card.
    pub screen: TextGrid,
    /// Execution boundary of the colour-card capture; absent if assembly or
    /// observation failed. The MDA card is captured at the same boundary.
    pub screen_stamp: Option<ScreenStamp>,
    /// The monochrome text window at `0xB0000`, rendered from the MDA card.
    /// This is where the Kiselev BIOS's INT 10h console actually writes after its
    /// no-video-BIOS MDA fallback, so a booted guest's console text lands here.
    pub mda: TextGrid,
    /// Bytes the guest transmitted to COM1 (a booted sector's serial output).
    pub com1: Vec<u8>,
    /// Why the run ended (assertions pin only the variant, never an address).
    pub stop: StopReason,
}

/// Assemble the full PC/AT BIOS machine, map the 32 KiB `rom` at `0xF8000`, and
/// boot it from the reset vector through POST and the no-media boot retry loop
/// (§3.1, §5, §9).
///
/// The address space is a full 1 MiB mosaic with no unmapped holes, so a POST
/// probe never aborts on unmapped memory:
///
/// - `[0x00000, 0xA0000)` — 640 KiB conventional RAM (RWX).
/// - `[0xA0000, 0xB0000)` — RAM (the BIOS may probe this graphics-adapter window).
/// - `[0xB0000, 0xB8000)` — the MDA text buffer and mirrors, trapped MMIO.
/// - `[0xB8000, 0xC0000)` — the VGA colour text buffer, trapped MMIO routed to the
///   text card.
/// - `[0xC0000, 0xF8000)` — zeroed RAM standing in for the option-ROM scan area:
///   the BIOS's `0x55AA` signature scan finds nothing and skips it.
/// - `[0xF8000, 0x100000)` — the masked 32 KiB BIOS ROM, loaded with `rom`.
///
/// The cascaded 8259 pair, 8253 PIT, 8042, seeded MC146818 CMOS/RTC, VGA and MDA text
/// cards, port-`0x61` system-control latch, and a POST port at `0x80` are all
/// decoded. The PIT counter-0 OUT is wired to IRQ0, the 8042 to IRQ1, and the RTC
/// update-ended line to IRQ8; the BIOS programs the PICs itself.
///
/// **Port `0x80` decode.** On a real AT the DMA page registers overlap `0x80`;
/// the POST port owns `0x80`. With media attached the 8237 page registers decode
/// `0x81..=0x8F`; without media those ports use the bus's open-bus default.
///
/// The POST stream and the text screen are captured on **every** exit path so
/// callers can inspect the firmware's progress even after an execution error.
pub fn run_bios(rom: &[u8]) -> BiosRun {
    run_bios_with_cap(rom, BIOS_MAX_RETIRED)
}

/// Boot the same PC/AT BIOS machine as [`run_bios`] with a caller-supplied
/// virtual-time cap in instruction equivalents (§3.1, §6, §9).
///
/// The POST golden passes [`BIOS_GOLDEN_MAX_RETIRED`] explicitly to capture
/// complete POST and the no-controller boot-failure/key prompt.
pub fn run_bios_with_cap(rom: &[u8], cap: u64) -> BiosRun {
    let config = assembly::bios(rom.to_vec(), None);
    // The historical POST rig omits DMA hardware. Check its intended BIOS
    // without changing the hardware or cadence pinned by that golden.
    if let Err(error) = assembly::check_board_system_bios(
        &config,
        assembly::BoardModel::Ibm5170,
        config.images[0].base,
        rom,
        assembly::AssemblyOptions::default(),
    ) {
        return bios_config_failure(assembly::AssemblyError::Config(error));
    }
    run_bios_config(config, |_| {
        RunPolicy::natural(Some(cap), Some(BIOS_IDLE_LIMIT))
    })
}

/// The natural-stop policy the plain BIOS/POST boots run under: cap the total
/// virtual time at [`BIOS_MAX_RETIRED`] instruction equivalents, declare the
/// interrupt source quiescent after [`BIOS_IDLE_LIMIT`] idle PIT clocks, and stop
/// when the guest halts without a serviceable interrupt.
fn bios_natural_policy() -> RunPolicy {
    RunPolicy::natural(Some(BIOS_MAX_RETIRED), Some(BIOS_IDLE_LIMIT))
}

/// Select the MDA observation route for FreeCOM prompt detection.
fn mda_console_probe() -> ConsoleProbe {
    ConsoleProbe {
        device: DeviceId::Mda,
    }
}

/// The prompt-poll policy the FreeDOS runs share: step to FreeCOM's date prompt on
/// the MDA console in [`DOS_INTERACTIVE_POLL_STEP`] chunks, then queue `stimuli` and
/// run `then_budget` more so the shell processes them. The plain FreeDOS boot passes
/// no stimuli and a zero budget (it stops at the prompt); the interactive run passes
/// the keyboard script and [`DOS_INTERACTIVE_COMMAND_BUDGET`].
fn freedos_prompt_policy(stimuli: Vec<KeyboardStimulus>, then_budget: u64) -> RunPolicy {
    RunPolicy {
        retired_cap: Some(BIOS_MAX_RETIRED),
        idle_limit: Some(BIOS_IDLE_LIMIT),
        stop: StopCondition::PromptThen {
            console: mda_console_probe(),
            needle: DOS_DATE_PROMPT.to_string(),
            poll_step: DOS_INTERACTIVE_POLL_STEP,
            then_budget,
        },
        stimuli,
    }
}

/// Assemble a BIOS machine from `config`, run it under the policy `build_policy`
/// derives, and capture every observable output path into a [`BiosRun`] (§3.1).
///
/// "How we run" is a [`RunPolicy`], not part of the machine, and `build_policy`
/// builds it *after* assembly so a stimulus can address the keyboard controller by
/// role through the assembler's device-index map (never a hardcoded bus index) —
/// exactly how the interactive FreeDOS run resolves its 8042. The plain BIOS/POST
/// paths hand back a natural-stop policy; the FreeDOS paths a prompt-poll one.
fn run_bios_config(
    config: MachineConfig,
    build_policy: impl FnOnce(&BTreeMap<DeviceId, usize>) -> RunPolicy,
) -> BiosRun {
    let mut assembled = match assemble_with_observation(&config, &bios_observation_policy()) {
        Ok(assembled) => assembled,
        Err(e) => {
            return bios_config_failure(e);
        }
    };

    run_bios_assembled(&mut assembled, build_policy)
}

fn bios_config_failure(error: assembly::AssemblyError) -> BiosRun {
    BiosRun {
        floppy_image: None,
        post: Vec::new(),
        screen: blank_screen(),
        screen_stamp: None,
        mda: blank_screen(),
        com1: Vec::new(),
        stop: StopReason::ConfigFailed {
            detail: error.to_string(),
        },
    }
}

fn bios_file_config(
    path: &std::path::Path,
    catalogue: &[assembly::FirmwareEntry],
) -> Result<MachineConfig> {
    let mut config = assembly::bios(Vec::new(), Some(vec![0; FLOPPY_IMAGE_BYTES]));
    config.images[0].source = assembly::ImageSource::File(path.into());
    machine_file::validate(&config).map_err(assembly::AssemblyError::Config)?;
    let bytes = std::fs::read(path).map_err(|error| {
        assembly::AssemblyError::Config(assembly::ConfigError::ImageRead {
            path: path.into(),
            message: error.to_string(),
        })
    })?;
    let entry = catalogue
        .iter()
        .find(|entry| entry.kind == assembly::FirmwareKind::SystemBios && entry.identifies(&bytes));
    let size = bytes.len() as u64;
    config.display_switch = assembly::bios_display_switch(&bytes);
    config.images[0].source = assembly::ImageSource::Inline(bytes);
    if size > 0 && size.is_multiple_of(8192) && size <= 0x100000 {
        config.images[0].base = 0x100000 - size;
    }
    if entry.is_some() {
        config.images[0].provides.clear();
    }
    Ok(config)
}

/// Load a BIOS file through the IBM 5170 catalogue, emitting run-only warnings.
/// The attached blank disk keeps this on the complete board profile; guest
/// disk writes remain volatile. See `backlog/docs/commands.md` for the flags.
pub fn run_bios_file(
    path: &std::path::Path,
    options: assembly::AssemblyOptions,
    warnings: &mut impl std::io::Write,
) -> Result<BiosRun> {
    let config = bios_file_config(path, assembly::BoardModel::Ibm5170.firmware_catalogue())?;
    let mut assembled = machine_file::assemble_with_observation_and_options(
        &config,
        &bios_observation_policy(),
        options,
    )?;
    for unlisted in &assembled.unlisted_firmware {
        writeln!(warnings, "{unlisted}")?;
    }
    Ok(run_bios_assembled_with_warnings(
        &mut assembled,
        |_| bios_natural_policy(),
        warnings,
    ))
}

fn run_bios_assembled(
    assembled: &mut AssembledMachine,
    build_policy: impl FnOnce(&BTreeMap<DeviceId, usize>) -> RunPolicy,
) -> BiosRun {
    run_bios_assembled_with_warnings(assembled, build_policy, &mut std::io::stderr())
}

fn run_bios_assembled_with_warnings(
    assembled: &mut AssembledMachine,
    build_policy: impl FnOnce(&BTreeMap<DeviceId, usize>) -> RunPolicy,
    warnings: &mut impl std::io::Write,
) -> BiosRun {
    let policy = build_policy(&assembled.device_index);
    let result = assembled.run_session(&policy);
    let result = result.and_then(|outcome| {
        finish_persistence(assembled, warnings)?;
        Ok(outcome)
    });
    let mut stop = classify_bios_stop(&assembled.machine, result);
    let post = stream_or_fail(capture_post(&assembled.observation), &mut stop);
    let (screen, screen_stamp) = match assembled.capture_text(DeviceId::Vga) {
        Ok(capture) => {
            let stamp = ScreenStamp {
                virtual_time: capture.virtual_time,
                retired: capture.retired,
            };
            let grid = match capture.frame {
                Frame::Text(grid) => grid,
                Frame::Pixels(_) => blank_screen(),
            };
            (grid, Some(stamp))
        }
        Err(error) => {
            if !matches!(
                stop,
                StopReason::ObservationFailed { .. } | StopReason::PersistenceFailed { .. }
            ) {
                stop = StopReason::ObservationFailed {
                    detail: format!("capturing display: {error}"),
                };
            }
            (blank_screen(), None)
        }
    };
    let mda = match assembled.capture_text(DeviceId::Mda) {
        Ok(capture) => match capture.frame {
            Frame::Text(grid) => grid,
            Frame::Pixels(_) => blank_screen(),
        },
        Err(error) => {
            if !matches!(
                stop,
                StopReason::ObservationFailed { .. } | StopReason::PersistenceFailed { .. }
            ) {
                stop = StopReason::ObservationFailed {
                    detail: format!("capturing display (MDA): {error}"),
                };
            }
            blank_screen()
        }
    };
    let machine = &mut assembled.machine;
    let com1 = stream_or_fail(capture_serial(&assembled.observation), &mut stop);
    let floppy_image = assembled
        .device_index
        .get(&DeviceId::Fdc)
        .and_then(|&index| machine.bus_mut().device_mut(index))
        .and_then(|fdc| read_bios_floppy_image(fdc.as_mut()));
    BiosRun {
        floppy_image,
        post,
        screen,
        screen_stamp,
        mda,
        com1,
        stop,
    }
}

/// Read the BIOS preset's 1.44 MB drive directly through the controller, bypassing
/// DOS's cache. NEC µPD765 datasheet, READ DATA and Result Phase; see §7.7.
/// Controller reset preserves media and makes reads independent of the phase the
/// guest left behind. No guest instructions or virtual clocks run during capture.
fn read_bios_floppy_image(fdc: &mut dyn Device) -> Option<Vec<u8>> {
    fdc.reset();
    fdc.port_write(0x3F2, 1, 0x1C).ok()?;
    let mut image = Vec::with_capacity(FLOPPY_IMAGE_BYTES);
    for cylinder in 0u8..80 {
        for head in 0..2 {
            for byte in [0x06, head << 2, cylinder, head, 1, 2, 18, 0x1B, 0xFF] {
                fdc.port_write(0x3F5, 1, u32::from(byte)).ok()?;
            }
            let request = fdc.take_dma_request()?;
            let device::DmaOp::ToMemory(bytes) = request.op else {
                return None;
            };
            if bytes.len() != 18 * 512 {
                return None;
            }
            fdc.dma_complete(device::DmaOutcome::ToMemory {
                transferred: bytes.len(),
                terminal_count: true,
            });
            image.extend_from_slice(&bytes);
            for _ in 0..7 {
                fdc.port_read(0x3F5, 1).ok()?;
            }
        }
    }
    Some(image)
}

/// Assemble the full PC/AT BIOS machine as [`run_bios`] does, but with a µPD765
/// floppy controller (ports `0x3F0..=0x3F7`, IRQ6) driven by an 8237 DMA
/// controller (ports `0x00..=0x0F` and the page latches `0x81..=0x8F`) backed by
/// `floppy`, and boot it (§3.1, §5, §7.7).
///
/// This is the path the BIOS's INT 13h / INT 19h use to load a boot sector: the
/// controller reads the sector from `floppy`, the machine moves it into the guest
/// buffer through the 8237 (the SMC-safe `note_write` path), and IRQ6 signals
/// completion. `floppy` is a raw disk image (typically [`FLOPPY_IMAGE_BYTES`]
/// bytes for a 1.44 MB disk); [`bootable_floppy_image`] builds a minimal one.
pub fn run_bios_boot(rom: &[u8], floppy: Vec<u8>) -> BiosRun {
    run_bios_config(assembly::bios(rom.to_vec(), Some(floppy)), |_| {
        bios_natural_policy()
    })
}

/// Boot a real FreeDOS floppy image on the Kiselev BIOS machine and report how
/// far the FreeDOS boot chain gets (§3.1, §5, §7.7).
///
/// This is exactly [`run_bios_boot`] — the same firmware, FDC, and DMA — named
/// for its purpose: `floppy` is a bootable 1.44 MB FreeDOS image (FAT12 BPB, a
/// FreeDOS boot sector, an 8086 `KERNEL.SYS` and `COMMAND.COM` in the root). The
/// BIOS's INT 19h loads sector 0 and jumps to it, the FreeDOS boot sector loads
/// the kernel, and the kernel initialises and hands off to the shell. The
/// returned [`BiosRun`] carries every observable output path so the stop point
/// is captured whatever it is: the POST stream, the colour page ([`BiosRun::screen`]),
/// the MDA console the BIOS actually writes to ([`BiosRun::mda`]), COM1, and the
/// classified [`BiosRun::stop`].
pub fn run_freedos(rom: &[u8], floppy: Vec<u8>) -> BiosRun {
    run_freedos_config(assembly::bios(rom.to_vec(), Some(floppy)))
}

/// Run a file-loaded FreeDOS machine with the same prompt policy and captures.
/// See machine-files § Frontends; hardware and run policy remain separate.
pub fn run_freedos_config(config: MachineConfig) -> BiosRun {
    run_bios_config(config, |_| freedos_prompt_policy(Vec::new(), 0))
}

/// A substring of the FreeCOM startup date prompt, used to detect that the shell
/// has booted and is blocked on keyboard input before the script is injected.
pub const DOS_DATE_PROMPT: &str = "Enter new date";

/// Step interval for prompt polling (§6). MEASURED at AT rate 8/13: the first
/// date-prompt text appears at 9,461,624; one-million-step polling observes it at
/// 10,000,000, a 538,376-step overshoot. Cadence is independent of CPU/PIT ratio.
pub const DOS_INTERACTIVE_POLL_STEP: u64 = 1_000_000;

/// Post-injection step budget (§6). MEASURED with injection at 10,000,000:
/// DIR first completes at 10,945,308 (945,308 steps); write/DIR at 11,167,482
/// (1,167,482 steps). The three-million-step budget leaves 2,054,692 and
/// 1,832,518 steps of margin respectively. Completion includes the full listing
/// and final prompt; the window end at 13,000,000 is only a capture boundary.
/// REP iterations count as steps. The safety cap reserves the entire window.
pub const DOS_INTERACTIVE_COMMAND_BUDGET: u64 = 3_000_000;

/// Boot a real FreeDOS floppy image on the Kiselev BIOS machine and drive the
/// shell interactively with a **keyboard script**, past its startup prompts and
/// into a command (§3.1, §5, §7.5).
///
/// Same firmware, FDC, DMA and instruction-boundary timing as [`run_freedos`],
/// driven in two phases:
///
/// 1. **Boot to the prompt.** Step in [`DOS_INTERACTIVE_POLL_STEP`] chunks until
///    FreeCOM boots and blocks at its date prompt ([`DOS_DATE_PROMPT`] on the MDA
///    console). Keys must wait for DOS to own the keyboard: injected at power-on
///    they are consumed or corrupted by the BIOS keyboard-reset handshake and the
///    POST buffer flush, and the prompt never advances.
/// 2. **Type the script and run the command.** Queue the whole `scancodes` script
///    into the 8042 at once — each a set-1 (XT) make/break byte, exactly what INT
///    09h reads at `0x60` (this machine models no translation) — then run
///    [`DOS_INTERACTIVE_COMMAND_BUDGET`] more. The 8042 delivers queued bytes
///    serially, shifting the next in on a tick after each read, so every scancode
///    drops and re-raises the IRQ1 line on its own and INT 09h gets a clean edge
///    per byte; INT 09h translates each into the BDA keyboard ring where DOS's INT
///    16h reads pop it, then FreeCOM runs the command and prints its output.
///
/// A short script such as `\r\rDIR\r` (accept the date, accept the time, then type
/// `DIR` + Enter) reaches the `A:\>` prompt and a directory listing.
///
/// Fully deterministic — every boundary falls on a retired-instruction count,
/// never wall time (§6) — so the session reproduces byte for byte. The returned
/// [`BiosRun`] carries every observable output path, so the resulting `A:\>`
/// prompt and command output land in the MDA console ([`BiosRun::mda`]).
pub fn run_freedos_interactive(rom: &[u8], floppy: Vec<u8>, scancodes: &[u8]) -> BiosRun {
    run_freedos_interactive_config(assembly::bios(rom.to_vec(), Some(floppy)), scancodes)
}

/// Run keyboard input on a file-loaded FreeDOS machine using the golden policy.
/// See machine-files § Frontends and observation § Deterministic boundaries.
pub fn run_freedos_interactive_config(config: MachineConfig, scancodes: &[u8]) -> BiosRun {
    let script = scancodes.to_vec();
    run_bios_config(config, |device_index| {
        let stimulus = KeyboardStimulus {
            keyboard_index: device_index[&DeviceId::Keyboard],
            scancodes: script,
            schedule: InjectionSchedule::AfterPrompt,
        };
        freedos_prompt_policy(vec![stimulus], DOS_INTERACTIVE_COMMAND_BUDGET)
    })
}

// ---------------------------------------------------------------------------
// Keyboard script: set-1 (XT) make/break scancodes for driving the DOS shell.
// ---------------------------------------------------------------------------
//
// The BIOS INT 09h keyboard table expects set-1 codes and this machine's 8042
// performs no translation, so the exact make/break bytes below are what INT 09h
// reads at port 0x60. Each key press is a make code followed by its break code
// (make | 0x80).

/// `Enter` — set-1 make/break. INT 09h maps it to carriage return (`\r`), which
/// terminates a DOS input line.
pub const KEY_ENTER: [u8; 2] = [0x1C, 0x9C];
/// `D` — set-1 make/break.
pub const KEY_D: [u8; 2] = [0x20, 0xA0];
/// `I` — set-1 make/break.
pub const KEY_I: [u8; 2] = [0x17, 0x97];
/// `R` — set-1 make/break.
pub const KEY_R: [u8; 2] = [0x13, 0x93];

/// The keyboard script that drives FreeCOM from its date prompt to a `DIR`
/// listing: accept the date (Enter), accept the time (Enter), then type `DIR`
/// and run it (Enter). Each key contributes its make **and** break code.
///
/// FreeCOM, started with no `AUTOEXEC.BAT`, prompts for the date and then the
/// time before giving `A:\>`; an empty line (a bare Enter) keeps the current
/// value. So two Enters clear both prompts, and `DIR`+Enter runs the directory
/// listing at the shell.
#[must_use]
pub fn dos_dir_script() -> Vec<u8> {
    let mut script = Vec::new();
    script.extend_from_slice(&KEY_ENTER); // accept the date
    script.extend_from_slice(&KEY_ENTER); // accept the time
    script.extend_from_slice(&KEY_D);
    script.extend_from_slice(&KEY_I);
    script.extend_from_slice(&KEY_R);
    script.extend_from_slice(&KEY_ENTER); // run DIR
    script
}

/// Set-1 (XT) make code for the lowercase letters, digits, space, and `.` that
/// the shell scripts type. The 8042 performs no translation, so these are the
/// exact bytes the BIOS INT 09h handler reads at port `0x60`; INT 09h maps them
/// (with the shift state) to the ASCII FreeCOM's line input sees.
fn set1_make(c: char) -> u8 {
    match c {
        'a' => 0x1E,
        'b' => 0x30,
        'c' => 0x2E,
        'd' => 0x20,
        'e' => 0x12,
        'f' => 0x21,
        'g' => 0x22,
        'h' => 0x23,
        'i' => 0x17,
        'j' => 0x24,
        'k' => 0x25,
        'l' => 0x26,
        'm' => 0x32,
        'n' => 0x31,
        'o' => 0x18,
        'p' => 0x19,
        'q' => 0x10,
        'r' => 0x13,
        's' => 0x1F,
        't' => 0x14,
        'u' => 0x16,
        'v' => 0x2F,
        'w' => 0x11,
        'x' => 0x2D,
        'y' => 0x15,
        'z' => 0x2C,
        ' ' => 0x39,
        '.' => 0x34,
        '0' => 0x0B,
        '1' => 0x02,
        '2' => 0x03,
        '3' => 0x04,
        '4' => 0x05,
        '5' => 0x06,
        '6' => 0x07,
        '7' => 0x08,
        '8' => 0x09,
        '9' => 0x0A,
        _ => panic!("no set-1 scancode for {c:?}"),
    }
}

/// Append `text` to a keyboard `script` as set-1 make/break pairs. `>` is typed
/// as **Shift+`.`** (its US-layout combination): a left-shift make brackets the
/// `.` key and a left-shift break releases it, so INT 09h reads a shifted `.` and
/// yields `>`. Every other character is a plain make-then-break key.
fn type_set1(script: &mut Vec<u8>, text: &str) {
    const LSHIFT_MAKE: u8 = 0x2A;
    const LSHIFT_BREAK: u8 = 0xAA;
    for c in text.chars() {
        if c == '>' {
            let dot = set1_make('.');
            script.extend_from_slice(&[LSHIFT_MAKE, dot, dot | 0x80, LSHIFT_BREAK]);
        } else {
            let m = set1_make(c);
            script.extend_from_slice(&[m, m | 0x80]);
        }
    }
}

/// The keyboard script that drives FreeCOM to **create a file on the floppy** and
/// list it: accept the date (Enter), accept the time (Enter), type `echo x>zz`
/// (redirect one byte into a new file `ZZ`) + Enter, then type `dir` + Enter.
///
/// `echo x>zz` makes FreeCOM open `ZZ` for output and write the line to it, which
/// DOS commits through INT 21h → INT 13h → the µPD765 WRITE DATA path onto the
/// FAT12 media (a directory entry plus one data sector). `dir` then reads the
/// directory back **from disk**, so a run where `ZZ` appears in the listing proves
/// the whole DOS write path reached media — not just that the keys arrived.
#[must_use]
pub fn dos_write_file_script() -> Vec<u8> {
    let mut script = Vec::new();
    script.extend_from_slice(&KEY_ENTER); // accept the date
    script.extend_from_slice(&KEY_ENTER); // accept the time
    type_set1(&mut script, "echo x>zz");
    script.extend_from_slice(&KEY_ENTER); // create the file
    type_set1(&mut script, "dir");
    script.extend_from_slice(&KEY_ENTER); // list the directory from disk
    script
}

/// Classify how the BIOS run stopped into a [`StopReason`], reading back the bytes
/// at the stopping CS:IP so an unlifted/faulting instruction can be disassembled
/// for the report (§3.1). Addresses live in `detail` for humans only.
pub fn classify_bios_stop(
    machine: &Machine<X86JitCpu>,
    result: Result<RunOutcome, RunError>,
) -> StopReason {
    let ip = machine.cpu().reg(CpuReg::Rip);
    let cs = machine.cpu().code_segment();
    let linear = (cs << 4).wrapping_add(ip) & 0xF_FFFF;
    let mut bytes = [0u8; 16];
    let hex = if machine.cpu().read_mem(linear, &mut bytes).is_ok() {
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        "<unreadable>".to_string()
    };
    let site = format!("cs:ip {cs:#06x}:{ip:#06x} (linear {linear:#07x}) bytes [{hex}]");
    match result {
        Ok(RunOutcome::Halted) => StopReason::Halt,
        Ok(RunOutcome::ResetRequested) => StopReason::ResetRequested,
        Ok(RunOutcome::RetiredCapReached) => StopReason::BudgetExhausted,
        Err(RunError::Machine(MachineError::Cpu(CpuError::UnsupportedInstruction {
            addr,
            ..
        }))) => StopReason::Unsupported {
            detail: format!("unsupported instruction at {addr:#x}; {site}"),
        },
        Err(RunError::Machine(MachineError::Cpu(CpuError::Unmapped { addr }))) => {
            StopReason::Unmapped {
                detail: format!("unmapped guest access at {addr:#x}; {site}"),
            }
        }
        Err(RunError::Machine(MachineError::Cpu(
            error @ (CpuError::Fault { .. }
            | CpuError::DeliveryStalled { .. }
            | CpuError::ResetStorm { .. }),
        ))) => StopReason::Fault {
            detail: error.to_string(),
        },
        Err(RunError::Machine(MachineError::Cpu(error @ CpuError::UnsupportedSystem { .. }))) => {
            StopReason::Unsupported {
                detail: error.to_string(),
            }
        }
        Err(RunError::Machine(MachineError::Cpu(CpuError::Memory(msg)))) => StopReason::Unmapped {
            detail: format!("guest memory error: {msg}; {site}"),
        },
        Err(RunError::Machine(MachineError::Cpu(e @ CpuError::UnexpectedExit { .. }))) => {
            StopReason::Unmapped {
                detail: format!("{e}; {site}"),
            }
        }
        Err(RunError::Machine(MachineError::Bus(e))) => StopReason::Unmapped {
            detail: format!("{e}; {site}"),
        },
        Err(error @ RunError::Persistence(_)) => StopReason::PersistenceFailed {
            detail: error.to_string(),
        },
        Err(error @ (RunError::ConsoleUnavailable { .. } | RunError::Audio(_))) => {
            StopReason::ObservationFailed {
                detail: error.to_string(),
            }
        }
        Err(error @ RunError::InsufficientCommandBudget { .. }) => StopReason::PolicyFailed {
            detail: error.to_string(),
        },
        Err(RunError::Machine(MachineError::Exception { vector })) => {
            StopReason::Exception { vector }
        }
    }
}

/// A cleared 80×25 text grid, for a BIOS run that failed before any video output.
fn blank_screen() -> TextGrid {
    let vram = vec![0u8; TEXT_BUFFER_LEN];
    render_text_mode(&TextModeState {
        vram: &vram,
        cols: COLS,
        rows: ROWS,
        start_cell: 0,
        cursor_cell: 0,
        cursor_visible: false,
        attributes: video::TextAttributeSet::Colour {
            blink_enabled: true,
        },
    })
}

// ---------------------------------------------------------------------------
// Fast floppy-boot demo: drive the FDC + 8237 to load a boot sector, no BIOS.
// ---------------------------------------------------------------------------

/// The byte the minimal boot sector transmits to COM1 to prove it executed.
pub const FLOPPY_BOOT_MARKER: u8 = b'B';

/// A hand-encoded real-mode loader (no assembler) that reads sector 0 of the
/// floppy into `0x7C00` and far-jumps to it — a self-contained stand-in for the
/// BIOS INT 13h/INT 19h boot path, so the FDC → 8237 → memory → IRQ6 plumbing is
/// exercised deterministically without BIOS POST. Every byte is
/// documented in the `floppy_boot_golden` integration test.
///
/// It programs 8237 channel 2 for a device→memory transfer to physical `0x07C00`
/// (512 bytes), enables the FDC (DOR: motor 0, DMA, out of reset), issues READ
/// DATA for C0/H0/R1, then `jmp 0x0000:0x7C00`. The machine runs the DMA transfer
/// as the READ command completes, so the sector is in memory before the jump.
#[rustfmt::skip]
pub const FLOPPY_LOADER: &[u8] = &[
    // ---- program 8237 channel 2 (device -> memory, phys 0x07C00, 512 bytes) ----
    0xB0, 0x06, 0xE6, 0x0A, // mov al,0x06 ; out 0x0A,al  (mask channel 2)
    0xB0, 0x00, 0xE6, 0x0C, // mov al,0x00 ; out 0x0C,al  (clear flip-flop)
    0xB0, 0x46, 0xE6, 0x0B, // mov al,0x46 ; out 0x0B,al  (mode: ch2, write, single)
    0xB0, 0x00, 0xE6, 0x04, // mov al,0x00 ; out 0x04,al  (address low)
    0xB0, 0x7C, 0xE6, 0x04, // mov al,0x7C ; out 0x04,al  (address high -> 0x7C00)
    0xB0, 0x00, 0xE6, 0x81, // mov al,0x00 ; out 0x81,al  (channel 2 page = 0)
    0xB0, 0xFF, 0xE6, 0x05, // mov al,0xFF ; out 0x05,al  (count low  = 0x01FF)
    0xB0, 0x01, 0xE6, 0x05, // mov al,0x01 ; out 0x05,al  (count high -> 512 bytes)
    0xB0, 0x02, 0xE6, 0x0A, // mov al,0x02 ; out 0x0A,al  (unmask channel 2)
    // ---- enable the floppy controller ----
    0xBA, 0xF2, 0x03,       // mov dx,0x03F2
    0xB0, 0x1C,             // mov al,0x1C  (motor 0 on, DMA enable, out of reset)
    0xEE,                   // out dx,al
    // ---- issue READ DATA (C0 H0 R1, N2, EOT1) via the data register ----
    0xBA, 0xF5, 0x03,       // mov dx,0x03F5
    0xB0, 0x06, 0xEE,       // mov al,0x06 ; out dx,al  (READ DATA opcode)
    0xB0, 0x00, 0xEE,       // mov al,0x00 ; out dx,al  (drive 0, head 0)
    0xB0, 0x00, 0xEE,       // mov al,0x00 ; out dx,al  (cylinder 0)
    0xB0, 0x00, 0xEE,       // mov al,0x00 ; out dx,al  (head 0)
    0xB0, 0x01, 0xEE,       // mov al,0x01 ; out dx,al  (sector 1)
    0xB0, 0x02, 0xEE,       // mov al,0x02 ; out dx,al  (N = 2 -> 512 bytes)
    0xB0, 0x01, 0xEE,       // mov al,0x01 ; out dx,al  (EOT = 1)
    0xB0, 0x1B, 0xEE,       // mov al,0x1B ; out dx,al  (GPL)
    0xB0, 0xFF, 0xEE,       // mov al,0xFF ; out dx,al  (DTL)
    // ---- hand control to the loaded boot sector ----
    0xEA, 0x00, 0x7C, 0x00, 0x00, // jmp 0x0000:0x7C00
];

/// The minimal boot sector carried at LBA 0 of [`bootable_floppy_image`]: write
/// `'B'` to COM1 and halt.
#[rustfmt::skip]
const FLOPPY_BOOT_SECTOR: &[u8] = &[
    0xBA, 0xF8, 0x03, // mov dx,0x03F8   (COM1 THR)
    0xB0, b'B',       // mov al,'B'
    0xEE,             // out dx,al
    0xF4,             // hlt
];

/// Build a minimal bootable 1.44 MB floppy image: the boot sector at offset 0
/// (writing `'B'` to COM1, then `hlt`) with the `0x55 0xAA` signature at offset
/// 510, and the rest zeroed. The FreeDOS runners use a separately built image; this
/// keeps the boot test self-contained.
#[must_use]
pub fn bootable_floppy_image() -> Vec<u8> {
    let mut image = vec![0u8; FLOPPY_IMAGE_BYTES];
    image[..FLOPPY_BOOT_SECTOR.len()].copy_from_slice(FLOPPY_BOOT_SECTOR);
    image[510] = 0x55;
    image[511] = 0xAA;
    image
}

/// Assemble a minimal real-mode machine — COM1 UART, 8237 DMA, µPD765 floppy
/// controller backed by [`bootable_floppy_image`] — run the [`FLOPPY_LOADER`],
/// and return the bytes the guest sent to COM1 (§7.7).
///
/// The loader drives the controller to read the boot sector into `0x7C00` over
/// DMA and jumps to it; the sector transmits [`FLOPPY_BOOT_MARKER`] and halts, so
/// a successful boot returns exactly that byte. This proves the whole path — FDC
/// command state machine, 8237 transfer into guest memory, and the boot jump —
/// deterministically and fast.
pub fn run_floppy_boot() -> Result<Vec<u8>> {
    let config = assembly::floppy_boot(FLOPPY_LOADER.to_vec(), bootable_floppy_image());
    let mut assembled = assemble_with_observation(&config, &serial_policy())
        .context("assembling floppy-boot machine")?;

    assembled
        .run_session(&RunPolicy::natural(None, None))
        .context("running machine")?;

    finish_persistence(&mut assembled, &mut std::io::stderr())?;
    capture_serial(&assembled.observation)
}

/// Assemble the PC/AT machine, run `blob` in real mode, and return the bytes the
/// guest sent to COM1 (§5, §7).
///
/// The PIT counter-0 OUT line (bus device index `2`) is wired to PIC input IRQ0;
/// the machine forwards both level transitions and acknowledges/injects the
/// timer vector. The PIC detects edges (Intel 8259A datasheet, Edge and Level
/// Triggered Modes).
pub fn run_boot(blob: &[u8]) -> Result<Vec<u8>> {
    let config = assembly::at_boot(blob.to_vec());
    let mut assembled =
        assemble_with_observation(&config, &serial_policy()).context("assembling boot machine")?;

    // No retired cap: the demo masks its timer and halts on its own. The idle
    // limit bounds the wait for an interrupt after that halt.
    assembled
        .run_session(&RunPolicy::natural(None, Some(BOOT_IDLE_LIMIT)))
        .context("running machine")?;

    finish_persistence(&mut assembled, &mut std::io::stderr())?;
    capture_serial(&assembled.observation)
}

/// Assemble the PC/AT machine with `scancodes` queued in the keyboard, run `blob`
/// in real mode, and return the bytes the guest sent to COM1 (§5, §7).
///
/// The 8042's keyboard-byte-ready line (resolved as `DeviceId::Keyboard`) is
/// wired to PIC input IRQ1; the machine forwards both level transitions and
/// acknowledges/injects the keyboard vector once the CPU is ready. A byte read
/// before INTA withdraws the request (Intel 8259A datasheet, Edge and Level
/// Triggered Modes).
pub fn run_keyboard(blob: &[u8], scancodes: &[u8]) -> Result<Vec<u8>> {
    let config = assembly::at_keyboard(blob.to_vec());
    let mut assembled = assemble_with_observation(&config, &serial_policy())
        .context("assembling keyboard machine")?;

    // Queue the scancodes into the 8042 before the run, exactly as the legacy
    // chipset seeded them at construction — the guest raises IRQ1 for each once it
    // enables keyboard interrupts.
    assembly::inject_scancodes(
        &mut assembled.machine,
        assembled.device_index[&DeviceId::Keyboard],
        scancodes,
    );

    // The idle limit lets the demo stop once no interrupt can wake the CPU.
    assembled
        .run_session(&RunPolicy::natural(None, Some(KBD_IDLE_LIMIT)))
        .context("running machine")?;

    finish_persistence(&mut assembled, &mut std::io::stderr())?;
    capture_serial(&assembled.observation)
}

/// Assemble the PC/AT machine with a fast-ticking RTC, run `blob` in real mode,
/// and return the bytes the guest sent to COM1 (§5, §7).
///
/// The RTC's update-ended output (`DeviceId::Rtc`) is wired to PIC input
/// IRQ8 on the slave 8259; the machine forwards both output transitions to it,
/// and slave INT drives master IRQ2 as a level (Intel 8259A datasheet, Cascading
/// the 8259A and Edge and Level Triggered Modes).
/// This is the first path where an interrupt reaches the CPU across the cascade.
/// The RTC is built with a small per-second tick ratio ([`RTC_SECOND_TICKS`]) so
/// the update-ended interrupt fires within a bounded run; the timing is virtual,
/// so the output stays deterministic.
pub fn run_rtc(blob: &[u8]) -> Result<Vec<u8>> {
    let config = assembly::at_rtc(blob.to_vec(), RTC_SECOND_TICKS, RTC_UIP_TICKS);
    let mut assembled =
        assemble_with_observation(&config, &serial_policy()).context("assembling rtc machine")?;

    // The idle limit lets the demo stop once no interrupt can wake the CPU.
    assembled
        .run_session(&RunPolicy::natural(None, Some(RTC_IDLE_LIMIT)))
        .context("running machine")?;

    finish_persistence(&mut assembled, &mut std::io::stderr())?;
    capture_serial(&assembled.observation)
}

/// Assemble the floppy-WRITE machine, run `program` in real mode, and return the
/// captured COM1 bytes (§7.7, §7.8).
///
/// The machine mounts a blank writable floppy; `program` drives the µPD765 and
/// 8237 to write a preloaded sector to media and read it back, emitting
/// [`FDC_WRITE_MARKER`] on a match. Nothing runs a wall clock, so the capture is
/// deterministic: a successful write→read cycle yields exactly `b"W"`.
pub fn run_fdc_write(program: &[u8]) -> Result<Vec<u8>> {
    let config = assembly::at_fdc_write(program.to_vec(), fdc_write_pattern(), FDC_WRITE_SRC);
    let mut assembled = assemble_with_observation(&config, &serial_policy())
        .context("assembling fdc-write machine")?;

    assembled
        .run_session(&RunPolicy::natural(Some(FDC_WRITE_RETIRED_CAP), None))
        .context("running machine")?;

    finish_persistence(&mut assembled, &mut std::io::stderr())?;
    capture_serial(&assembled.observation)
}

/// Assemble the PC/AT machine, run `blob` in real mode, then render the VGA text
/// card and return the [`TextGrid`] received by its configured
/// [`video::CaptureFrameSink`] (§9.2). Missing sink delivery is an explicit error.
///
/// DS/ES are seeded to `0xB800` so the guest's near stores reach the
/// text buffer, and the `0xB8000` window is mapped as trapped MMIO so those
/// stores surface on the bus and reach the card. Nothing runs a wall clock, so
/// the produced grid is deterministic.
pub fn run_vga_text(blob: &[u8]) -> Result<TextGrid> {
    let config = assembly::at_vga_text(blob.to_vec());
    let mut observation = display_policy();
    observation.output.frame_sink = assembly::FrameSinkKind::Capture;
    let mut assembled =
        assemble_with_observation(&config, &observation).context("assembling vga-text machine")?;

    let result = assembled.run_session(&RunPolicy::natural(None, None));
    let capture = assembled.capture_text(DeviceId::Vga);
    result.context("running machine")?;
    finish_persistence(&mut assembled, &mut std::io::stderr())?;
    capture.context("capturing vga-text display")?;
    let Frame::Text(grid) = assembled
        .observation
        .captured_frame(DeviceId::Vga)
        .context("VGA capture sink received no frame")?
    else {
        anyhow::bail!("VGA capture sink received a graphics frame");
    };
    Ok(grid)
}

fn display_policy() -> ObservationPolicy {
    ObservationPolicy {
        trace: device::TracePolicy::Off,
        routes: vec![ObservationRequest {
            device: DeviceId::Vga,
            output: OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    }
}

/// Each requested stream accepts at most 1 MiB across the whole run, including
/// drained batches. This frontend bound leaves ample room for boot diagnostics
/// while making runaway guest output fail capture; it does not cap guest execution.
const STREAM_BYTE_LIMIT: usize = 1024 * 1024;

fn byte_policy(device: DeviceId, kind: ByteStreamKind) -> ObservationPolicy {
    ObservationPolicy {
        trace: device::TracePolicy::Off,
        routes: vec![ObservationRequest {
            device,
            output: OutputKind::Bytes(kind),
            byte_limit: STREAM_BYTE_LIMIT,
        }],
        ..Default::default()
    }
}

fn serial_policy() -> ObservationPolicy {
    byte_policy(DeviceId::Uart, ByteStreamKind::SerialTx)
}

fn post_policy() -> ObservationPolicy {
    byte_policy(DeviceId::Post, ByteStreamKind::PostCheckpoints)
}

fn bios_observation_policy() -> ObservationPolicy {
    let mut policy = display_policy();
    policy.routes.push(ObservationRequest {
        device: DeviceId::Mda,
        output: OutputKind::TextScanout,
        byte_limit: 0,
    });
    policy.routes.extend(serial_policy().routes);
    policy.routes.extend(post_policy().routes);
    policy
}

fn stream_or_fail(result: Result<Vec<u8>>, stop: &mut StopReason) -> Vec<u8> {
    match result {
        Ok(bytes) => bytes,
        Err(error) => {
            if !matches!(
                stop,
                StopReason::ObservationFailed { .. } | StopReason::PersistenceFailed { .. }
            ) {
                *stop = StopReason::ObservationFailed {
                    detail: format!("{error:#}"),
                };
            }
            Vec::new()
        }
    }
}

fn capture_serial(observation: &Observation) -> Result<Vec<u8>> {
    observation
        .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
        .context("capturing COM1")
}

fn capture_post(observation: &Observation) -> Result<Vec<u8>> {
    observation
        .drain_bytes(DeviceId::Post, ByteStreamKind::PostCheckpoints)
        .context("capturing POST checkpoints")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_restarts_at_the_cpu_reset_vector() {
        let mut rom = vec![0; 0x8000];
        rom[..5].copy_from_slice(&[0xB0, 0xFE, 0xE6, 0x64, 0xF4]);
        rom[0x7FF0..0x7FF8].copy_from_slice(&[0xBA, 0xF8, 3, 0xB0, b'Z', 0xEE, 0xFA, 0xF4]);
        let mut config = assembly::bios(rom, None);
        config.cpu.reset = assembly::ResetSpec::Entry { cs: 0xF800, ip: 0 };
        let result = run_bios_config(config.clone(), |_| RunPolicy::natural(Some(100), None));
        assert_eq!(result.stop, StopReason::Halt);
        assert_eq!(result.com1, b"Z");
        for cap in [None, Some(100)] {
            let mut assembled = assemble_with_observation(&config, &serial_policy()).unwrap();
            assert_eq!(
                run_past_reset_requests(&mut assembled.machine, cap).unwrap(),
                RunOutcome::Halted
            );
            assert_eq!(capture_serial(&assembled.observation).unwrap(), b"Z");
        }
    }

    #[test]
    fn cli_finishes_every_drive_and_warns_without_failing_the_run() {
        for operation in ["temporary write", "publication directory sync"] {
            for both_fail in [false, true] {
                let temp = machine_file::test_support::TempDir::new("cli-maintenance");
                let base = temp.path().join("base.img");
                std::fs::write(&base, vec![0; 1_474_560]).unwrap();
                let mut config = assembly::bios(vec![0xF4; 32768], Some(vec![0; 1_474_560]));
                config.board = assembly::BoardModel::Bare;
                let demo = assembly::at_boot(vec![0xFA, 0xF4]);
                config.cpu = demo.cpu;
                config.images = demo.images;
                config.drives[0].image = assembly::ImageSource::File(base);
                config.drives[0].write = assembly::WriteMode::Journal {
                    path: temp.path().join("first.upfj"),
                    keep_generations: 1,
                };
                let mut second = config.drives[0].clone();
                second.unit = 1;
                second.write = assembly::WriteMode::Journal {
                    path: temp.path().join("second.upfj"),
                    keep_generations: if both_fail { 1 } else { 10 },
                };
                config.drives.push(second);
                let mut assembled =
                    assemble_with_observation(&config, &bios_observation_policy()).unwrap();
                for handle in &mut assembled.persistence {
                    handle.checkpoint(&mut assembled.machine).unwrap();
                }
                assembled.persistence[0].fail_retention(operation);
                if both_fail {
                    assembled.persistence[1].fail_retention("temporary write");
                }
                let mut warnings = Vec::new();
                let run = run_bios_assembled_with_warnings(
                    &mut assembled,
                    |_| RunPolicy::natural(Some(100), None),
                    &mut warnings,
                );
                assert_eq!(run.stop, StopReason::Halt);
                let expected = if operation == "temporary write" {
                    "warning: drive 0: retention maintenance failed before publication: injected retention write failure\n"
                } else {
                    "warning: drive 0: retention maintenance completion uncertain; reopen required: injected retention barrier failure\n"
                };
                let expected = if both_fail {
                    format!("{expected}warning: drive 1: retention maintenance failed before publication: injected retention write failure\n")
                } else {
                    expected.to_owned()
                };
                assert_eq!(String::from_utf8(warnings).unwrap(), expected);
                let base = std::fs::read(temp.path().join("base.img")).unwrap();
                let scan = journal_files::scan(
                    &std::fs::read(temp.path().join("second.upfj")).unwrap(),
                    &base,
                    base.len() as u64,
                );
                assert_eq!(scan.report.status, journal_files::Status::Valid);
                assert_eq!(
                    scan.report.generations.last().unwrap().reason,
                    journal_files::ClosingReason::SessionEnd
                );
                assert_eq!(scan.report.generations.last().unwrap().id, 2);
            }
        }
    }

    fn hd_fdc_write_program() -> Vec<u8> {
        let mut program = FDC_WRITE_PROGRAM.to_vec();
        // These journal fixtures retain their HD images and need 500 kbps.
        assert_eq!(&program[6..12], &[0xB0, 2, 0xBA, 0xF7, 3, 0xEE]);
        program[7] = 0;
        program
    }

    #[test]
    fn journal_host_failures_are_reported_as_persistence_failures() {
        let temp = machine_file::test_support::TempDir::new("cli-host-failure");
        let base = temp.path().join("base.img");
        let journal = temp.path().join("disk.upfj");
        std::fs::write(&base, vec![0; 1_474_560]).unwrap();
        let demo =
            assembly::at_fdc_write(hd_fdc_write_program(), fdc_write_pattern(), FDC_WRITE_SRC);
        let mut config = assembly::bios(vec![0xF4; 32768], Some(vec![0; 1_474_560]));
        config.board = assembly::BoardModel::Bare;
        config.cpu = demo.cpu;
        config.images = demo.images;
        config.drives[0].image = assembly::ImageSource::File(base);
        config.drives[0].write = assembly::WriteMode::journal(&journal);
        let mut assembled = assemble_with_observation(&config, &bios_observation_policy()).unwrap();
        assembled.persistence[0].fail_output();
        let result = run_bios_assembled(&mut assembled, |_| RunPolicy::natural(Some(10_000), None));
        assert!(
            matches!(result.stop, StopReason::PersistenceFailed { ref detail } if detail.contains("drive 0 persistence")),
            "{:?}",
            result.stop
        );
        let result = assembled.run_session(&RunPolicy::natural(Some(20_000), None));
        let rip = assembled.machine.cpu().reg(CpuReg::Rip);
        assert!(matches!(
            classify_test386_stop(rip, result),
            StopReason::PersistenceFailed { .. }
        ));
        let bytes = std::fs::read(journal).unwrap();
        assert_eq!(bytes.len(), 128, "failed output must not seal the session");
    }

    #[test]
    fn journal_frontend_finishes_successful_caps_but_leaves_execution_errors_unsealed() {
        for fail in [false, true] {
            let temp = machine_file::test_support::TempDir::new("journal-test");
            let path = temp.path();
            let base = path.join("base.img");
            let journal = path.join("disk.upfj");
            let mut program = hd_fdc_write_program();
            if fail {
                program.truncate(program.len() - 3);
                program.extend([0x0F, 0x05]);
            }
            let demo = assembly::at_fdc_write(program, fdc_write_pattern(), FDC_WRITE_SRC);
            let mut config = assembly::bios(vec![0xF4; 32768], Some(vec![0; 1_474_560]));
            config.board = assembly::BoardModel::Bare;
            config.cpu = demo.cpu;
            config.images = demo.images;
            // Keep this host-error fixture on flat Application(Real16); an
            // A20-capable 80286 rig delivers processor exceptions in the guest.
            config.devices.retain(|device| {
                !matches!(
                    device.kind,
                    assembly::DeviceKind::Keyboard(_) | assembly::DeviceKind::SysCtrlPortA
                )
            });

            std::fs::write(&base, vec![0; 1_474_560]).unwrap();
            config.drives[0].image = assembly::ImageSource::File(base.clone());
            config.drives[0].write = assembly::WriteMode::journal(&journal);
            let run = run_bios_config(config, |_| RunPolicy::natural(Some(10_000), None));
            if fail {
                assert!(
                    matches!(run.stop, StopReason::Unsupported { .. }),
                    "{:?}",
                    run.stop
                );
            } else {
                assert_eq!(run.stop, StopReason::BudgetExhausted);
            }
            let bytes = std::fs::read(&journal).unwrap();
            assert!(bytes.len() > 128);
            let kinds: Vec<_> = bytes[128..]
                .as_chunks::<640>()
                .0
                .iter()
                .map(|frame| frame[8])
                .collect();
            assert!(kinds.contains(&2), "guest command must be durable");
            assert_eq!(kinds.last() == Some(&3), !fail);
            assert_eq!(std::fs::read(base).unwrap(), vec![0; 1_474_560]);
        }
    }

    #[test]
    fn display_policy_selects_sinks_without_changing_the_vga_frame() {
        let config = assembly::at_vga_text(VGA_TEXT_PROGRAM.to_vec());
        let mut reference = assemble_with_observation(&config, &display_policy()).unwrap();
        reference
            .run_session(&RunPolicy::natural(None, None))
            .unwrap();
        let expected = reference.capture_text(DeviceId::Vga).unwrap().frame;
        for kind in [
            assembly::FrameSinkKind::Headless,
            assembly::FrameSinkKind::Capture,
        ] {
            let mut policy = display_policy();
            policy.output.frame_sink = kind;
            let mut assembled = assemble_with_observation(&config, &policy).unwrap();
            let count = usize::from(kind == assembly::FrameSinkKind::Capture);
            assert_eq!(assembled.observation.subscriber_count(DeviceId::Vga), count);
            assert_eq!(assembled.observation.captured_frame(DeviceId::Vga), None);
            assembled
                .run_session(&RunPolicy::natural(None, None))
                .unwrap();
            let frame = assembled.capture_text(DeviceId::Vga).unwrap().frame;
            assert_eq!(
                assembled.observation.captured_frame(DeviceId::Vga),
                (count == 1).then(|| frame.clone())
            );
            assert_eq!(frame, expected);
            assert_eq!(assembled.observation.subscriber_count(DeviceId::Vga), count);
        }
        assert_eq!(
            Frame::Text(run_vga_text(VGA_TEXT_PROGRAM).unwrap()),
            expected
        );
    }

    #[test]
    fn inline_host_can_select_capture_output() {
        let mut devices: Vec<Box<dyn Device>> = vec![Box::new(devices::VgaText::new())];
        devices[0].mmio_write(0xB8000, 2, 0x7049).unwrap();
        let device_index = BTreeMap::from([(DeviceId::Vga, 0)]);
        let mut observation =
            Observation::bind(&display_policy(), &device_index, &mut devices).unwrap();
        observation.configure_output(assembly::OutputConfig {
            frame_sink: assembly::FrameSinkKind::Capture,
            ..Default::default()
        });
        let cpu =
            X86JitCpu::new_real16_bare(0x10000, 0, 0, cpu::x86jit_backend::RealSegments::flat(0))
                .unwrap();
        let mut assembled = AssembledMachine {
            floppy_names: Default::default(),
            a20_line: Default::default(),
            machine: Machine::new(
                cpu,
                bus::Bus::new(devices, bus::DecodeMap::default()),
                Box::new(chipset::NoInterruptController),
                Vec::new(),
                None,
            ),
            observation,
            device_index,
            mouse_target: None,
            persistence: Vec::new(),
            unlisted_firmware: Vec::new(),
            option_roms: Vec::new(),
            mount: Default::default(),
        };
        assert_eq!(assembled.observation.subscriber_count(DeviceId::Vga), 1);
        let frame = assembled.capture_text(DeviceId::Vga).unwrap().frame;
        let Frame::Text(grid) = &frame else {
            panic!("expected text frame")
        };
        assert_eq!(grid.cell(0, 0).unwrap().glyph, b'I');
        assert_eq!(
            assembled.observation.captured_frame(DeviceId::Vga),
            Some(frame.clone())
        );
        assembled
            .observation
            .configure_output(assembly::OutputConfig {
                frame_sink: assembly::FrameSinkKind::Headless,
                ..Default::default()
            });
        assert_eq!(assembled.observation.subscriber_count(DeviceId::Vga), 0);
        assert_eq!(assembled.observation.captured_frame(DeviceId::Vga), None);
        assert_eq!(
            assembled.capture_text(DeviceId::Vga).unwrap().frame,
            Frame::Text(grid.clone())
        );
        assert_eq!(assembled.observation.captured_frame(DeviceId::Vga), None);
    }

    #[test]
    fn bios_headless_and_capture_share_ordered_prompt_and_final_frames() {
        use std::{cell::RefCell, rc::Rc};
        use video::FrameSink;

        struct Recorder(Rc<RefCell<Vec<Frame>>>);
        impl FrameSink for Recorder {
            fn present(&mut self, frame: &Frame) {
                self.0.borrow_mut().push(frame.clone());
            }
        }

        for tail in [&[0xFA, 0xF4][..], &[0x0F, 0x05], &[0xEB, 0xFE]] {
            let error_path = tail == [0x0F, 0x05];
            let mut baseline = None;
            for (kind, record) in [
                (assembly::FrameSinkKind::Headless, true),
                (assembly::FrameSinkKind::Capture, true),
                (assembly::FrameSinkKind::Headless, false),
            ] {
                let mut rom = vec![0; 32768];
                let mut program = vec![
                    0xB8, 0, 0xB0, 0x8E, 0xD8, // MDA data segment
                    0xB8, b'A', 7, 0xA3, 0, 0, 0xB8, b'B', 0x70, 0xA3, 2, 0, 0xBA, 0xF8, 3, 0xB0,
                    b'Z', 0xEE, 0xB0, 0xFF, 0xE6, 0x80, 0xB8, b'C', 0x0F, 0xA3, 0, 0,
                ];
                program.extend_from_slice(tail);
                rom[..program.len()].copy_from_slice(&program);
                // Start at the ROM body without spending a poll on a reset jump.
                let mut config = assembly::bios(rom, None);
                config.cpu.reset = assembly::ResetSpec::Entry { cs: 0xF800, ip: 0 };
                config.board = assembly::BoardModel::Bare;
                if error_path {
                    // Only this case needs a host decode error; keep keyboard
                    // injection on the halt and budget cases below.
                    config.devices.retain(|device| {
                        !matches!(
                            device.kind,
                            assembly::DeviceKind::Keyboard(_) | assembly::DeviceKind::SysCtrlPortA
                        )
                    });
                }
                config.devices.reverse();
                let mut observation = bios_observation_policy();
                observation.output.frame_sink = kind;
                let mut assembled = assemble_with_observation(&config, &observation).unwrap();
                let capture_count = usize::from(kind == assembly::FrameSinkKind::Capture);
                for device in [DeviceId::Mda, DeviceId::Vga] {
                    assert_eq!(
                        assembled.observation.subscriber_count(device),
                        capture_count
                    );
                    assert_eq!(assembled.observation.captured_frame(device), None);
                }
                let frames = Rc::new(RefCell::new(Vec::new()));
                if record {
                    assembled
                        .observation
                        .subscribe(DeviceId::Mda, Box::new(Recorder(frames.clone())))
                        .unwrap();
                }
                assert_eq!(
                    assembled.observation.subscriber_count(DeviceId::Mda),
                    capture_count + usize::from(record)
                );
                assert_eq!(
                    assembled.observation.subscriber_count(DeviceId::Vga),
                    capture_count
                );
                let mut policy = RunPolicy {
                    retired_cap: Some(100),
                    idle_limit: None,
                    stop: StopCondition::PromptThen {
                        console: ConsoleProbe {
                            device: DeviceId::Mda,
                        },
                        needle: "AB".into(),
                        poll_step: 4,
                        then_budget: 20,
                    },
                    stimuli: if error_path {
                        Vec::new()
                    } else {
                        vec![KeyboardStimulus {
                            keyboard_index: assembled.device_index[&DeviceId::Keyboard],
                            scancodes: vec![0x1C],
                            schedule: InjectionSchedule::AfterPrompt,
                        }]
                    },
                };
                if error_path {
                    // Without keyboard stimuli, continue explicitly beyond the
                    // prompt to retain the same prompt/final capture sequence.
                    assert_eq!(
                        assembled.run_session(&policy).unwrap(),
                        machine::RunOutcome::RetiredCapReached
                    );
                    policy.stop = StopCondition::Natural;
                    policy.retired_cap = Some(assembled.machine.virtual_time() + 20);
                }
                let run = run_bios_assembled(&mut assembled, |_| policy);
                assert_eq!(run.com1, b"Z");
                assert_eq!(run.post, [0xFF]);
                match tail {
                    [0xFA, 0xF4] => assert_eq!(run.stop, StopReason::Halt),
                    [0x0F, 0x05] => assert!(matches!(run.stop, StopReason::Unsupported { .. })),
                    _ => assert!(matches!(run.stop, StopReason::BudgetExhausted)),
                }
                let frames = frames.borrow();
                if record {
                    // Two prompt polls, then final capture after the command budget.
                    assert_eq!(frames.len(), 3);
                    let grids: Vec<_> = frames
                        .iter()
                        .map(|frame| {
                            let Frame::Text(grid) = frame else {
                                panic!("expected text frame")
                            };
                            grid
                        })
                        .collect();
                    assert_eq!(
                        grids
                            .iter()
                            .map(|grid| grid.cell(0, 0).unwrap().glyph)
                            .collect::<Vec<_>>(),
                        b"AAC"
                    );
                    assert_eq!(grids[0].cell(1, 0).unwrap().glyph, 0);
                    assert_eq!(grids[1].cell(1, 0).unwrap().glyph, b'B');
                    assert_eq!(frames.last(), Some(&Frame::Text(run.mda.clone())));
                } else {
                    assert!(frames.is_empty());
                }
                for (device, grid) in [(DeviceId::Vga, &run.screen), (DeviceId::Mda, &run.mda)] {
                    let expected = (kind == assembly::FrameSinkKind::Capture)
                        .then(|| Frame::Text(grid.clone()));
                    assert_eq!(assembled.observation.captured_frame(device), expected);
                }
                if let Some(expected) = &baseline {
                    assert_eq!(&run, expected);
                } else {
                    baseline = Some(run);
                }
            }
        }
    }

    #[test]
    fn divisor_programming_keeps_exact_serial_stream() {
        // National Semiconductor PC16550D datasheet, Line Control Register:
        // DLAB redirects offsets 0/1 to the divisor latches, not serial TX.
        let program = [
            0xFA, // CLI
            0xBA, 0xFB, 0x03, 0xB0, 0x80, 0xEE, // DLAB on
            0xBA, 0xF8, 0x03, 0xB0, 0x34, 0xEE, // divisor low
            0x42, 0xB0, 0x12, 0xEE, // divisor high
            0xBA, 0xFB, 0x03, 0xB0, 0x03, 0xEE, // DLAB off, 8N1
            0xBA, 0xF8, 0x03, 0xB0, b'O', 0xEE, 0xB0, b'K', 0xEE, 0xF4,
        ];
        assert_eq!(run_boot(&program).unwrap(), b"OK");
    }

    #[test]
    fn vga_snapshot_repeated_capture_preserves_the_grid() {
        let mut policy = display_policy();
        policy.trace = device::TracePolicy::Ring { capacity: 100 };
        let mut assembled =
            assemble_with_observation(&assembly::at_vga_text(VGA_TEXT_PROGRAM.to_vec()), &policy)
                .unwrap();
        assembled.machine.run().unwrap();
        let index = assembled.device_index[&DeviceId::Vga];
        assembled.machine.bus_mut().settle();
        let before = assembled.machine.bus().device(index).unwrap().save();
        let first = assembled.capture_text(DeviceId::Vga).unwrap();
        let second = assembled.capture_text(DeviceId::Vga).unwrap();
        let Frame::Text(grid) = &first.frame else {
            panic!("expected text frame")
        };
        assert_eq!(grid.cell(0, 0).unwrap().glyph, b'H');
        assert_eq!(second, first);
        assert_eq!(
            assembled.machine.bus().device(index).unwrap().save(),
            before
        );
        assert_eq!(first.virtual_time, assembled.machine.virtual_time());
        assert_eq!(first.retired, assembled.machine.cpu().retired());
        let trace = assembled
            .machine
            .bus_mut()
            .device_mut(index)
            .unwrap()
            .take_trace();
        assert!(!trace.is_empty(), "capture preserves existing accesses");
        assert_eq!(assembled.capture_text(DeviceId::Vga).unwrap(), first);
        assert!(assembled
            .machine
            .bus_mut()
            .device_mut(index)
            .unwrap()
            .take_trace()
            .is_empty());
    }

    #[test]
    fn bios_snapshot_survives_a_cpu_error_stop() {
        let mut rom = vec![0; 32768];
        let program = [
            0xB8, 0, 0xB8, 0x8E, 0xD8, 0xB8, b'X', 0x4E, 0xA3, 0, 0, 0xBA, 0xF8, 0x03, 0xB0, b'Z',
            0xEE, 0xB0, 0, 0xE6, 0x80, 0xE6, 0x80, 0xB0, 0xFF, 0xE6, 0x80, 0x0F, 0x05,
        ];
        rom[..program.len()].copy_from_slice(&program);
        rom[0x7FF0..0x7FF5].copy_from_slice(&[0xEA, 0, 0, 0, 0xF8]);
        let mut config = assembly::bios(rom, None);
        // Keep this host-error fixture on flat Application(Real16); an
        // A20-capable 80286 rig delivers processor exceptions in the guest.
        config.devices.retain(|device| {
            !matches!(
                device.kind,
                assembly::DeviceKind::Keyboard(_) | assembly::DeviceKind::SysCtrlPortA
            )
        });
        let run = run_bios_config(config, |_| {
            RunPolicy::natural(Some(100), Some(BIOS_IDLE_LIMIT))
        });
        assert!(
            matches!(run.stop, StopReason::Unsupported { .. }),
            "{:?}",
            run.stop
        );
        assert_eq!(
            run.screen.cell(0, 0),
            Some(&video::TextCell {
                glyph: b'X',
                attr: 0x4E,
                style: video::TextStyle::Colour {
                    foreground: 14,
                    background: 4,
                    blink: false
                },
            })
        );
        assert_eq!(run.com1, b"Z");
        assert_eq!(run.post, [0, 0, 0xFF]);
        let stamp = run.screen_stamp.unwrap();
        assert!(stamp.retired > 0);
        assert_eq!(stamp.virtual_time, stamp.retired);
    }

    #[test]
    fn bios_assembly_failure_has_no_screen_stamp() {
        let mut config = assembly::bios(vec![0xF4; 32768], None);
        config.devices.retain(|device| device.id != DeviceId::Vga);
        let run = run_bios_config(config, |_| RunPolicy::natural(Some(100), None));
        assert!(matches!(run.stop, StopReason::ConfigFailed { .. }));
        assert_eq!(run.screen_stamp, None);
    }

    fn run_bios_without_display_routes(removed: &[DeviceId]) -> BiosRun {
        let mut observation = bios_observation_policy();
        observation.routes.retain(|route| {
            route.output != OutputKind::TextScanout || !removed.contains(&route.device)
        });
        let mut assembled =
            assemble_with_observation(&assembly::bios(vec![0xF4; 32768], None), &observation)
                .unwrap();
        run_bios_assembled(&mut assembled, |_| RunPolicy::natural(Some(0), None))
    }

    #[test]
    fn bios_vga_capture_failure_is_an_observation_failure() {
        let run = run_bios_without_display_routes(&[DeviceId::Vga]);
        assert!(matches!(run.stop, StopReason::ObservationFailed { detail }
            if detail.contains("capturing display") && detail.contains("route is not bound")
                && !detail.contains("(MDA)")));
        assert_eq!(run.screen_stamp, None);
        assert_eq!(run.screen, blank_screen());
    }

    #[test]
    fn bios_mda_capture_failure_preserves_the_colour_stamp() {
        let run = run_bios_without_display_routes(&[DeviceId::Mda]);
        assert!(matches!(run.stop, StopReason::ObservationFailed { detail }
            if detail.contains("capturing display (MDA)") && detail.contains("route is not bound")));
        assert!(run.screen_stamp.is_some());
        assert_eq!(run.mda, blank_screen());
    }

    #[test]
    fn bios_display_capture_preserves_the_first_observation_failure() {
        let run = run_bios_without_display_routes(&[DeviceId::Vga, DeviceId::Mda]);
        assert!(matches!(run.stop, StopReason::ObservationFailed { detail }
            if detail.contains("Vga") && !detail.contains("(MDA)")));
        assert_eq!(run.screen_stamp, None);
    }

    #[test]
    fn bios_later_capture_failures_preserve_the_post_failure() {
        let mut assembled =
            machine_file::assemble(&assembly::bios(vec![0xF4; 32768], None)).unwrap();
        let run = run_bios_assembled(&mut assembled, |_| RunPolicy::natural(Some(0), None));
        assert!(matches!(run.stop, StopReason::ObservationFailed { detail }
            if detail.contains("capturing POST checkpoints")));
    }

    #[test]
    fn console_unavailable_is_classified_as_an_observation_failure() {
        let assembled = machine_file::assemble(&assembly::at_boot(vec![0xF4])).unwrap();
        let device = DeviceId::Keyboard;
        for stop in [
            classify_bios_stop(
                &assembled.machine,
                Err(RunError::ConsoleUnavailable {
                    device,
                    source: assembly::ObservationError::RouteNotBound {
                        device,
                        output: OutputKind::TextScanout,
                    },
                }),
            ),
            classify_test386_stop(
                0,
                Err(RunError::ConsoleUnavailable {
                    device,
                    source: assembly::ObservationError::RouteNotBound {
                        device,
                        output: OutputKind::TextScanout,
                    },
                }),
            ),
        ] {
            assert!(matches!(stop, StopReason::ObservationFailed { detail }
                if detail.contains("console text scanout is unavailable")));
        }
    }

    #[test]
    fn stream_overflow_is_an_explicit_failed_run() {
        let policy = byte_policy(DeviceId::Uart, ByteStreamKind::SerialTx);
        let mut tiny = policy;
        tiny.routes[0].byte_limit = 1;
        let mut assembled =
            assemble_with_observation(&assembly::at_boot(vec![0xF4]), &tiny).unwrap();
        assembled
            .machine
            .bus_mut()
            .port_write(chipset::COM1_BASE, 1, 0)
            .unwrap();
        assembled
            .machine
            .bus_mut()
            .port_write(chipset::COM1_BASE, 1, 0xFF)
            .unwrap();
        let mut stop = StopReason::Halt;
        let bytes = stream_or_fail(
            assembled
                .observation
                .drain_bytes(DeviceId::Uart, ByteStreamKind::SerialTx)
                .map_err(Into::into),
            &mut stop,
        );
        assert!(bytes.is_empty());
        assert!(
            matches!(stop, StopReason::ObservationFailed { detail } if detail.contains("1-byte limit"))
        );
    }
}

#[cfg(all(test, not(feature = "test-support")))]
#[path = "../tests/support/fixtures.rs"]
mod trace_fixtures;

#[cfg(all(test, feature = "test-support"))]
use test_support::fixtures as trace_fixtures;

#[cfg(test)]
mod bounded_trace_tests {
    use super::*;
    use device::TracePolicy;

    fn assert_bounds(assembled: &mut AssembledMachine, capacity: usize) -> u64 {
        let mut drops = assembled.machine.bus().open_bus_trace_dropped();
        assert!(assembled.machine.bus_mut().take_open_bus_trace().len() <= capacity);
        for (&id, &index) in &assembled.device_index {
            let device = assembled.machine.bus_mut().device_mut(index).unwrap();
            let dropped = device.trace_dropped();
            let events = device.take_trace();
            assert!(events.len() <= capacity, "{id:?}: {}", events.len());
            let scope = if id == DeviceId::Fdc {
                " (includes end-of-run floppy readback)"
            } else {
                ""
            };
            eprintln!(
                "{id:?}: retained={}, dropped={dropped}{scope}",
                events.len()
            );
            drops += dropped;
        }
        drops
    }

    #[test]
    #[ignore = "slow: compares all BIOS/DOS golden outputs with tracing off and overflowing"]
    fn bios_and_dos_outputs_ignore_trace_overflow_and_post_buffers_stay_bounded() {
        let (Some(rom), Some(image)) = (
            trace_fixtures::fixture("kiselev-bios/bios-xi8088.bin"),
            trace_fixtures::fixture("freedos/freedos.img"),
        ) else {
            return;
        };
        for workload in ["post", "floppy", "prompt", "dir", "write"] {
            let floppy = match workload {
                "post" => None,
                "floppy" => Some(bootable_floppy_image()),
                _ => Some(image.clone()),
            };
            let config = if matches!(workload, "post" | "floppy") {
                assembly::bios(rom.clone(), floppy)
            } else {
                machine_file::load(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../../machines/at-freedos.toml"),
                )
                .unwrap()
            };
            let mut results = Vec::new();
            for trace in [TracePolicy::Off, TracePolicy::Ring { capacity: 2 }] {
                let mut observation = bios_observation_policy();
                observation.trace = trace;
                let mut assembled = assemble_with_observation(&config, &observation).unwrap();
                let result = run_bios_assembled(&mut assembled, |indices| match workload {
                    "post" => {
                        RunPolicy::natural(Some(BIOS_GOLDEN_MAX_RETIRED), Some(BIOS_IDLE_LIMIT))
                    }
                    "floppy" => bios_natural_policy(),
                    _ => {
                        let script = match workload {
                            "dir" => dos_dir_script(),
                            "write" => dos_write_file_script(),
                            _ => Vec::new(),
                        };
                        let interactive = !script.is_empty();
                        let stimuli = if interactive {
                            vec![KeyboardStimulus {
                                keyboard_index: indices[&DeviceId::Keyboard],
                                scancodes: script,
                                schedule: InjectionSchedule::AfterPrompt,
                            }]
                        } else {
                            Vec::new()
                        };
                        freedos_prompt_policy(
                            stimuli,
                            if interactive {
                                DOS_INTERACTIVE_COMMAND_BUDGET
                            } else {
                                0
                            },
                        )
                    }
                });
                assert!(!result.post.is_empty(), "{workload} must execute BIOS POST");
                assert_eq!(result.stop, StopReason::BudgetExhausted, "{workload}");
                let console = result.mda.to_dump();
                match workload {
                    "post" => assert!(console.contains("Boot failed"), "{console}"),
                    "floppy" => assert_eq!(result.com1, b"B"),
                    "prompt" => assert!(console.contains(DOS_DATE_PROMPT), "{console}"),
                    "dir" | "write" => {
                        assert!(console.contains("Directory of A:"), "{console}");
                        assert!(
                            console.lines().any(|line| line.trim() == "A:\\>"),
                            "{console}"
                        );
                        if workload == "write" {
                            assert!(
                                console
                                    .lines()
                                    .any(|line| line.split_whitespace().next() == Some("ZZ")),
                                "{console}"
                            );
                            assert!(console.contains("3 file(s)"), "{console}");
                        } else {
                            assert!(console.contains("2 file(s)"), "{console}");
                        }
                    }
                    _ => unreachable!(),
                }
                let capacity = if trace == TracePolicy::Off { 0 } else { 2 };
                let drops = assert_bounds(&mut assembled, capacity);
                if trace == TracePolicy::Off {
                    assert_eq!(drops, 0);
                } else {
                    assert!(drops > 0, "{workload} must exercise overflow");
                }
                results.push(result);
            }
            assert_eq!(
                results[0], results[1],
                "{workload}: all outputs and stop must agree"
            );
        }
    }
}

#[cfg(test)]
mod configuration_error_tests {
    use super::*;

    #[test]
    fn invalid_runner_images_report_configuration_failure() {
        let diagnostic = run_test386(&vec![0; 0x10_0001]);
        assert!(matches!(diagnostic.stop, StopReason::ConfigFailed { .. }));
        assert!(diagnostic.post.is_empty());
        let bios = run_bios_with_cap(&vec![0; 0x8001], 0);
        assert!(matches!(bios.stop, StopReason::ConfigFailed { .. }));
        assert!(bios.post.is_empty());
    }
}

#[cfg(test)]
mod user_bios_tests {
    use super::*;

    #[test]
    fn bios_file_display_switch_follows_the_loaded_firmware() {
        let directory = machine_file::test_support::TempDir::new("cli-display-switch");
        let path = directory.path().join("bios.bin");
        let catalogue = assembly::BoardModel::Ibm5170.firmware_catalogue();
        for image in [
            "kiselev-bios/bios-xi8088.bin",
            "kiselev-bios/bios-xi8088-atreset.bin",
        ] {
            let Some(bytes) = trace_fixtures::fixture(image) else {
                return;
            };
            std::fs::write(&path, bytes).unwrap();
            let config = bios_file_config(&path, catalogue).unwrap();
            assert_eq!(config.display_switch, assembly::DisplaySwitch::Colour);
            let mut assembled = machine_file::assemble(&config).unwrap();
            assembled
                .machine
                .bus_mut()
                .port_write(0x64, 1, 0xC0)
                .unwrap();
            assert_eq!(
                assembled.machine.bus_mut().port_read(0x60, 1).unwrap(),
                0xA0
            );
            assert_eq!(
                assembly::derive_cmos_config(&config).unwrap()[0x14] & 0x30,
                0x20
            );
        }
        // The open AT BIOS reads the switch with the IBM polarity, so it keeps
        // Auto: the preset's MDA is selected, input bit 6 set, CMOS video 11b.
        let Some(bytes) = trace_fixtures::fixture("unemu-at-bios/bios-unemu-at.bin") else {
            return;
        };
        std::fs::write(&path, bytes).unwrap();
        let config = bios_file_config(&path, catalogue).unwrap();
        assert_eq!(config.display_switch, assembly::DisplaySwitch::Auto);
        assert_eq!(
            config.primary_display().unwrap(),
            assembly::PrimaryDisplay::Monochrome
        );
        let mut assembled = machine_file::assemble(&config).unwrap();
        assembled
            .machine
            .bus_mut()
            .port_write(0x64, 1, 0xC0)
            .unwrap();
        assert_eq!(
            assembled.machine.bus_mut().port_read(0x60, 1).unwrap(),
            0xE0
        );
        assert_eq!(
            assembly::derive_cmos_config(&config).unwrap()[0x14] & 0x30,
            0x30
        );
        std::fs::write(&path, vec![0xF4; 32768]).unwrap();
        let config = bios_file_config(&path, catalogue).unwrap();
        assert_eq!(config.display_switch, assembly::DisplaySwitch::Auto);
        let mut assembled = machine_file::assemble_with_options(
            &config,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap();
        assembled
            .machine
            .bus_mut()
            .port_write(0x64, 1, 0xC0)
            .unwrap();
        assert_eq!(
            assembled.machine.bus_mut().port_read(0x60, 1).unwrap(),
            0xE0
        );
    }

    #[test]
    fn preinterleaved_bios_file_uses_catalogue_placement_and_services() {
        let directory = machine_file::test_support::TempDir::new("cli-user-bios");
        let path = directory.path().join("assembled.bin");
        let bytes = vec![0; 65536];
        std::fs::write(&path, &bytes).unwrap();
        let entry = assembly::FirmwareEntry {
            id: "synthetic-user-bios",
            size: 65536,
            sha256: "de2f256064a0af797747c2b97505dc0b9f3df0de4f489eac731c23ae9ca9cc31",
            distribution: assembly::FirmwareDistribution::UserSupplied,
            provides: &[
                assembly::FirmwareService::Floppy,
                assembly::FirmwareService::FixedDisk,
            ],
            ..assembly::KISELEV_XI8088
        };
        assert!(entry.identifies(&bytes));
        assert!(!assembly::FirmwareEntry {
            size: 32768,
            ..entry
        }
        .identifies(&bytes));
        let config = bios_file_config(&path, &[entry]).unwrap();
        assert_eq!(config.images[0].base, 0xF0000);
        assert_eq!(
            config.images[0].source,
            assembly::ImageSource::Inline(bytes.clone())
        );
        assert!(config.images[0].provides.is_empty());
        assert!(assembly::check_system_bios(
            &config,
            assembly::BoardModel::Ibm5170,
            &[entry],
            config.images[0].base,
            &bytes,
            assembly::AssemblyOptions::default()
        )
        .unwrap()
        .is_none());
        let unknown = bios_file_config(&path, &[]).unwrap();
        assert_eq!(
            unknown.images[0].provides,
            vec![assembly::FirmwareService::Floppy]
        );
    }
}

#[cfg(test)]
mod small_bios_tests {
    use super::*;

    #[test]
    fn unlisted_16k_bios_file_is_top_aligned_under_escape() {
        let directory = machine_file::test_support::TempDir::new("cli-16k-bios");
        let path = directory.path().join("bios.bin");
        std::fs::write(&path, vec![0xF4; 16384]).unwrap();
        let config =
            bios_file_config(&path, assembly::BoardModel::Ibm5170.firmware_catalogue()).unwrap();
        assert_eq!(config.images[0].base, 0xFC000);
        let error =
            machine_file::assemble_with_options(&config, assembly::AssemblyOptions::default())
                .map(|_| ())
                .expect_err("unlisted firmware requires the escape");
        assert!(matches!(
            error,
            assembly::AssemblyError::Config(assembly::ConfigError::FirmwareNotListed { .. })
        ));
        let assembled = machine_file::assemble_with_options(
            &config,
            assembly::AssemblyOptions {
                allow_unlisted_firmware: true,
            },
        )
        .unwrap();
        assert_eq!(assembled.unlisted_firmware.len(), 1);
    }
}
