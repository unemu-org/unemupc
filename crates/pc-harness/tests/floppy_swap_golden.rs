// SPDX-License-Identifier: AGPL-3.0-or-later
//! FreeDOS observes an AT disk change at a scheduled virtual-time boundary.
//! IBM PC AT Technical Reference, Diskette Drive Adapter, Digital Input
//! Register (3F7h); Microsoft FAT Specification, BPB and FAT12 directory entries.

use assembly::{
    DeviceId, FloppyAction, FloppyEvent, FloppyMediaList, FloppyMedium, ImageSource,
    ObservationPolicy, ObservationRequest, OutputKind, RunPolicy,
};
use video::Frame;

#[path = "support/fixtures.rs"]
mod fixtures;

fn second_disk() -> Vec<u8> {
    let mut image = vec![0; 1_474_560];
    image[..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
    image[3..11].copy_from_slice(b"UNEMU   ");
    for (offset, value) in [
        (11, 512u16),
        (14, 1),
        (17, 224),
        (19, 2880),
        (22, 9),
        (24, 18),
        (26, 2),
    ] {
        image[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    image[13] = 1;
    image[16] = 2;
    image[21] = 0xf0;
    image[38] = 0x29;
    image[39..43].copy_from_slice(&0x12345678u32.to_le_bytes());
    image[43..54].copy_from_slice(b"SECOND     ");
    image[54..62].copy_from_slice(b"FAT12   ");
    image[510..512].copy_from_slice(&[0x55, 0xaa]);
    for offset in [512, 10 * 512] {
        image[offset..offset + 6].copy_from_slice(&[0xf0, 0xff, 0xff, 0xff, 0x0f, 0]);
    }
    let root = 19 * 512;
    image[root..root + 11].copy_from_slice(b"SECOND     ");
    image[root + 11] = 8;
    let file = root + 32;
    image[file..file + 11].copy_from_slice(b"SWAPPED TXT");
    image[file + 11] = 0x20;
    image[file + 24..file + 26].copy_from_slice(&0x2821u16.to_le_bytes());
    image[file + 26..file + 28].copy_from_slice(&2u16.to_le_bytes());
    image[file + 28..file + 32].copy_from_slice(&6u32.to_le_bytes());
    image[33 * 512..33 * 512 + 6].copy_from_slice(b"hello\n");
    image
}

#[ignore = "slow: boots FreeDOS then swaps a floppy through a scheduled host input"]
#[test]
fn freedos_dir_observes_scheduled_second_floppy() {
    assert!(include_str!("../../../Cargo.lock")
        .contains("rev=865a0aa414cbf1325ef5a9383dfd00ef2e31cfae"));
    let (Some(_), Some(_)) = (
        fixtures::fixture("kiselev-bios/bios-xi8088.bin"),
        fixtures::fixture("freedos/freedos.img"),
    ) else {
        return;
    };
    let mut config = machine_file::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../machines/at-freedos.toml"),
    )
    .unwrap();
    let drive = &mut config.drives[0];
    drive.media = Some(FloppyMediaList {
        empty: false,
        entries: vec![
            FloppyMedium {
                image: drive.image.clone(),
                expected_sha256: drive.expected_sha256,
                write: assembly::WriteMode::ReadOnly,
                name: "boot.img".into(),
            },
            FloppyMedium {
                image: ImageSource::Inline(second_disk()),
                expected_sha256: None,
                write: assembly::WriteMode::ReadOnly,
                name: "second.img".into(),
            },
        ],
        inserted: 0,
    });
    let policy = ObservationPolicy {
        routes: vec![ObservationRequest {
            device: DeviceId::Mda,
            output: OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    };
    let mut assembled = machine_file::assemble_with_observation(&config, &policy).unwrap();
    let keyboard = assembled.device_index[&DeviceId::Keyboard];
    let boot = RunPolicy {
        retired_cap: Some(pc_harness::BIOS_MAX_RETIRED),
        idle_limit: Some(pc_harness::BIOS_IDLE_LIMIT),
        stop: assembly::StopCondition::PromptThen {
            console: assembly::ConsoleProbe {
                device: DeviceId::Mda,
            },
            needle: "Enter new date".into(),
            poll_step: pc_harness::DOS_INTERACTIVE_POLL_STEP,
            then_budget: pc_harness::DOS_INTERACTIVE_COMMAND_BUDGET,
        },
        stimuli: vec![assembly::KeyboardStimulus {
            keyboard_index: keyboard,
            scancodes: pc_harness::dos_dir_script(),
            schedule: assembly::InjectionSchedule::AfterPrompt,
        }],
    };
    // The first DIR primes FreeDOS's directory cache. Without disk change the
    // second DIR reuses that listing even though the new medium is readable.
    assembled.run_session(&boot).unwrap();
    let at = 14_000_000;
    assert!(assembled.machine.virtual_time() < at);
    assembled
        .run_floppy_schedule(
            at,
            Some(pc_harness::BIOS_IDLE_LIMIT),
            &[FloppyEvent {
                at,
                controller: DeviceId::Fdc,
                unit: 0,
                action: FloppyAction::Insert(1),
            }],
        )
        .unwrap();
    assert_eq!(assembled.machine.virtual_time(), at);
    // DIR A: (colon is shifted semicolon in the BIOS set-1 table).
    assembly::inject_scancodes(
        &mut assembled.machine,
        keyboard,
        &[
            0x20, 0xa0, 0x17, 0x97, 0x13, 0x93, 0x39, 0xb9, 0x1e, 0x9e, 0x2a, 0x27, 0xa7, 0xaa,
            0x1c, 0x9c,
        ],
    );
    assembled
        .run_floppy_schedule(at + 3_000_000, Some(pc_harness::BIOS_IDLE_LIMIT), &[])
        .unwrap();
    let capture = assembled.capture_text(DeviceId::Mda).unwrap();
    let Frame::Text(grid) = capture.frame else {
        panic!("text console");
    };
    let text = grid.to_dump();
    let hash = text.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    eprintln!("swap screen hash: {hash:016x}\n{text}");
    assert!(
        text.contains("SWAPPED"),
        "second floppy's known file must reach DOS DIR"
    );
    assert!(text.contains("SECOND"));
    assert_eq!(hash, 0xd686853413a29fcf);
    assembled.finish_persistence().unwrap();
}
