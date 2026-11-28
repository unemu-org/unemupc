// SPDX-License-Identifier: AGPL-3.0-or-later
//! Lo-tech 2MB EMS Board schematic, revision r02, HCT573 page registers and SRAM.
//! Open AT firmware and FreeDOS MEM exercise a fetched local-only driver.
//! LIM Expanded Memory Specification 4.0, INT 67h AH=41h/43h/44h/45h;
//! Intel 80286 Programmer's Reference Manual, Appendix A, Jcc and JMP, and
//! Appendix B, CALL (far, indirect) and PUSH (immediate).
//! See machines/README.md §Playing DOS games. CPU revision:
//! 38377ddcd34022654893e7cbad46b84c72159c64.

use assembly::{
    ConsoleProbe, DeviceConfig, DeviceId, DeviceKind, ImageSource, LotechEmsCfg, MmioWindow,
    ObservationPolicy, ObservationRequest, OutputKind, RunPolicy, StopCondition,
};
use cpu::Cpu;
use device::{Device, DeviceError, NextEvent, VClock};
use video::Frame;

#[path = "support/fat12.rs"]
mod fat12;
#[path = "support/fixtures.rs"]
pub mod fixtures;
#[path = "support/host_profile.rs"]
mod host_profile;

// Re-pinned when EMSCHECK gained the routine it runs from an EMS page. The
// screen is unchanged. The larger program alone moves the endpoint: the
// aliased-page mutant, which fails before reaching that code, moves equally.
const GOLDEN: (u64, u64) = (0x0eda968da7052433, 18_681_761);

// Deliberately alias every physical page to page zero as a negative control.
// It wraps the assembled board, so the frame map follows the aliased latches:
// windows selecting page zero together are trapped, and EMSCHECK's page check
// fails before it runs any code from the frame.
struct AliasedPages(Box<dyn Device>);
impl Device for AliasedPages {
    fn name(&self) -> &'static str {
        "aliased-ems-test"
    }
    fn port_read(&mut self, port: u16, size: u8) -> Result<u32, DeviceError> {
        self.0.port_read(port, size)
    }
    fn port_write(&mut self, port: u16, size: u8, value: u32) -> Result<(), DeviceError> {
        self.0
            .port_write(port, size, if value < 128 { 0 } else { value })
    }
    fn mmio_read(&mut self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        self.0.mmio_read(addr, size)
    }
    fn mmio_write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), DeviceError> {
        self.0.mmio_write(addr, size, value)
    }
    fn tick(&mut self, now: VClock) {
        self.0.tick(now);
    }
    fn next_event(&self, _: VClock) -> NextEvent {
        NextEvent::Dormant
    }
    fn irq_pending(&self) -> bool {
        false
    }
    fn reset(&mut self) {
        self.0.reset();
    }
    fn save(&self) -> Vec<u8> {
        self.0.save()
    }
    fn restore(&mut self, blob: &[u8]) -> Result<(), DeviceError> {
        self.0.restore(blob)
    }
}

