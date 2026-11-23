// SPDX-License-Identifier: AGPL-3.0-or-later
//! Volatile BIOS/FreeDOS media leave vendor fixtures and their namespaces intact.
//! NEC µPD765 READ/WRITE DATA and Intel 8237A single-transfer mode are exercised
//! through firmware and DOS; see persistent-media.md §Implementation acceptance tests.
//!
//! Port 61h bits follow the PIT since 2026-11-11; delay loops take real refresh time.
//!

use cpu::Cpu;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use assembly::{
    ConsoleProbe, DeviceId, ImageSource, InjectionSchedule, KeyboardStimulus, ObservationPolicy,
    ObservationRequest, OutputKind, RunPolicy, StopCondition, WriteMode,
};
use machine_file::assemble_with_observation;

#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/session.rs"]
pub mod session;

const PINNED_SCRIPTS: &[(&str, &str, &str)] = &[
    (
        "kiselev-bios/bios-xi8088.bin",
        include_str!("../../../vendor/kiselev-bios/fetch.sh"),
        "OUTPUT_SHA256=\"",
    ),
    (
        "freedos/freedos.img",
        include_str!("../../../vendor/freedos/fetch.sh"),
        "check_sha256 \"$OUT\" \"",
    ),
    (
        "freedos/freedos-1200.img",
        include_str!("../../../vendor/freedos/fetch.sh"),
        "check_sha256 \"$OUT_1200\" \"",
    ),
    (
        "freedos/freedos-tools-1200.img",
        include_str!("../../../vendor/freedos/fetch.sh"),
        "check_sha256 \"$OUT_TOOLS\" \"",
    ),
    (
        "test386/test386.bin",
        include_str!("../../../vendor/test386/fetch.sh"),
        "check_sha256 \"$OUT\" \"",
    ),
    (
        "hdd/fat16.img",
        include_str!("../../../vendor/hdd/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    ),
    (
        "xtide-universal-bios/ide_at-r638-8k.bin",
        include_str!("../../../vendor/xtide-universal-bios/fetch.sh"),
        "OUTPUT_SHA256=\"",
    ),
    (
        "ltemm/LTEMM.EXE",
        include_str!("../../../vendor/ltemm/fetch.sh"),
        "OUTPUT_SHA256=\"",
    ),
    (
        "at-pm-test/at-pm-test-1200.img",
        include_str!("../../../vendor/at-pm-test/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    ),
    (
        "at-loadall-test/at-loadall-test-1200.img",
        include_str!("../../../vendor/at-loadall-test/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    ),
    (
        "vga-modex-test/vga-pas-test-1200.img",
        include_str!("../../../vendor/vga-modex-test/build.sh"),
        "check_sha256 \"$PAS_IMAGE\" \"",
    ),
    (
        "vga-modex-test/vga-modex-test-1200.img",
        include_str!("../../../vendor/vga-modex-test/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    ),
    (
        "at-privilege-test/at-privilege-test-1200.img",
        include_str!("../../../vendor/at-privilege-test/build.sh"),
        "check_sha256 \"$IMAGE\" \"",
    ),
];

use fixtures::parse_pin;

fn fixture_pins() -> Vec<(&'static str, &'static str)> {
    PINNED_SCRIPTS
        .iter()
        .map(|&(relative, script, prefix)| {
            let pin = parse_pin(script, prefix).unwrap_or_else(|error| {
                panic!("fixture {relative}: {error}; update the build-script pin parser")
            });
            (relative, pin)
        })
        .collect()
}

#[test]
fn fixture_output_pins_follow_build_script_format() {
    fixture_pins();
}

#[test]
fn fixture_pin_parser_rejects_script_format_drift() {
    let prefix = "OUTPUT_SHA256=\"";
    let hash = "a".repeat(64);
    let valid = format!("{prefix}{hash}\"");
    assert_eq!(parse_pin(&valid, prefix), Ok(hash.as_str()));
    for invalid in [
        String::new(),
        format!("UPSTREAM_SHA256=\"{hash}\""),
        format!("OUTPUT_SHA256={hash}"),
        format!("{prefix}{}\"", "a".repeat(63)),
        format!("{prefix}{}\"", "z".repeat(64)),
        format!("{valid}\n{valid}"),
    ] {
        assert!(parse_pin(&invalid, prefix)
            .unwrap_err()
            .contains("script format changed"));
    }
}

#[test]
fn freedos_floppy_fixtures_have_pinned_geometry_signature_and_hash() {
    let pins = fixture_pins();
    for (relative, size) in [
        ("freedos/freedos.img", 1_474_560),
        ("freedos/freedos-1200.img", 1_228_800),
        ("freedos/freedos-tools-1200.img", 1_228_800),
        ("vga-modex-test/vga-modex-test-1200.img", 1_228_800),
        ("vga-modex-test/vga-pas-test-1200.img", 1_228_800),
    ] {
        let Some(bytes) = fixtures::fixture(relative) else {
            continue;
        };
        assert_eq!(bytes.len(), size, "fixture size: {relative}");
        assert_eq!(
            &bytes[510..512],
            &[0x55, 0xAA],
            "boot signature: {relative}"
        );
        let hash: String = journal_files::Header::new(&bytes, 0)
            .unwrap()
            .base_digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let expected = pins.iter().find(|(path, _)| *path == relative).unwrap().1;
        assert_eq!(hash, expected, "fixture SHA-256: {relative}");
    }
}

fn entries(root: &Path, listing: &mut BTreeSet<PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        listing.insert(entry.path());
        if entry.file_type().unwrap().is_dir() {
            entries(&entry.path(), listing);
        }
    }
}

fn listing(vendor: &Path, checked: &[(&str, &str)]) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::new();
    entries(vendor, &mut paths);
    // Worktrees may symlink fixtures. Check their real directories too, without
    // traversing directory symlinks or assuming the checkout owns the targets.
    for &(relative, _) in checked {
        let target = vendor.join(relative).canonicalize().unwrap();
        entries(target.parent().unwrap(), &mut paths);
    }
    paths
}

fn assert_pinned(vendor: &Path, checked: &[(&str, &str)]) {
    for &(relative, expected) in checked {
        let bytes = std::fs::read(vendor.join(relative)).unwrap();
        let hash = assembly::test_support::fixture_sha256(&bytes);
        assert_eq!(hash, expected, "fixture SHA-256: {relative}");
    }
}

#[test]
#[ignore = "slow: BIOS POST, FreeDOS prompt and DOS writes; requires fetched boot fixtures"]
fn volatile_boots_preserve_pinned_fixture_hashes_and_vendor_entries() {
    let vendor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor");
    // The CI fixture gate fetches the boot/CPU inputs. Auxiliary ATA/option-ROM
    // images and the protected-mode, LOADALL, privilege-level and VGA test
    // floppies, which these boots do not use, remain optional, but a file
    // present initially must survive the run.
    let checked: Vec<_> = fixture_pins()
        .into_iter()
        .filter(|&(relative, _)| {
            !relative.starts_with("hdd/")
                && !relative.starts_with("xtide-")
                && !relative.starts_with("ltemm/")
                && !relative.starts_with("at-pm-test/")
                && !relative.starts_with("at-loadall-test/")
                && !relative.starts_with("at-privilege-test/")
                && !relative.starts_with("vga-modex-test/")
                || vendor.join(relative).symlink_metadata().is_ok()
        })
        .collect();
    let mut images = Vec::new();
    for &(relative, _) in &checked {
        let Some(bytes) = fixtures::fixture(relative) else {
            return;
        };
        images.push(bytes);
    }
    assert_pinned(&vendor, &checked);
    let before = listing(&vendor, &checked);
    for boot in ["bios", "freedos", "dos-write"] {
        let mut config = assembly::bios(
            images[0].clone(),
            (boot != "bios").then(|| images[1].clone()),
        );
        for drive in &mut config.drives {
            drive.image = ImageSource::File(vendor.join("freedos/freedos.img"));
            drive.write = WriteMode::Volatile;
        }
        let observation = ObservationPolicy {
            routes: vec![ObservationRequest {
                device: DeviceId::Mda,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            }],
            ..Default::default()
        };
        let mut assembled = assemble_with_observation(&config, &observation).unwrap();
        assert!(assembled.persistence.is_empty());
        let policy = if boot == "bios" {
            RunPolicy::natural(
                Some(pc_harness::BIOS_GOLDEN_MAX_RETIRED),
                Some(pc_harness::BIOS_IDLE_LIMIT),
            )
        } else {
            RunPolicy {
                retired_cap: Some(pc_harness::BIOS_MAX_RETIRED),
                idle_limit: Some(pc_harness::BIOS_IDLE_LIMIT),
                stop: StopCondition::PromptThen {
                    console: ConsoleProbe {
                        device: DeviceId::Mda,
                    },
                    needle: pc_harness::DOS_DATE_PROMPT.into(),
                    poll_step: pc_harness::DOS_INTERACTIVE_POLL_STEP,
                    then_budget: if boot == "dos-write" {
                        pc_harness::DOS_INTERACTIVE_COMMAND_BUDGET
                    } else {
                        0
                    },
                },
                stimuli: if boot == "dos-write" {
                    vec![KeyboardStimulus {
                        keyboard_index: assembled.device_index[&DeviceId::Keyboard],
                        scancodes: pc_harness::dos_write_file_script(),
                        schedule: InjectionSchedule::AfterPrompt,
                    }]
                } else {
                    Vec::new()
                },
            }
        };
        let outcome = assembled.run_session(&policy);
        eprintln!(
            "{boot}: stop={outcome:?}, cs={:04x}, ip={:04x}, retired={}",
            assembled.machine.cpu().code_segment(),
            assembled.machine.cpu().reg(cpu::CpuReg::Rip),
            assembled.machine.cpu().retired()
        );
        let video::Frame::Text(grid) = assembled.capture_text(DeviceId::Mda).unwrap().frame else {
            panic!("expected text frame")
        };
        eprintln!(
            "{boot}: {outcome:?}, virtual={}\n{}",
            assembled.machine.virtual_time(),
            grid.to_dump()
        );
        assert_eq!(outcome.unwrap(), machine::RunOutcome::RetiredCapReached);
        let console = grid.rows_as_strings().join("\n");
        let logical_hash = match boot {
            // The bare rig's 8042 selects 80286; the trap after POP DS changes
            // the open BIOS's CPU identification and adds its warning row.
            "bios" => 0x945fcce568e5d520,
            // BIOS century 20h gives FreeDOS Sat 01-01-2000 and stamps new files with year 00.
            "freedos" => 0xc46f0c147c881837,
            "dos-write" => 0x6b0404d06d7532c3,
            _ => unreachable!(),
        };
        assert_eq!(session::logical_hash(&console), logical_hash, "{boot}");

        assert!(
            console.contains(if boot == "bios" {
                "Booting OS"
            } else if boot == "freedos" {
                "Enter new date"
            } else {
                "ZZ"
            }),
            "{boot}: {console}"
        );
        assembled.finish_persistence().unwrap();
        drop(assembled);
        assert_pinned(&vendor, &checked);
        assert_eq!(
            listing(&vendor, &checked),
            before,
            "vendor entries changed during {boot}"
        );
    }
}

#[test]
fn ltemm_local_fixture_has_pinned_hash_and_mz_header() {
    let Some(bytes) = fixtures::optional_fixture("ltemm/LTEMM.EXE") else {
        return;
    };
    assert_eq!(&bytes[..2], b"MZ");
    let hash = assembly::test_support::fixture_sha256(&bytes);
    let pins = fixture_pins();
    assert_eq!(
        hash,
        pins.iter()
            .find(|(path, _)| *path == "ltemm/LTEMM.EXE")
            .unwrap()
            .1
    );
}

/// A corresponding-source archive that an open-firmware fixture script writes
/// next to its images.
struct SourceArchive {
    relative: &'static str,
    script: &'static str,
    /// Top-level entries under the archive's root directory.
    entries: &'static [&'static str],
    /// Members copied verbatim from this repository.
    copies: &'static [(&'static str, &'static str)],
    /// A member and text that identify the pinned revision or our patch.
    marker: (&'static str, &'static str),
    /// Upstream paths left out: prebuilt binaries or developer tooling.
    absent: &'static [&'static str],
}

const SOURCE_ARCHIVE_PIN: &str = "SOURCE_ARCHIVE_SHA256=\"";

const SOURCE_ARCHIVES: &[SourceArchive] = &[
    SourceArchive {
        relative: "kiselev-bios/kiselev-bios-a4af458-atreset-src.tar",
        script: include_str!("../../../vendor/kiselev-bios/fetch.sh"),
        entries: &["8088_bios", "NOTICE", "at-reset.patch"],
        copies: &[
            (
                "NOTICE",
                include_str!("../../../vendor/kiselev-bios/source-notice.txt"),
            ),
            (
                "at-reset.patch",
                include_str!("../../../vendor/kiselev-bios/at-reset.patch"),
            ),
        ],
        // The archived tree has the AT reset patch applied.
        marker: (
            "8088_bios/src/floppy2.inc",
            "je\tint_13_fn00\t\t; Reset disk system regardless of DL",
        ),
        absent: &["8088_bios/binaries", "8088_bios/xtide_r625"],
    },
    SourceArchive {
        relative: "vgabios/vgabios-0.9d-unemu.1-src.tar",
        script: include_str!("../../../vendor/vgabios/fetch.sh"),
        entries: &["NOTICE", "vgabios"],
        copies: &[(
            "NOTICE",
            include_str!("../../../vendor/vgabios/source-notice.txt"),
        )],
        // The fork's teletype BEL tone, which its 0.8a-based releases lacked.
        marker: ("vgabios/vgabios.c", "static void vga_beep()"),
        absent: &["vgabios/.beads/config.yaml"],
    },
    SourceArchive {
        relative: "unemu-at-bios/unemu-at-bios-1.0.2-unemu.3-src.tar",
        script: include_str!("../../../vendor/unemu-at-bios/fetch.sh"),
        entries: &["NOTICE", "unemu-at-bios"],
        copies: &[(
            "NOTICE",
            include_str!("../../../vendor/unemu-at-bios/source-notice.txt"),
        )],
        // The release's own version string, which the banner prints.
        marker: (
            "unemu-at-bios/src/config.inc",
            "%define VERSION\t\t'1.0.2-unemu.3'",
        ),
        absent: &["unemu-at-bios/binaries", "unemu-at-bios/xtide_r625"],
    },
    SourceArchive {
        relative: "xtide-universal-bios/xtide-universal-bios-r638-src.tar",
        script: include_str!("../../../vendor/xtide-universal-bios/fetch.sh"),
        entries: &["NOTICE", "trunk"],
        copies: &[(
            "NOTICE",
            include_str!("../../../vendor/xtide-universal-bios/source-notice.txt"),
        )],
        // The revision string the ROM's sign-on embeds.
        marker: ("trunk/XTIDE_Universal_BIOS/Inc/Revision.inc", "638 "),
        absent: &[],
    },
];

struct TarMember {
    path: String,
    kind: u8,
    mode: u64,
    owner: (u64, u64),
    owner_names: (String, String),
    mtime: u64,
    data: Vec<u8>,
}

fn tar_text(field: &[u8]) -> String {
    let end = field
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(field.len());
    String::from_utf8(field[..end].to_vec()).expect("UTF-8 tar header field")
}

fn tar_number(field: &[u8]) -> u64 {
    u64::from_str_radix(tar_text(field).trim(), 8).expect("octal tar header field")
}

/// Members of a POSIX ustar archive (POSIX.1, pax utility, "ustar Interchange
/// Format"): 512-byte headers, each followed by its data padded to 512 bytes,
/// ended by zero blocks.
fn ustar_members(archive: &[u8]) -> Vec<TarMember> {
    assert_eq!(
        archive.len() % 512,
        0,
        "tar archives are whole 512-byte blocks"
    );
    let mut members = Vec::new();
    let mut offset = 0;
    loop {
        let header = &archive[offset..offset + 512];
        if header.iter().all(|&byte| byte == 0) {
            return members;
        }
        assert_eq!(&header[257..265], b"ustar\x0000", "ustar magic and version");
        let name = tar_text(&header[..100]);
        let prefix = tar_text(&header[345..500]);
        let size = usize::try_from(tar_number(&header[124..136])).unwrap();
        let start = offset + 512;
        members.push(TarMember {
            path: if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            },
            kind: header[156],
            mode: tar_number(&header[100..108]),
            owner: (tar_number(&header[108..116]), tar_number(&header[116..124])),
            owner_names: (tar_text(&header[265..297]), tar_text(&header[297..329])),
            mtime: tar_number(&header[136..148]),
            data: archive[start..start + size].to_vec(),
        });
        offset = start + size.div_ceil(512) * 512;
    }
}

#[test]
fn source_archive_pins_follow_build_script_format() {
    for archive in SOURCE_ARCHIVES {
        parse_pin(archive.script, SOURCE_ARCHIVE_PIN)
            .unwrap_or_else(|error| panic!("source archive {}: {error}", archive.relative));
    }
}

#[test]
#[ignore = "requires fetched source archives"]
fn source_archives_match_pins_and_carry_their_notice() {
    for archive in SOURCE_ARCHIVES {
        let Some(bytes) = fixtures::fixture(archive.relative) else {
            continue;
        };
        let relative = archive.relative;
        assert_eq!(
            assembly::test_support::fixture_sha256(&bytes),
            parse_pin(archive.script, SOURCE_ARCHIVE_PIN).unwrap(),
            "source archive SHA-256: {relative}"
        );
        let root = Path::new(relative).file_stem().unwrap().to_str().unwrap();
        let members = ustar_members(&bytes);
        let mut entries = BTreeSet::new();
        for member in &members {
            let path = &member.path;
            let inner = path
                .strip_prefix(root)
                .and_then(|inner| inner.strip_prefix('/'))
                .unwrap_or_else(|| panic!("{relative}: {path} is outside {root}/"));
            entries.extend(inner.split('/').next().filter(|entry| !entry.is_empty()));
            // A source archive carries no prebuilt firmware images.
            let lowercase = path.to_ascii_lowercase();
            assert!(
                !lowercase.ends_with(".bin") && !lowercase.ends_with(".rom"),
                "{relative}: {path} is a binary image"
            );
            for &absent in archive.absent {
                let excluded = inner == absent
                    || inner
                        .strip_prefix(absent)
                        .is_some_and(|rest| rest.starts_with('/'));
                assert!(!excluded, "{relative}: {path} should be left out");
            }
            // Nothing from the host that wrote the archive: no owner, no
            // per-file times, only plain files and directories.
            assert!(
                matches!(member.kind, b'0' | b'5'),
                "{relative}: {path} type"
            );
            assert!(
                matches!(member.mode, 0o644 | 0o755),
                "{relative}: {path} mode"
            );
            assert_eq!(member.owner, (0, 0), "{relative}: {path} owner");
            assert_eq!(
                member.owner_names,
                (String::new(), String::new()),
                "{relative}: {path} owner names"
            );
            assert_eq!(member.mtime, members[0].mtime, "{relative}: {path} mtime");
        }
        assert_eq!(
            entries,
            archive.entries.iter().copied().collect(),
            "{relative}: top-level entries"
        );
        let member = |inner: &str| {
            let path = format!("{root}/{inner}");
            let found = members.iter().find(|member| member.path == path);
            let found = found.unwrap_or_else(|| panic!("{relative}: no {path}"));
            String::from_utf8_lossy(&found.data).into_owned()
        };
        for &(inner, expected) in archive.copies {
            assert_eq!(member(inner), expected, "{relative}: {inner}");
        }
        let (inner, text) = archive.marker;
        assert!(
            member(inner).contains(text),
            "{relative}: {inner} lacks {text:?}"
        );
    }
}
