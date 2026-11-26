// SPDX-License-Identifier: AGPL-3.0-or-later
//! Real-mode floppy WRITE golden test (§7, §7.7, §7.8, §10).
//!
//! The first end-to-end path where a guest **writes to media**. A hand-encoded
//! real-mode guest runs on the PC/AT machine shape (UART + cascaded 8259 pair +
//! PIT + 8042 + VGA + CMOS/RTC, plus an 8237 DMA controller and a µPD765 floppy
//! controller backed by a **blank writable** floppy) through the real `X86JitCpu`
//! in `CpuMode::Real16`. It:
//!
//! 1. brings the µPD765 out of reset (DOR: not-reset + DMA-enable + motor)
//!    and selects 250 kbps for the double-density medium;
//! 2. programs 8237 channel 2 for a **memory→device** transfer of one 512-byte
//!    sector from a preloaded source buffer holding a distinctive pattern, then
//!    issues **WRITE DATA** for drive 0 at C=0/H=0/R=1 and drains the result phase;
//! 3. programs 8237 channel 2 for a **device→memory** transfer into a separate
//!    read-back buffer, then issues **READ DATA** for the same CHS;
//! 4. compares the read-back bytes against the pattern and, on a match, emits the
//!    marker byte `W` to the UART (COM1 THR, `0x3F8`), then busy-spins on a port
//!    read until the run's retired-instruction cap stops it.
//!
//! The machine services the DMA on its run boundaries (both directions), exactly
//! as the floppy READ golden relies on — so the write is committed and the sector
//! is read back within the same run. Because the floppy starts **blank** (all
//! zeroes) and the pattern contains non-zero bytes throughout, a match can only mean
//! the guest's own write reached media: a broken or no-op write leaves the sector
//! zero, the read-back mismatches, and no marker is emitted. The captured serial is
//! the golden (`b"W"`), fully deterministic because nothing in the path reads a
//! wall clock (§6).
//!
//! The segment registers and stack pointer are seeded by the embedder
//! (`X86JitCpu::new_real16_bare`) because the `x86jit` core does not lift
//! `mov Sreg, r16`; the guest starts straight at device setup.
//!
//! ## Pinned CPU revision
//!
//! Golden traces are only valid against a known CPU revision (AGENTS.md); this
//! path runs the same pinned `x86jit-core` rev as the other CPU-driven goldens —
//! the rev in `Cargo.lock`. Re-baseline when that rev moves.

use pc_harness::{
    fdc_write_pattern, run_fdc_write, FDC_WRITE_MARKER, FDC_WRITE_PROGRAM, FDC_WRITE_SRC,
};
use testkit::golden::assert_bytes_eq;

#[path = "support/host_profile.rs"]
mod host_profile;
#[path = "support/serving.rs"]
mod serving;

#[test]
fn fdc_write_commits_a_sector_and_reads_it_back() {
    let serial = run_fdc_write(FDC_WRITE_PROGRAM).expect("machine run");
    // Exactly one marker: the read-back sector matched the written pattern, which
    // (against a blank floppy) proves the WRITE DATA reached media through the
    // symmetric DMA seam.
    assert_bytes_eq(&serial, &[FDC_WRITE_MARKER]);
}