fn page_check_program() -> Vec<u8> {
    let mut code = Vec::new();
    let mut labels = std::collections::BTreeMap::new();
    let mut branches = Vec::new();
    // The 286 has short conditional jumps; invert the condition over a near JMP.
    let branch_ne =
        |code: &mut Vec<u8>, branches: &mut Vec<(usize, &'static str)>, label: &'static str| {
            code.extend([0x74, 3, 0xE9, 0, 0]);
            branches.push((code.len() - 2, label));
        };
    // INT 67h: get frame segment, allocate all 128 logical pages.
    code.extend([0xB4, 0x41, 0xCD, 0x67, 0x84, 0xE4]);
    branch_ne(&mut code, &mut branches, "failed");
    code.extend([0x8E, 0xC3, 0xBB, 128, 0, 0xB4, 0x43, 0xCD, 0x67, 0x84, 0xE4]);
    branch_ne(&mut code, &mut branches, "failed");
    code.extend([0x89, 0xD6, 0x31, 0xED]);
    labels.insert("fill", code.len());
    // Spread logical pages across all four physical windows and latches.
    for (label, verifying) in [("fill", false), ("verify", true)] {
        if verifying {
            code.extend([0x31, 0xED]);
            labels.insert(label, code.len());
        }
        code.extend([
            0x89, 0xEB, 0x89, 0xF2, 0x89, 0xE8, 0x24, 3, 0xB4, 0x44, 0xCD, 0x67, 0x84, 0xE4,
        ]);
        branch_ne(&mut code, &mut branches, "failed-free");
        code.extend([0x89, 0xEF, 0x83, 0xE7, 3, 0xB1, 14, 0xD3, 0xE7]);
        if verifying {
            code.extend([0x26, 0x39, 0x2D]);
            branch_ne(&mut code, &mut branches, "failed-free");
        } else {
            code.extend([0x89, 0xE8, 0x26, 0x89, 5]);
        }
        code.extend([0x45, 0x81, 0xFD, 128, 0]);
        branch_ne(&mut code, &mut branches, label);
    }
    // Run code from an EMS page: copy a routine in through window 1, move the
    // page out to window 2 and far-call it there. MOV AX, CS:[0];
    // ADD AX, 1230h; RETF reads the page's fill word, its logical number 5.
    let map = |code: &mut Vec<u8>, branches: &mut Vec<_>, logical: u8, window: u8| {
        code.extend([
            0x89, 0xF2, 0xBB, logical, 0, 0xB8, window, 0x44, 0xCD, 0x67, 0x84, 0xE4,
        ]);
        branch_ne(code, branches, "failed-free");
    };
    map(&mut code, &mut branches, 5, 1);
    for (offset, word) in [
        (0x4100u16, 0xA12Eu16),
        (0x4102, 0),
        (0x4104, 0x3005),
        (0x4106, 0xCB12),
    ] {
        code.extend([0x26, 0xC7, 0x06]);
        code.extend(offset.to_le_bytes());
        code.extend(word.to_le_bytes());
    }
    map(&mut code, &mut branches, 6, 1);
    map(&mut code, &mut branches, 5, 2);
    // MOV AX, ES; ADD AX, 800h; PUSH AX; PUSH 100h; MOV BP, SP;
    // CALL FAR [BP]; ADD SP, 4; CMP AX, 1235h.
    code.extend([
        0x8C, 0xC0, 0x05, 0x00, 0x08, 0x50, 0x68, 0x00, 0x01, 0x89, 0xE5, 0xFF, 0x5E, 0x00, 0x83,
        0xC4, 0x04, 0x3D, 0x35, 0x12,
    ]);
    branch_ne(&mut code, &mut branches, "failed-free");
    // Release the handle before MEM observes free pages and handles.
    code.extend([0x89, 0xF2, 0xB4, 0x45, 0xCD, 0x67, 0x84, 0xE4]);
    branch_ne(&mut code, &mut branches, "failed");
    code.extend([0xB8, 0, 0x4C, 0xCD, 0x21]);
    labels.insert("failed-free", code.len());
    code.extend([0x89, 0xF2, 0xB4, 0x45, 0xCD, 0x67]);
    labels.insert("failed", code.len());
    code.extend([0xB8, 1, 0x4C, 0xCD, 0x21]);
    for (at, label) in branches {
        let displacement = (labels[label] as i16).wrapping_sub((at + 2) as i16);
        code[at..at + 2].copy_from_slice(&displacement.to_le_bytes());
    }
    code
}

fn screen_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

fn capture(config: &assembly::MachineConfig, broken: bool) -> (String, u64) {
    let observation = ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::Mda,
            output: OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    };
    let mut rig = machine_file::assemble_with_observation(config, &observation).unwrap();
    if broken {
        let index = rig.device_index[&DeviceId::LotechEms];
        let slot = rig.machine.bus_mut().device_mut(index).unwrap();
        let spare: Box<dyn Device> =
            Box::new(devices::LotechEms::new(0x260, 0xD000, 2048).unwrap());
        let board = std::mem::replace(slot, spare);
        *slot = Box::new(AliasedPages(board));
    }
    let _profile =
        host_profile::WorkloadProfile::from_env(&mut rig, || format!("lotech ems broken={broken}"));
    rig.run_session(&RunPolicy {
        retired_cap: Some(80_000_000),
        idle_limit: Some(pc_harness::BIOS_IDLE_LIMIT),
        stop: StopCondition::PromptThen {
            console: ConsoleProbe {
                device: DeviceId::Mda,
            },
            needle: "A:\\>".into(),
            poll_step: 100_000,
            then_budget: 0,
        },
        stimuli: vec![],
    })
    .unwrap();
    let Frame::Text(grid) = rig.capture_text(DeviceId::Mda).unwrap().frame else {
        panic!("expected text");
    };
    (grid.to_dump(), rig.machine.cpu().retired())
}

