// SPDX-License-Identifier: AGPL-3.0-or-later
//! Extended memory across a restart of the open AT: no VDISK boot record may
//! survive POST, or the next DOS session's XMS driver refuses to load.
//!
//! IBM VDISK marks the extended memory it owns with a boot record at 100000h,
//! the signature "VDISK" at 100003h. The FreeDOS kernel writes such a record
//! when it loads into the HMA (kernel/inithma.c, InstallVDISK), and FDXMS286
//! refuses to install when it finds the signature (FDXMS286.ASM, check_vdisk,
//! FFFF:0013). RAM keeps its contents across Ctrl+Alt+Del (IBM Personal
//! Computer AT Technical Reference, BIOS, keyboard interrupt: a jump to POST)
//! and across a CPU reset through the 8042 (Keyboard Controller, output port
//! bit 0), so only POST can remove a record the last session left behind.
//! The open AT BIOS clears the memory it sizes above 1 MiB on every POST, as
//! the IBM PC/AT BIOS listing dated 06/10/85 does in TEST1, "storage
//! size/clear" (HOW_BIG_2A), which clears each sized 64 KiB block above 1 MiB,
//! on a soft reset too; release 1.0.2-unemu.2 did not, and this test failed
//! with it. The machine is `machines/at-ide.toml` with the game profile's 1 MiB
//! of extended memory: the same open AT BIOS, keyboard controller and XTIDE
//! ROM, without the VGA BIOS, so a candidate system BIOS named by
//! `UNEMUPC_OPEN_AT_BIOS` can replace the pinned one. See machines/README.md
//! §Playing DOS games. CPU revision is pinned by Cargo.lock.

use assembly::{
    AssemblyOptions, DeviceId, ImageSource, ObservationPolicy, ObservationRequest, OutputKind,
};

#[path = "support/fat12.rs"]
mod fat12;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/session.rs"]
pub mod session;

const CPU_REV: &str = "8f23fdcd6f9ce2a1fda2b105104d8a67a82bf126";
const VDISK_BASE: u64 = 0x100000;

/// The record the FreeDOS kernel copies to FFFF:0010 (InstallVDISK): a VDISK
/// boot sector of a 64 KiB RAM disk, and at offset 1Eh the first free
/// extended address in KiB (1024 + 64).
fn kernel_vdisk_record() -> [u8; 32] {
    let mut record = [0; 32];
    record[..22].copy_from_slice(b"\xCF  VDISK   \x00\x02FDOS  \x80\x00 ");
    record[0x1E..].copy_from_slice(&1088u16.to_le_bytes());
    record
}

/// A DOS program that leaves the kernel's record behind, as a DOS=HIGH
/// session does: A20 on through the 8042 output port (command D1h, DFh),
/// the record copied to FFFF:0010, A20 off again (DDh).
fn plant_program() -> Vec<u8> {
    // IN AL, 64h; TEST AL, 2; JNZ back: wait until the 8042 input buffer is empty.
    const WAIT: [u8; 6] = [0xE4, 0x64, 0xA8, 0x02, 0x75, 0xFA];
    let a20 = |code: &mut Vec<u8>, port_value: u8| {
        code.extend(WAIT);
        code.extend([0xB0, 0xD1, 0xE6, 0x64]);
        code.extend(WAIT);
        code.extend([0xB0, port_value, 0xE6, 0x60]);
        code.extend(WAIT);
    };
    let mut code = vec![0xFA];
    a20(&mut code, 0xDF);
    // PUSH FFFFh; POP ES; MOV DI, 10h; MOV SI, record; MOV CX, 32; CLD; REP MOVSB
    code.extend([0x68, 0xFF, 0xFF, 0x07, 0xBF, 0x10, 0x00, 0xBE]);
    let record_operand = code.len();
    code.extend([0, 0, 0xB9, 0x20, 0x00, 0xFC, 0xF3, 0xA4]);
    a20(&mut code, 0xDD);
    // STI; MOV DX, message; MOV AH, 9; INT 21h; MOV AX, 4C00h; INT 21h
    code.extend([0xFB, 0xBA]);
    let message_operand = code.len();
    code.extend([0, 0, 0xB4, 0x09, 0xCD, 0x21, 0xB8, 0x00, 0x4C, 0xCD, 0x21]);
    let record = 0x100 + code.len() as u16;
    code[record_operand..record_operand + 2].copy_from_slice(&record.to_le_bytes());
    code.extend(kernel_vdisk_record());
    let message = 0x100 + code.len() as u16;
    code[message_operand..message_operand + 2].copy_from_slice(&message.to_le_bytes());
    code.extend(b"VDISK-PLANTED\r\n$");
    code
}

