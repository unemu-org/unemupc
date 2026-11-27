// SPDX-License-Identifier: AGPL-3.0-or-later
//! A CPU-only reset with the A20 gate off, under a real system BIOS: the
//! project's own boot sector turns the gate off through the 8042, resets the
//! processor through a shutdown and then through the 8042's reset pulse, and
//! both times the BIOS resumes it through the CMOS shutdown byte (code 0Ah,
//! a jump through 0040:0067h without EOI). On the 80286 the reset fetch goes
//! to FFFFF0h; the board's gate would turn it into EFFFF0h, where nothing
//! answers, so without the reset-fetch window, at SP 1, the processor takes
//! #UD at F000:FFF0 and shuts down again and again: "64 resets within
//! 0 steps, the last shutdown at f000:fff0" (machine-config.md §A20 line and
//! System Control Port A, "Reset fetch with the gate off").
//!
//! IBM Personal Computer AT Technical Reference: System Board, Keyboard
//! Controller (output port bit 1 Gate A20 and bit 0 System Reset; commands
//! D0h, D1h and FEh), Real-Time Clock/CMOS RAM Information (shutdown status
//! byte 0Fh), and the BIOS listing's POST reset path (shutdown code 0Ah).
//! Intel 80286 Programmer's Reference Manual, Appendix C, Hardware
//! Compatibility Considerations ("Address after Reset"), and Appendix B,
//! LIDT and INT (a real-mode interrupt beyond the IDT limit shuts down).

use assembly::test_support::fixture_sha256;
use assembly::{AssemblyOptions, BoardModel, ObservationPolicy, RunPolicy};
use machine::RunOutcome;
use machine_file::test_support::TempDir;
use machine_file::test_support::UserFirmwareLayout;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[path = "support/fixtures.rs"]
mod fixtures;

const CPU_REV: &str = "fb7bc5b7fea0bef47ddfc31f557dd6b14db34668";
const OPEN_BIOS: &str = "unemu-at-bios/bios-unemu-at.bin";
// The BIOS image of machines/at-games-open.toml.
const OPEN_BIOS_SHA256: &str = "6938dc8b665fa120f33775163ab6bacc559389525db87adedf734416f9ef2442";
const IBM_ENTRY: &str = "ibm-5170-bios-1985-06-10";
// One POST, one boot and two resumes; POST dominates.
const CAP: u64 = 300_000_000;