#[test]
#[ignore = "slow: boots open AT firmware and FreeDOS MEM /E, including an aliased-page mutation"]
fn freedos_mem_reports_lotech_expanded_memory() {
    let Some(driver) = fixtures::optional_fixture("ltemm/LTEMM.EXE") else {
        return;
    };
    assert!(
        include_str!("../../../Cargo.lock").contains("#38377ddcd34022654893e7cbad46b84c72159c64")
    );
    for path in [
        "unemu-at-bios/bios-unemu-at.bin",
        "freedos/freedos.img",
        "xtide-universal-bios/ide_xtp-r638-ata1f0-8k.bin",
        "hdd/fat16.img",
    ] {
        if fixtures::fixture(path).is_none() {
            return;
        }
    }
    let Some(mut image) = fixtures::fixture("freedos/freedos-tools-1200.img") else {
        return;
    };
    let driver_pin = fixtures::parse_pin(
        include_str!("../../../vendor/ltemm/fetch.sh"),
        "OUTPUT_SHA256=\"",
    )
    .unwrap();
    let tools_pin = fixtures::parse_pin(
        include_str!("../../../vendor/freedos/fetch.sh"),
        "check_sha256 \"$OUT_TOOLS\" \"",
    )
    .unwrap();
    assert_eq!(assembly::test_support::fixture_sha256(&driver), driver_pin);
    assert_eq!(assembly::test_support::fixture_sha256(&image), tools_pin);
    fat12::add_file(&mut image, b"LTEMM   EXE", &driver);
    fat12::add_file(
        &mut image,
        b"CONFIG  SYS",
        b"FILES=20\r\nBUFFERS=10\r\nDEVICE=LTEMM.EXE /p:D000 /n\r\nSHELL=COMMAND.COM /P /E:256\r\n",
    );
    fat12::add_file(&mut image, b"EMSCHECKCOM", &page_check_program());
    fat12::add_file(&mut image, b"AUTOEXECBAT", b"@ECHO OFF\r\nCLS\r\nEMSCHECK.COM\r\nIF ERRORLEVEL 1 GOTO FAILED\r\nMEM /E\r\nECHO EMS-CHECK-DONE\r\nGOTO END\r\n:FAILED\r\nECHO EMS-CHECK-FAILED\r\n:END\r\n");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut config = machine_file::load(root.join("machines/at-ide.toml")).unwrap();
    let drive = config
        .drives
        .iter_mut()
        .find(|d| d.controller == DeviceId::Fdc && d.unit == 0)
        .unwrap();
    drive.image = ImageSource::Inline(image);
    drive.media = None;
    drive.expected_sha256 = None;
    drive.kind = assembly::DriveKind::Floppy {
        geometry: assembly::FloppyGeometry::HD_1_2,
    };
    drive.write = assembly::WriteMode::Volatile;
    let cfg = LotechEmsCfg {
        frame: 0xD000,
        ..Default::default()
    };
    config.devices.push(DeviceConfig::new(
        DeviceId::LotechEms,
        DeviceKind::LotechEms(cfg),
        cfg.port_windows(),
    ));
    config
        .memory
        .mmio
        .push(MmioWindow::new(0xD0000, 0x10000, DeviceId::LotechEms));
    let (text, retired) = capture(&config, false);
    // Only FreeDOS MEM's output is inspected; the driver banner is never stored.
    eprintln!(
        "EMS golden retired={retired} hash={:016x}",
        screen_hash(&text)
    );
    for row in text.lines().filter(|row| {
        row.starts_with("Total Expanded (EMS)")
            || row.starts_with("Free Expanded (EMS)")
            || row.trim_start().starts_with("Free EMS memory")
    }) {
        eprintln!("MEM: {row}");
    }
    assert!(
        text.contains("EMS-CHECK-DONE"),
        "DOS page checks did not finish"
    );
    for prefix in ["Total Expanded (EMS)", "Free Expanded (EMS)"] {
        let row = text
            .lines()
            .find(|row| row.starts_with(prefix))
            .expect("FreeDOS MEM EMS row");
        assert!(
            row.contains("2,048K (2,097,152 bytes)"),
            "unexpected EMS size: {row}"
        );
    }
    assert!(text
        .lines()
        .any(|row| row.trim() == "Free EMS memory     2,097,152 bytes"));
    let (mutant, mutant_retired) = capture(&config, true);
    eprintln!(
        "EMS mutant retired={mutant_retired} hash={:016x}",
        screen_hash(&mutant)
    );
    assert!(
        mutant.contains("EMS-CHECK-FAILED"),
        "mutation must fail the DOS page check"
    );
    assert_ne!(
        (screen_hash(&mutant), mutant_retired),
        GOLDEN,
        "aliased page mutation must fail the golden"
    );
    assert_eq!((screen_hash(&text), retired), GOLDEN);
}