/// Guard the hand-encoded program bytes so an accidental edit is caught
/// independently of the run, and document every instruction's exact bytes.
///
/// Loaded at linear `0x7C00` (segment `0x0000`, IP `0x7C00`; CS = 0, so a linear
/// address equals its offset). The source pattern is preloaded at `0x0600`; the
/// read-back buffer is at `0x0A00`.
///
/// ```text
///   7C00: B0 1C              mov al,0x1C                  ; DOR: not-reset|DMA|motor0
///   7C02: BA F2 03           mov dx,0x03F2                ; DOR port
///   7C05: EE                 out dx,al
///   7C06: B0 02              mov al,0x02                  ; 250 kbps for DD media
///   7C08: BA F7 03           mov dx,0x03F7                ; CCR port
///   7C0B: EE                 out dx,al
///   ; --- 8237 ch2: memory -> device (read), src 0x0600, 512 bytes ---
///   7C0C: B0 06 E6 0A        mov al,0x06 ; out 0x0A,al    ; mask ch2
///   7C10: B0 00 E6 0C        mov al,0x00 ; out 0x0C,al    ; clear flip-flop
///   7C14: B0 4A E6 0B        mov al,0x4A ; out 0x0B,al    ; mode: ch2 read single
///   7C18: B0 00 E6 04        mov al,0x00 ; out 0x04,al    ; addr low
///   7C1C: B0 06 E6 04        mov al,0x06 ; out 0x04,al    ; addr high -> 0x0600
///   7C20: B0 00 E6 81        mov al,0x00 ; out 0x81,al    ; page ch2
///   7C24: B0 FF E6 05        mov al,0xFF ; out 0x05,al    ; count low
///   7C28: B0 01 E6 05        mov al,0x01 ; out 0x05,al    ; count high -> 512
///   7C2C: B0 02 E6 0A        mov al,0x02 ; out 0x0A,al    ; unmask ch2
///   ; --- WRITE DATA (0x05): drive 0, C=0 H=0 R=1 N=2 EOT=1 (DATA @ 0x3F5) ---
///   7C30: BA F5 03           mov dx,0x03F5                ; DATA port
///   7C33: B0 05 EE           mov al,0x05 ; out dx,al      ; WRITE DATA
///   7C36: B0 00 EE           mov al,0x00 ; out dx,al      ; drive/head
///   7C39: B0 00 EE           mov al,0x00 ; out dx,al      ; cylinder
///   7C3C: B0 00 EE           mov al,0x00 ; out dx,al      ; head
///   7C3F: B0 01 EE           mov al,0x01 ; out dx,al      ; sector R=1
///   7C42: B0 02 EE           mov al,0x02 ; out dx,al      ; N=2 (512-byte sector)
///   7C45: B0 01 EE           mov al,0x01 ; out dx,al      ; EOT=1
///   7C48: B0 1B EE           mov al,0x1B ; out dx,al      ; GPL
///   7C4B: B0 FF EE           mov al,0xFF ; out dx,al      ; DTL
///   7C4E: EC x7              in al,dx  (x7)               ; drain WRITE result phase
///   ; --- 8237 ch2: device -> memory (write), dst 0x0A00, 512 bytes ---
///   7C55: B0 06 E6 0A        mov al,0x06 ; out 0x0A,al    ; mask ch2
///   7C59: B0 00 E6 0C        mov al,0x00 ; out 0x0C,al    ; clear flip-flop
///   7C5D: B0 46 E6 0B        mov al,0x46 ; out 0x0B,al    ; mode: ch2 write single
///   7C61: B0 00 E6 04        mov al,0x00 ; out 0x04,al    ; addr low
///   7C65: B0 0A E6 04        mov al,0x0A ; out 0x04,al    ; addr high -> 0x0A00
///   7C69: B0 00 E6 81        mov al,0x00 ; out 0x81,al    ; page ch2
///   7C6D: B0 FF E6 05        mov al,0xFF ; out 0x05,al    ; count low
///   7C71: B0 01 E6 05        mov al,0x01 ; out 0x05,al    ; count high -> 512
///   7C75: B0 02 E6 0A        mov al,0x02 ; out 0x0A,al    ; unmask ch2
///   ; --- READ DATA (0x06): same CHS, streams the sector into 0x0A00 ---
///   7C79: BA F5 03           mov dx,0x03F5                ; DATA port
///   7C7C: B0 06 EE           mov al,0x06 ; out dx,al      ; READ DATA
///   7C7F: B0 00 EE           mov al,0x00 ; out dx,al      ; drive/head
///   7C82: B0 00 EE           mov al,0x00 ; out dx,al      ; cylinder
///   7C85: B0 00 EE           mov al,0x00 ; out dx,al      ; head
///   7C88: B0 01 EE           mov al,0x01 ; out dx,al      ; sector R=1
///   7C8B: B0 02 EE           mov al,0x02 ; out dx,al      ; N=2
///   7C8E: B0 01 EE           mov al,0x01 ; out dx,al      ; EOT=1
///   7C91: B0 1B EE           mov al,0x1B ; out dx,al      ; GPL
///   7C94: B0 FF EE           mov al,0xFF ; out dx,al      ; DTL
///   ; --- compare all 512 read-back bytes, regenerating 0xC0 ^ index ---
///   7C97: BE 00 0A           mov si,0x0A00
///   7C9A: B9 00 02           mov cx,512
///   7C9D: 31 DB              xor bx,bx
///   7C9F: 8A C3              mov al,bl                    ; compare loop
///   7CA1: 34 C0              xor al,0xC0
///   7CA3: 38 04              cmp [si],al
///   7CA5: 75 0A              jne end (+10)
///   7CA7: 46                 inc si
///   7CA8: 43                 inc bx
///   7CA9: E2 F4              loop compare (-12)
///   7CAB: B0 57              mov al,0x57 ('W')
///   7CAD: BA F8 03           mov dx,0x03F8                ; COM1 THR
///   7CB0: EE                 out dx,al                    ; transmit marker
///   7CB1: EC                 in al,dx                     ; I/O-yielding spin  <- end
///   7CB2: EB FD              jmp $-3                      ; back to the in
/// ```
#[test]
fn fdc_write_program_bytes_are_the_documented_encoding() {
    #[rustfmt::skip]
    let expected: &[u8] = &[
        0xB0, 0x1C, 0xBA, 0xF2, 0x03, 0xEE,             // DOR out of reset
        0xB0, 0x02, 0xBA, 0xF7, 0x03, 0xEE,             // CCR: 250 kbps for DD media
        0xB0, 0x06, 0xE6, 0x0A,                         // mask ch2
        0xB0, 0x00, 0xE6, 0x0C,                         // clear flip-flop
        0xB0, 0x4A, 0xE6, 0x0B,                         // mode: ch2 read single
        0xB0, 0x00, 0xE6, 0x04,                         // addr low
        0xB0, 0x06, 0xE6, 0x04,                         // addr high -> 0x0600
        0xB0, 0x00, 0xE6, 0x81,                         // page ch2
        0xB0, 0xFF, 0xE6, 0x05,                         // count low
        0xB0, 0x01, 0xE6, 0x05,                         // count high -> 512
        0xB0, 0x02, 0xE6, 0x0A,                         // unmask ch2
        0xBA, 0xF5, 0x03,                               // mov dx,0x03F5 (DATA)
        0xB0, 0x05, 0xEE,                               // WRITE DATA
        0xB0, 0x00, 0xEE,                               // drive/head
        0xB0, 0x00, 0xEE,                               // cylinder
        0xB0, 0x00, 0xEE,                               // head
        0xB0, 0x01, 0xEE,                               // sector R=1
        0xB0, 0x02, 0xEE,                               // N=2
        0xB0, 0x01, 0xEE,                               // EOT=1
        0xB0, 0x1B, 0xEE,                               // GPL
        0xB0, 0xFF, 0xEE,                               // DTL
        0xEC, 0xEC, 0xEC, 0xEC, 0xEC, 0xEC, 0xEC,       // drain WRITE result (7 bytes)
        0xB0, 0x06, 0xE6, 0x0A,                         // mask ch2
        0xB0, 0x00, 0xE6, 0x0C,                         // clear flip-flop
        0xB0, 0x46, 0xE6, 0x0B,                         // mode: ch2 write single
        0xB0, 0x00, 0xE6, 0x04,                         // addr low
        0xB0, 0x0A, 0xE6, 0x04,                         // addr high -> 0x0A00
        0xB0, 0x00, 0xE6, 0x81,                         // page ch2
        0xB0, 0xFF, 0xE6, 0x05,                         // count low
        0xB0, 0x01, 0xE6, 0x05,                         // count high -> 512
        0xB0, 0x02, 0xE6, 0x0A,                         // unmask ch2
        0xBA, 0xF5, 0x03,                               // mov dx,0x03F5 (DATA)
        0xB0, 0x06, 0xEE,                               // READ DATA
        0xB0, 0x00, 0xEE,                               // drive/head
        0xB0, 0x00, 0xEE,                               // cylinder
        0xB0, 0x00, 0xEE,                               // head
        0xB0, 0x01, 0xEE,                               // sector R=1
        0xB0, 0x02, 0xEE,                               // N=2
        0xB0, 0x01, 0xEE,                               // EOT=1
        0xB0, 0x1B, 0xEE,                               // GPL
        0xB0, 0xFF, 0xEE,                               // DTL
        0xBE, 0x00, 0x0A,                               // mov si,0x0A00
        0xB9, 0x00, 0x02,                               // mov cx,512
        0x31, 0xDB,                                     // xor bx,bx
        0x8A, 0xC3, 0x34, 0xC0,                         // mov al,bl ; xor al,0xC0
        0x38, 0x04, 0x75, 0x0A,                         // cmp [si],al ; jne end
        0x46, 0x43, 0xE2, 0xF4,                         // inc si ; inc bx ; loop compare
        0xB0, 0x57, 0xBA, 0xF8, 0x03, 0xEE,             // mov al,'W' ; mov dx,0x3F8 ; out dx,al
        0xEC, 0xEB, 0xFD,                               // in al,dx ; jmp $-3 (I/O-yielding spin, end)
    ];
    assert_eq!(FDC_WRITE_PROGRAM, expected);

    assert_eq!(fdc_write_pattern().len(), 512);

    // The marker the guest transmits is the byte the golden asserts.
    assert_eq!(FDC_WRITE_MARKER, b'W');
}