/// The boot sector (its first 97h bytes; the rest is zero up to the 55h AAh
/// signature). The bytes are this NASM listing (`nasm -f bin`, org 7C00h);
/// the whole 512-byte sector's SHA-256 is pinned by [`BOOT_SECTOR_SHA256`].
/// Before each reset it leaves SP at 1, so a reset fetch that missed the ROM
/// cannot deliver its #UD: the frame's second word would sit at offset FFFFh,
/// across the top of the stack segment, and the processor shuts down again
/// (Intel 80286 Programmer's Reference Manual, Appendix B, INT: a real-mode
/// delivery shuts down at SP 1, 3 or 5 and wraps inside the segment at any
/// other SP). The reported Windows 3.1 stop had SP at 2, which shut down
/// while the CPU core required the whole frame to fit below SP; at 2 the #UD
/// now delivers, and its handler's IRET reloads CS with a real-mode base,
/// which reaches the low ROM copy and would hide the missing reset fetch. At
/// each resume n
/// (1, 2) it stores at 0510h + n the 8042 output port read back with
/// command D0h and at 0530h + n the byte at FFFF:0610h after writing n to
/// 0000:0600h (n when the gate wraps).
///
/// ```text
/// 7C00 FA               cli
/// 7C01 31C0             xor ax, ax
/// 7C03 8ED8             mov ds, ax
/// 7C05 8ED0             mov ss, ax
/// 7C07 BC007C           mov sp, 7C00h
/// 7C0A C7066704347C     mov word [0467h], 7C34h   ; 0040:0067 -> resume
/// 7C10 C70669040000     mov word [0469h], 0
/// 7C16 E87100           call 7C8Ah                ; wait for an empty input buffer
/// 7C19 B0D1             mov al, 0D1h              ; write output port
/// 7C1B E664             out 64h, al
/// 7C1D E86A00           call 7C8Ah
/// 7C20 B0DD             mov al, 0DDh              ; gate A20 off, reset line high
/// 7C22 E660             out 60h, al
/// 7C24 E86300           call 7C8Ah
/// 7C27 E85700           call 7C81h                ; shutdown code 0Ah
/// 7C2A BC0100           mov sp, 1                 ; a frame word would sit at FFFFh
/// 7C2D 0F011E917C       lidt [7C91h]
/// 7C32 CC               int3                      ; beyond the limit: shutdown
/// 7C33 F4               hlt
/// 7C34 FA               cli                       ; resume
/// 7C35 31C0             xor ax, ax
/// 7C37 8ED8             mov ds, ax
/// 7C39 8ED0             mov ss, ax
/// 7C3B BC007C           mov sp, 7C00h
/// 7C3E FE060005         inc byte [0500h]
/// 7C42 8A1E0005         mov bl, [0500h]
/// 7C46 30FF             xor bh, bh
/// 7C48 E83F00           call 7C8Ah
/// 7C4B B0D0             mov al, 0D0h              ; read output port
/// 7C4D E664             out 64h, al
/// 7C4F E464             in al, 64h
/// 7C51 A801             test al, 1
/// 7C53 74FA             jz 7C4Fh
/// 7C55 E460             in al, 60h
/// 7C57 88871005         mov [bx+0510h], al
/// 7C5B 881E0006         mov [0600h], bl
/// 7C5F B8FFFF           mov ax, 0FFFFh
/// 7C62 8EC0             mov es, ax
/// 7C64 26A01006         mov al, [es:0610h]        ; 100600h, or 000600h gated
/// 7C68 88873005         mov [bx+0530h], al
/// 7C6C 80FB01           cmp bl, 1
/// 7C6F 750D             jne 7C7Eh
/// 7C71 E80D00           call 7C81h                ; shutdown code 0Ah
/// 7C74 BC0100           mov sp, 1                 ; a frame word would sit at FFFFh
/// 7C77 B0FE             mov al, 0FEh              ; pulse the reset line
/// 7C79 E664             out 64h, al
/// 7C7B F4               hlt
/// 7C7C EBFD             jmp 7C7Bh
/// 7C7E F4               hlt                       ; done
/// 7C7F EBFD             jmp 7C7Eh
/// 7C81 B08F             mov al, 8Fh               ; CMOS 0Fh, NMI masked
/// 7C83 E670             out 70h, al
/// 7C85 B00A             mov al, 0Ah
/// 7C87 E671             out 71h, al
/// 7C89 C3               ret
/// 7C8A E464             in al, 64h
/// 7C8C A802             test al, 2
/// 7C8E 75FA             jnz 7C8Ah
/// 7C90 C3               ret
/// 7C91 0000 0000 0000   IDTR: limit 0, base 0
/// 7DFE 55AA             boot signature
/// ```
#[rustfmt::skip]
const BOOT_CODE: &[u8] = &[
    0xFA, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, 0xBC, 0x00, 0x7C, 0xC7, 0x06, 0x67, 0x04, 0x34, 0x7C,
    0xC7, 0x06, 0x69, 0x04, 0x00, 0x00, 0xE8, 0x71, 0x00, 0xB0, 0xD1, 0xE6, 0x64, 0xE8, 0x6A, 0x00,
    0xB0, 0xDD, 0xE6, 0x60, 0xE8, 0x63, 0x00, 0xE8, 0x57, 0x00, 0xBC, 0x01, 0x00, 0x0F, 0x01, 0x1E,
    0x91, 0x7C, 0xCC, 0xF4, 0xFA, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xD0, 0xBC, 0x00, 0x7C, 0xFE, 0x06,
    0x00, 0x05, 0x8A, 0x1E, 0x00, 0x05, 0x30, 0xFF, 0xE8, 0x3F, 0x00, 0xB0, 0xD0, 0xE6, 0x64, 0xE4,
    0x64, 0xA8, 0x01, 0x74, 0xFA, 0xE4, 0x60, 0x88, 0x87, 0x10, 0x05, 0x88, 0x1E, 0x00, 0x06, 0xB8,
    0xFF, 0xFF, 0x8E, 0xC0, 0x26, 0xA0, 0x10, 0x06, 0x88, 0x87, 0x30, 0x05, 0x80, 0xFB, 0x01, 0x75,
    0x0D, 0xE8, 0x0D, 0x00, 0xBC, 0x01, 0x00, 0xB0, 0xFE, 0xE6, 0x64, 0xF4, 0xEB, 0xFD, 0xF4, 0xEB,
    0xFD, 0xB0, 0x8F, 0xE6, 0x70, 0xB0, 0x0A, 0xE6, 0x71, 0xC3, 0xE4, 0x64, 0xA8, 0x02, 0x75, 0xFA,
    0xC3, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// SHA-256 of [`boot_sector`], the output of its NASM listing.
const BOOT_SECTOR_SHA256: &str = "6872f80f1a05a8dcbf848af74cf77b3a96caef285894a005d6c73d5e3c4a1cad";

fn boot_sector() -> Vec<u8> {
    let mut sector = vec![0; 512];
    sector[..BOOT_CODE.len()].copy_from_slice(BOOT_CODE);
    sector[510..].copy_from_slice(&[0x55, 0xAA]);
    sector
}

/// A 1.2 MB floppy whose first sector is [`boot_sector`].
fn write_floppy(directory: &Path) -> PathBuf {
    let mut image = vec![0; 1_228_800];
    image[..512].copy_from_slice(&boot_sector());
    let path = directory.join("a20-off-reset.img");
    std::fs::write(&path, image).unwrap();
    path
}

fn toml_string(path: &Path) -> String {
    let text = path.to_str().expect("UTF-8 path");
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            character if character.is_control() => {
                write!(quoted, "\\u{:04x}", u32::from(character)).unwrap()
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

/// The IBM 5170 preset with 640 KiB conventional and 1 MiB extended memory,
/// derived CMOS and the floppy as A:, as the protected-mode test's machine.
fn machine_file(directory: &Path, board: &str, firmware: &str) -> PathBuf {
    let floppy = write_floppy(directory);
    let text = format!(
        "schema = 1\nextends = \"preset:ibm-5170\"\nname = \"a20-off-reset\"\n\
         {board}\n\
         [board.rtc]\nstart = \"2000-01-01T00:00:00\"\nnvram = \"derived\"\n\
         [cpu]\nfpu = \"none\"\n\
         [memory]\nconventional_kib = 640\nextended_kib = 1024\n\
         [firmware.bios]\n{firmware}\n\
         [[card]]\nid = \"fdc-765\"\n\
         [[card.drives]]\nunit = 0\ntype = \"1.2M\"\nimage = {}\nwrite = \"readonly\"\n",
        toml_string(&floppy)
    );
    let path = directory.join("a20-off-reset.toml");
    std::fs::write(&path, text).unwrap();
    path
}

/// What the boot sector recorded: resumes, then per resume the 8042 output
/// port read back and the FFFF:0610h probe.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    outcome: RunOutcome,
    resumes: u8,
    output_port: [u8; 2],
    probe: [u8; 2],
}

fn run(file: &Path) -> anyhow::Result<Seen> {
    let loaded = machine_file::load_for_assembly(file, AssemblyOptions::default())?;
    let mut assembled = loaded.assemble(&ObservationPolicy::default())?;
    let outcome = assembled.run_session(&RunPolicy::natural(
        Some(CAP),
        Some(pc_harness::BIOS_IDLE_LIMIT),
    ))?;
    let cpu = assembled.machine.cpu();
    let byte = |addr: u64| -> anyhow::Result<u8> {
        let mut value = [0];
        cpu.read_mem(addr, &mut value)?;
        Ok(value[0])
    };
    Ok(Seen {
        outcome,
        resumes: byte(0x500)?,
        output_port: [byte(0x511)?, byte(0x512)?],
        probe: [byte(0x531)?, byte(0x532)?],
    })
}

/// Both resumes ran; the gate the guest turned off reads back off (bit 1 of
/// the output port clear) and still wraps FFFF:0610h onto 000600h.
fn assert_resumed_with_the_gate_off(seen: &Seen) {
    assert_eq!(seen.outcome, RunOutcome::Halted, "{seen:?}");
    assert_eq!(seen.resumes, 2, "both resets resumed: {seen:?}");
    assert!(
        seen.output_port.iter().all(|port| port & 2 == 0),
        "the 8042 output port reads back with A20 off: {seen:?}"
    );
    assert_eq!(
        seen.probe,
        [1, 2],
        "the gate wraps after each reset: {seen:?}"
    );
}

#[test]
fn the_boot_sector_matches_its_pinned_listing() {
    assert_eq!(fixture_sha256(&boot_sector()), BOOT_SECTOR_SHA256);
}

#[test]
#[ignore = "slow: open AT BIOS POST, then two CPU resets with A20 off"]
fn open_bios_resumes_after_cpu_resets_with_the_gate_off() {
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    if fixtures::fixture(OPEN_BIOS).is_none() {
        return;
    }
    let temporary = TempDir::new("a20-off-reset-open");
    let bios = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor")
        .join(OPEN_BIOS);
    let firmware = format!(
        "provides = [\"floppy\"]\nimage = {}\nsha256 = \"{OPEN_BIOS_SHA256}\"",
        toml_string(&bios)
    );
    let seen = run(&machine_file(
        temporary.path(),
        "[board]\ndisplay_switch = \"mono\"",
        &firmware,
    ))
    .unwrap();
    eprintln!("a20-off reset, open AT BIOS: {seen:?}");
    assert_resumed_with_the_gate_off(&seen);
}

/// The same with the user-supplied IBM 5170 BIOS of 1985-06-10; the
/// verdict is printed, never the firmware's screen.
#[test]
#[ignore = "requires the user-supplied IBM 5170 system ROM"]
fn user_rom_ibm_5170_resumes_after_cpu_resets_with_the_gate_off() {
    let catalogue = BoardModel::Ibm5170.firmware_catalogue();
    let found = machine_file::require_user_firmware!(IBM_ENTRY, catalogue);
    assert!(
        include_str!("../../../Cargo.lock").contains(CPU_REV),
        "CPU pin drifted"
    );
    let entry = catalogue
        .iter()
        .find(|entry| entry.id == IBM_ENTRY)
        .expect("catalogued IBM 5170 BIOS");
    let mut firmware = format!(
        "provides = [\"floppy\", \"fixed-disk\"]\nsha256 = \"{}\"\n",
        entry.sha256
    );
    match &found.source {
        assembly::ImageSource::File(path) => {
            write!(firmware, "image = {}", toml_string(path)).unwrap()
        }
        assembly::ImageSource::Interleaved(parts) => {
            // Each chip carries its catalogue identity, as at_pm_test_golden.rs.
            let known: &[_] = match &found.layout {
                UserFirmwareLayout::Parts(layout) => entry.parts[*layout],
                _ => &[],
            };
            firmware.push_str("parts = [\n");
            for part in parts {
                let lane = match part.lane {
                    assembly::Lane::Even => "even",
                    assembly::Lane::Odd => "odd",
                };
                write!(
                    firmware,
                    "{{ image = {}, lane = \"{lane}\", offset = {}",
                    toml_string(&part.path),
                    part.offset
                )
                .unwrap();
                if let Some(chip) = known
                    .iter()
                    .find(|chip| chip.lane == part.lane && chip.offset == part.offset)
                {
                    write!(firmware, ", sha256 = \"{}\"", chip.sha256).unwrap();
                }
                firmware.push_str(" },\n");
            }
            firmware.push(']');
        }
        _ => panic!("user firmware discovery must return file-backed sources"),
    }
    // The IBM POST probes the fixed-disk controller that shares the diskette
    // adapter; an empty AT adapter answers it (see at_pm_test_golden.rs).
    let board = "[board]\ndisplay_switch = \"auto\"\n\
         [[card]]\nid = \"vga-text\"\nremove = true\n\
         [[card]]\nkind = \"ide\"\ncontroller_model = \"at-adapter\"\nport = 0x1F0\nirq = 14";
    let temporary = TempDir::new("a20-off-reset-ibm");
    let seen = found.expect_redacted(
        run(&machine_file(temporary.path(), board, &firmware)),
        "running the boot sector",
    );
    eprintln!("a20-off reset, {IBM_ENTRY}: {seen:?}");
    assert_resumed_with_the_gate_off(&seen);
}