/// A DOS program that reboots through a CPU reset: A20 on through the 8042
/// output port, so the reset fetch reaches the ROM alias at FFFFF0h, then
/// command FEh pulses the CPU reset line.
fn reset_program() -> Vec<u8> {
    vec![
        0xFA, 0xB0, 0xD1, 0xE6, 0x64, 0xE4, 0x64, 0xA8, 0x02, 0x75, 0xFA, 0xB0, 0xDF, 0xE6, 0x60,
        0xE4, 0x64, 0xA8, 0x02, 0x75, 0xFA, 0xB0, 0xFE, 0xE6, 0x64, 0xF4, 0xEB, 0xFD,
    ]
}

fn rows(assembled: &assembly::AssembledMachine) -> Vec<String> {
    match assembled.capture_text(DeviceId::Mda).unwrap().frame {
        video::Frame::Text(grid) => grid.rows_as_strings(),
        _ => panic!("MDA text required"),
    }
}

fn record_at_1mib(assembled: &assembly::AssembledMachine) -> [u8; 32] {
    let mut bytes = [0; 32];
    assembled
        .machine
        .cpu()
        .read_mem(VDISK_BASE, &mut bytes)
        .unwrap();
    bytes
}

fn run_step(assembled: &mut assembly::AssembledMachine, budget: u64) {
    let cap = assembled.machine.virtual_time() + budget;
    session::run_progressing(
        assembled,
        &assembly::RunPolicy::natural(Some(cap), Some(pc_harness::BIOS_IDLE_LIMIT)),
    )
    .unwrap();
}

/// Run until `ready` holds for the console, or fail after `budget`.
fn run_until(
    assembled: &mut assembly::AssembledMachine,
    budget: u64,
    what: &str,
    ready: impl Fn(&[String]) -> bool,
) {
    let deadline = assembled.machine.virtual_time() + budget;
    while !ready(&rows(assembled)) {
        assert!(
            assembled.machine.virtual_time() < deadline,
            "{what} not reached:\n{}",
            rows(assembled).join("\n")
        );
        run_step(assembled, 1_000_000);
    }
}

fn at_prompt_after(marker: &'static str) -> impl Fn(&[String]) -> bool {
    move |screen: &[String]| {
        let used: Vec<_> = screen.iter().filter(|row| !row.trim().is_empty()).collect();
        used.iter().any(|row| row.trim() == marker)
            && used.last().is_some_and(|row| row.trim() == "A:\\>")
    }
}

/// Type a DOS command, pacing keys so FreeCOM's line editor takes each one.
fn command(assembled: &mut assembly::AssembledMachine, text: &str) {
    for byte in text.bytes().chain(*b"\r") {
        let code = match byte {
            b'A' => 0x1E,
            b'E' => 0x12,
            b'L' => 0x26,
            b'N' => 0x31,
            b'P' => 0x19,
            b'R' => 0x13,
            b'S' => 0x1F,
            b'T' => 0x14,
            b'\r' => 0x1C,
            _ => panic!("unsupported key {byte}"),
        };
        assert!(assembled.type_scancodes(&[code, code | 0x80]));
        run_step(assembled, 100_000);
    }
}

fn plant(assembled: &mut assembly::AssembledMachine) {
    command(assembled, "PLANT");
    run_until(
        assembled,
        20_000_000,
        "VDISK record planted",
        at_prompt_after("VDISK-PLANTED"),
    );
    assert_eq!(
        record_at_1mib(assembled),
        kernel_vdisk_record(),
        "the planted record must reach 100000h"
    );
}