#[test]
fn fdc_write_rejects_corruption_in_every_sector_byte() {
    // Corrupt the DMA source before WRITE DATA. Expected bytes are regenerated
    // independently by the guest, so even a damaged source must be rejected.
    for offset in 0u16..512 {
        let address = FDC_WRITE_SRC + u64::from(offset);
        let mut program = vec![
            0xC6,
            0x06,
            address as u8,
            (address >> 8) as u8,
            fdc_write_pattern()[usize::from(offset)] ^ 0xFF,
        ];
        program.extend_from_slice(FDC_WRITE_PROGRAM);
        let serial = run_fdc_write(&program).expect("machine run");
        assert!(
            serial.is_empty(),
            "corrupted sector byte {offset} was accepted"
        );
    }
}

/// The write-and-read-back golden with the machine's served ranges served in
/// process and with every access returning: the 8237 is programmed through
/// served ports, and the last command byte of WRITE DATA and of READ DATA
/// at the served µPD765 data port raises the controller's DMA request (NEC
/// µPD765A datasheet, WRITE DATA, READ DATA), memory to device and device to
/// memory. Both ways leave the same machine, serial output and traces.
#[test]
fn fdc_write_serving_in_process_matches_every_access_returning() {
    use assembly::{DeviceId, ObservationPolicy, ObservationRequest, OutputKind};
    use device::ByteStreamKind;
    let compared = serving::compare(
        &serving::Lockstep {
            label: "fdc write",
            step: 53,
            cap: pc_harness::FDC_WRITE_RETIRED_CAP,
            idle_limit: None,
            outputs: vec![(DeviceId::Uart, ByteStreamKind::SerialTx)],
            displays: Vec::new(),
            profile: Some(|rig, label| {
                Box::new(host_profile::WorkloadProfile::from_env(rig, || label))
            }),
            inject: None,
            audio: false,
        },
        || {
            let config = assembly::at_fdc_write(
                FDC_WRITE_PROGRAM.to_vec(),
                fdc_write_pattern(),
                FDC_WRITE_SRC,
            );
            let policy = ObservationPolicy {
                routes: vec![ObservationRequest {
                    device: DeviceId::Uart,
                    output: OutputKind::Bytes(ByteStreamKind::SerialTx),
                    byte_limit: 65536,
                }],
                ..Default::default()
            };
            machine_file::assemble_with_observation(&config, &policy).unwrap()
        },
        |_| false,
    );
    assert_eq!(compared.outcome, machine::RunOutcome::RetiredCapReached);
    assert_bytes_eq(&compared.outputs[0], &[FDC_WRITE_MARKER]);
    assert!(
        compared.served_accesses.ports > 30,
        "{:?}",
        compared.served_accesses
    );
}