/// Reboot, wait for POST to clear the old console, then for DOS again.
fn reboot(assembled: &mut assembly::AssembledMachine, how: &str) -> [u8; 32] {
    run_until(
        assembled,
        100_000_000,
        "POST clearing the console",
        |screen| !screen.iter().any(|row| row.trim() == "VDISK-PLANTED"),
    );
    run_until(
        assembled,
        400_000_000,
        "the DOS prompt after a restart",
        at_prompt_after("READY"),
    );
    let record = record_at_1mib(assembled);
    eprintln!("after {how}: 100000h = {record:02x?}");
    record
}

#[test]
#[ignore = "slow: open AT POST and FreeDOS three times, restarting through Ctrl+Alt+Del and the 8042"]
fn no_vdisk_record_survives_a_restart() {
    assert!(include_str!("../../../Cargo.lock").contains(CPU_REV));
    let mut vendor = Vec::new();
    for relative in [
        "unemu-at-bios/bios-unemu-at.bin",
        "xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin",
        "hdd/fat16.img",
        "freedos/freedos.img",
    ] {
        let Some(bytes) = fixtures::fixture(relative) else {
            return;
        };
        vendor.push((relative, bytes));
    }
    // A system BIOS build to try in place of the pinned one, such as a
    // release candidate of the open AT BIOS.
    let candidate = std::env::var_os("UNEMUPC_OPEN_AT_BIOS");
    let mut image = vendor[3].1.clone();
    fat12::add_file(&mut image, b"PLANT   COM", &plant_program());
    fat12::add_file(&mut image, b"RESET   COM", &reset_program());
    fat12::add_file(&mut image, b"AUTOEXECBAT", b"@ECHO OFF\r\nECHO READY\r\n");
    // Copies of the shipped machine files and their fixtures, and a child
    // that adds the extended memory and replaces A:.
    let tree = machine_file::test_support::TempDir::new("xms-warm-boot");
    let write = |relative: &str, bytes: &[u8]| {
        let path = tree.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["at-freedos.toml", "at-ide.toml"] {
        write(
            &format!("machines/{name}"),
            &std::fs::read(repository.join("machines").join(name)).unwrap(),
        );
    }
    for (relative, bytes) in &vendor {
        write(&format!("vendor/{relative}"), bytes);
    }
    write("machines/local/xms-boot.img", &image);
    write(
        "machines/local/at-ide-1m.toml",
        b"schema = 1\nextends = \"../at-ide.toml\"\nname = \"at-ide-1m\"\n\n\
          [memory]\nextended_kib = 1024\n\n[[card]]\nid = \"fdc-765\"\n\n\
          [[card.drives]]\nunit = 0\nimage = \"xms-boot.img\"\nwrite = \"readonly\"\n",
    );
    let machine = tree.path().join("machines/local/at-ide-1m.toml");
    let mut config = machine_file::load(&machine).unwrap();
    if let Some(path) = &candidate {
        let bios = config
            .images
            .iter_mut()
            .find(|image| image.base == 0xF8000)
            .unwrap();
        bios.source = ImageSource::Inline(std::fs::read(path).unwrap());
    }
    let mut assembled = machine_file::assemble_with_observation_and_options(
        &config,
        &ObservationPolicy {
            routes: vec![ObservationRequest {
                device: DeviceId::Mda,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            }],
            ..ObservationPolicy::default()
        },
        AssemblyOptions {
            allow_unlisted_firmware: candidate.is_some(),
        },
    )
    .unwrap();
    run_until(
        &mut assembled,
        400_000_000,
        "the DOS prompt after power-on",
        at_prompt_after("READY"),
    );
    assert_ne!(&record_at_1mib(&assembled)[3..8], b"VDISK");

    plant(&mut assembled);
    // The window's Ctrl+Alt+Del: the whole chord, releases included.
    assert!(assembled.type_scancodes(&[0x1D, 0x38, 0x53, 0xD3, 0xB8, 0x9D]));
    let warm = reboot(&mut assembled, "Ctrl+Alt+Del");

    plant(&mut assembled);
    command(&mut assembled, "RESET");
    let reset = reboot(&mut assembled, "the 8042 CPU reset");

    assert_ne!(&warm[3..8], b"VDISK", "Ctrl+Alt+Del kept the VDISK record");
    assert_ne!(
        &reset[3..8],
        b"VDISK",
        "the 8042 reset kept the VDISK record"
    );
    assembled.finish_persistence().unwrap();
}
