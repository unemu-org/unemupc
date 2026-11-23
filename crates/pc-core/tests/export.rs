// SPDX-License-Identifier: AGPL-3.0-or-later
//! Offline PC exports use current readers without assembling or opening firmware.

use pc_config::{image::Digest, launch};
use unemu_core_api::{test_support::MemoryExportSink, *};
use unemu_pc_core::PcCore;

fn config() -> Vec<u8> {
    let text = format!(
        r#"schema = 1
platform = "ibm-pc"
[machine]
schema = 1
[machine.board]
model = "ibm-5170"
[machine.firmware.bios]
image = "assets/bios.bin"
sha256 = "{bios}"
provides = ["fixed-disk", "floppy"]
[[machine.card]]
kind = "fdc-765"
[[machine.card.drives]]
unit = 0
type = "1.44M"
[[machine.card]]
kind = "ide"
[[machine.card.drives]]
channel = "master"
geometry = {{ cylinders = 2, heads = 2, sectors = 3 }}
[[firmware]]
key = "firmware.bios"
source = "assets/bios.bin"
blob = "sha256:{bios}"
[[media]]
id = "disk"
source = "assets/disk.img"
head = "sha256:{disk}"
policy = "readonly"
geometry = {{ cylinders = 2, heads = 2, sectors = 3, sector_bytes = 512 }}
[[slots]]
controller = "fdc-765"
unit = 0
catalogue = []
catalogue_mode = true
[[slots]]
controller = "ide"
unit = 0
catalogue = ["disk"]
catalogue_mode = false
inserted = "disk"
[[content]]
digest = "sha256:{bios}"
sha256 = "{bios}"
size = 65536
[[content]]
digest = "sha256:{disk}"
sha256 = "{disk}"
size = 6144
"#,
        bios = Digest::of(b"firmware identity").hex(),
        disk = Digest::of(b"base identity").hex(),
    );
    launch::decode(text.as_bytes()).unwrap();
    text.into_bytes()
}
fn document(bytes: &[u8]) -> ConfigDocument<'_> {
    ConfigDocument::new(launch::MEDIA_TYPE, bytes)
}
struct Bytes(Vec<u8>);
impl ContentReader for Bytes {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        assert!(buf.len() <= 65536, "disk reads must stay bounded");
        buf.copy_from_slice(&self.0[offset as usize..offset as usize + buf.len()]);
        Ok(())
    }
}

#[test]
fn offline_description_matches_slots_and_fixed_lengths() {
    let bytes = config();
    PcCore.validate("ibm-pc", document(&bytes)).unwrap();
    let media = PcCore.describe_media("ibm-pc", document(&bytes)).unwrap();
    assert_eq!(media.len(), 2);
    assert_eq!(media[0].slot.id, SlotId(0));
    assert_eq!(media[0].detail, "3.5-inch 1.44M");
    assert_eq!(media[0].size, None);
    assert_eq!(media[1].slot.id, SlotId(2));
    assert_eq!(media[1].detail, "CHS 2/2/3, 0 MiB");
    assert_eq!(media[1].size, Some(6144));
}

#[test]
fn raw_exports_current_bytes_and_receipts_deterministically() {
    let config = config();
    let reader = Bytes((0..6144).map(|n| n as u8).collect());
    let media = [ExportMedium::new(SlotId(2), reader.size(), &reader)];
    let mut first = MemoryExportSink::new();
    let report = PcCore
        .export(
            "ibm-pc",
            document(&config),
            ExportRequest::new("raw", &media),
            &mut first,
        )
        .unwrap();
    assert_eq!(first.outputs()["slot-2.img"], reader.0);
    assert_eq!(report.outputs[0].sha256, Digest::of(&reader.0).sha256());
    let mut second = MemoryExportSink::new();
    assert_eq!(
        PcCore
            .export(
                "ibm-pc",
                document(&config),
                ExportRequest::new("raw", &media),
                &mut second
            )
            .unwrap(),
        report
    );
    assert_eq!(first.outputs(), second.outputs());
}

#[test]
fn machine_export_references_relative_outputs_and_reports_losses() {
    let config = config();
    let reader = Bytes(vec![0xe5; 6144]);
    let media = [ExportMedium::new(SlotId(2), reader.size(), &reader)];
    let mut sink = MemoryExportSink::new();
    let report = PcCore
        .export(
            "ibm-pc",
            document(&config),
            ExportRequest::new("machine", &media),
            &mut sink,
        )
        .unwrap();
    let outputs = sink.outputs();
    let text = std::str::from_utf8(&outputs["machine.toml"]).unwrap();
    let machine = pc_config::parse_toml("machine.toml", text).unwrap();
    let firmware = machine.firmware.unwrap();
    assert!(firmware.bios.is_some());
    let expected_image = Digest::of(b"firmware identity").hex();
    for rom in firmware
        .bios
        .iter()
        .chain(firmware.option_roms.iter().flatten())
    {
        assert_eq!(rom.image.as_deref(), Some(expected_image.as_str()));
    }
    assert!(text.contains("slot-2.img"));
    assert!(text.contains(&Digest::of(&reader.0).hex()));
    assert!(text.contains("volatile"));
    assert!(!text.contains("assets/bios.bin"));
    assert_eq!(report.outputs.len(), 2);
    for receipt in &report.outputs {
        assert_eq!(receipt.size, outputs[&receipt.name].len() as u64);
        assert_eq!(receipt.sha256, Digest::of(&outputs[&receipt.name]).sha256());
    }
    assert!(report.notes.iter().any(|n| n.contains("readonly")));
}

// This parser uses the published offsets directly and recomputes the checksum
// with the checksum word excluded, independently of the export implementation.
struct Footer<'a>(&'a [u8]);
impl Footer<'_> {
    fn word(&self, offset: usize) -> u32 {
        u32::from_be_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }
    fn size(&self, offset: usize) -> u64 {
        u64::from_be_bytes(self.0[offset..offset + 8].try_into().unwrap())
    }
    fn checksum(&self) -> u32 {
        !self
            .0
            .iter()
            .enumerate()
            .filter(|(i, _)| !(64..68).contains(i))
            .map(|(_, &b)| u32::from(b))
            .sum::<u32>()
    }
}

#[test]
fn vhd_footer_matches_independent_parser_and_hand_computed_fixture() {
    let config = config();
    let reader = Bytes(vec![0; 6144]);
    let media = [ExportMedium::new(SlotId(2), reader.size(), &reader)];
    let mut first = MemoryExportSink::new();
    let report = PcCore
        .export(
            "ibm-pc",
            document(&config),
            ExportRequest::new("vhd-fixed", &media),
            &mut first,
        )
        .unwrap();
    let outputs = first.outputs();
    let image = &outputs["slot-2.vhd"];
    assert_eq!(&image[..6144], &reader.0);
    assert_eq!(image.len(), 6656);
    let f = Footer(&image[6144..]);
    assert_eq!(&f.0[..8], b"conectix");
    assert_eq!(f.word(8), 2);
    assert_eq!(f.word(12), 0x10000);
    assert_eq!(f.size(16), u64::MAX);
    assert_eq!(f.word(24), 0);
    assert_eq!(&f.0[28..32], b"upc ");
    assert_eq!(f.word(32), 0x10000);
    assert_eq!(&f.0[36..40], b"Wi2k");
    assert_eq!(f.size(40), 6144);
    assert_eq!(f.size(48), 6144);
    assert_eq!(&f.0[56..60], &[0, 2, 2, 3]);
    assert_eq!(f.word(60), 2);
    assert_eq!(f.word(64), f.checksum());
    assert_eq!(f.0[84], 0);
    assert!(f.0[85..].iter().all(|&b| b == 0));
    // 6144 zero bytes have SHA-256 fd9243e1ba57263ed469c3bdbd7ade6e...
    // Summing the fields by hand with the checksum word zeroed:
    // 861 + 2 + 1 + 2040 + 360 + 1 + 349 + 48 + 7 + 2 + 2440 = 6111.
    // The one's complement of 0x17df is 0xffffe820.
    let prefix = "636f6e65637469780000000200010000ffffffffffffffff0000000075706320000100005769326b000000000000180000000000000018000002020300000002ffffe820fd9243e1ba57863e9469c3bdbd7ade6e";
    let expected: Vec<u8> = prefix
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .chain(std::iter::repeat_n(0, 428))
        .collect();
    assert_eq!(f.0, expected);
    assert_eq!(report.outputs[0].sha256, Digest::of(image).sha256());
    let mut second = MemoryExportSink::new();
    assert_eq!(
        PcCore
            .export(
                "ibm-pc",
                document(&config),
                ExportRequest::new("vhd-fixed", &media),
                &mut second
            )
            .unwrap(),
        report
    );
    assert_eq!(outputs, second.outputs());
    let changed = Bytes(vec![1; 6144]);
    let mut sink = MemoryExportSink::new();
    PcCore
        .export(
            "ibm-pc",
            document(&config),
            ExportRequest::new(
                "vhd-fixed",
                &[ExportMedium::new(SlotId(2), changed.size(), &changed)],
            ),
            &mut sink,
        )
        .unwrap();
    assert_ne!(
        &f.0[68..84],
        &sink.outputs()["slot-2.vhd"][6144 + 68..6144 + 84]
    );
}

#[test]
fn export_formats_and_slot_queries_are_canonical() {
    let config = config();
    let core: &dyn Core = &PcCore;
    let formats = &core.description().platforms[0].export_formats;
    assert_eq!(
        formats
            .iter()
            .map(|f| (f.id, f.extension, f.scope))
            .collect::<Vec<_>>(),
        [
            ("raw", "img", ExportScope::Slot),
            ("vhd-fixed", "vhd", ExportScope::Slot),
            ("machine", "toml", ExportScope::Machine)
        ]
    );
    for (format, expected) in [
        ("raw", vec![SlotId(0), SlotId(2)]),
        ("vhd-fixed", vec![SlotId(2)]),
        ("machine", vec![SlotId(2)]),
    ] {
        assert_eq!(
            core.export_slots("ibm-pc", document(&config), format)
                .unwrap(),
            expected
        );
    }
    assert_eq!(
        core.export_slots("ibm-pc", document(&config), "unknown")
            .unwrap_err()
            .kind,
        ErrorKind::Usage
    );
}

#[test]
fn bad_export_requests_fail_before_any_output_is_created() {
    let config = config();
    let disk = Bytes(vec![1; 6144]);
    let short = Bytes(vec![1; 512]);
    let valid = ExportMedium::new(SlotId(2), disk.size(), &disk);
    let unknown = ExportMedium::new(SlotId(8), disk.size(), &disk);
    let floppy = ExportMedium::new(SlotId(0), disk.size(), &disk);
    for (format, media, kind) in [
        ("unknown", vec![valid], ErrorKind::Usage),
        ("raw", vec![], ErrorKind::Usage),
        ("raw", vec![valid, valid], ErrorKind::Usage),
        ("raw", vec![unknown], ErrorKind::Usage),
        ("vhd-fixed", vec![floppy], ErrorKind::Usage),
        ("machine", vec![], ErrorKind::Usage),
        ("machine", vec![valid, valid], ErrorKind::Usage),
        ("machine", vec![unknown], ErrorKind::Usage),
        ("machine", vec![floppy], ErrorKind::Usage),
        (
            "raw",
            vec![ExportMedium::new(SlotId(2), 1, &disk)],
            ErrorKind::Content,
        ),
        (
            "raw",
            vec![ExportMedium::new(SlotId(2), short.size(), &short)],
            ErrorKind::Content,
        ),
    ] {
        let mut sink = MemoryExportSink::new();
        assert_eq!(
            PcCore
                .export(
                    "ibm-pc",
                    document(&config),
                    ExportRequest::new(format, &media),
                    &mut sink
                )
                .unwrap_err()
                .kind,
            kind
        );
        assert!(sink.outputs().is_empty());
    }
}

#[test]
fn offline_config_errors_agree_with_validate() {
    let config = config();
    for (platform, doc, kind) in [
        ("foreign", document(&config), ErrorKind::Unsupported),
        (
            "ibm-pc",
            ConfigDocument::new("text/plain", &config),
            ErrorKind::Unsupported,
        ),
        ("ibm-pc", document(b"invalid"), ErrorKind::Config),
    ] {
        assert_eq!(PcCore.validate(platform, doc).unwrap_err().kind, kind);
        assert_eq!(PcCore.describe_media(platform, doc).unwrap_err().kind, kind);
        assert_eq!(
            PcCore.export_slots(platform, doc, "raw").unwrap_err().kind,
            kind
        );
        let mut sink = MemoryExportSink::new();
        assert_eq!(
            PcCore
                .export(platform, doc, ExportRequest::new("raw", &[]), &mut sink)
                .unwrap_err()
                .kind,
            kind
        );
        assert!(sink.outputs().is_empty());
    }
}

#[test]
fn reader_and_sink_failures_never_return_partial_success() {
    use unemu_core_api::test_support::ExportOperation;
    struct Failed(ErrorKind);
    impl ContentReader for Failed {
        fn size(&self) -> u64 {
            6144
        }
        fn read_at(&self, _: u64, buf: &mut [u8]) -> Result<(), Error> {
            buf.fill(0xee);
            Err(Error::new(self.0, "reader failure"))
        }
    }
    let config = config();
    for kind in [ErrorKind::Usage, ErrorKind::Content, ErrorKind::Storage] {
        let mut sink = MemoryExportSink::new();
        let reader = Failed(kind);
        let media = [ExportMedium::new(SlotId(2), reader.size(), &reader)];
        assert_eq!(
            PcCore
                .export(
                    "ibm-pc",
                    document(&config),
                    ExportRequest::new("raw", &media),
                    &mut sink
                )
                .unwrap_err()
                .kind,
            if kind == ErrorKind::Usage {
                ErrorKind::Content
            } else {
                kind
            }
        );
        assert!(sink.outputs()["slot-2.img"].is_empty());
    }
    let reader = Bytes(vec![1; 6144]);
    let media = [ExportMedium::new(SlotId(2), reader.size(), &reader)];
    for operation in [
        ExportOperation::Create,
        ExportOperation::Write,
        ExportOperation::Finish,
    ] {
        let mut sink = MemoryExportSink::new();
        sink.fail_next(operation);
        assert_eq!(
            PcCore
                .export(
                    "ibm-pc",
                    document(&config),
                    ExportRequest::new("raw", &media),
                    &mut sink
                )
                .unwrap_err()
                .kind,
            ErrorKind::Storage
        );
    }
    let mut sink = MemoryExportSink::new();
    let output = sink.create("slot-2.img").unwrap();
    output.write(b"preserve this").unwrap();
    output.finish().unwrap();
    assert_eq!(
        PcCore
            .export(
                "ibm-pc",
                document(&config),
                ExportRequest::new("raw", &media),
                &mut sink
            )
            .unwrap_err()
            .kind,
        ErrorKind::Usage
    );
    assert_eq!(sink.outputs()["slot-2.img"], b"preserve this");
}

fn catalogue_config() -> Vec<u8> {
    use pc_config::image::{Geometry, Medium, Policy};
    let mut launch = launch::decode(&config()).unwrap();
    let slot = &mut launch.config.slots[0];
    slot.catalogue = vec!["other".into(), "inserted".into()];
    slot.inserted = Some("inserted".into());
    for (id, sectors) in [("other", 18), ("inserted", 9)] {
        let digest = Digest::of(id.as_bytes());
        launch.config.media.push(Medium {
            id: id.into(),
            source: format!("assets/{id}.img"),
            head: digest,
            name: Some(format!("Custom {id}")),
            policy: Policy::Volatile,
            geometry: Geometry {
                cylinders: 80,
                heads: 2,
                sectors,
                sector_bytes: 512,
            },
        });
        launch.content.push(launch::ContentRow {
            digest,
            sha256: digest.sha256(),
            size: 80 * 2 * u64::from(sectors) * 512,
        });
    }
    launch::encode(&launch).unwrap()
}

#[test]
fn floppy_types_describe_fixed_and_variable_lengths() {
    for (kind, detail, size) in [
        ("1.44M", "3.5-inch 1.44M", None),
        ("1.2M", "5.25-inch 1.2M", None),
        ("720K", "3.5-inch 720K", Some(737280)),
        ("360K", "5.25-inch 360K", Some(368640)),
    ] {
        let text = String::from_utf8(config()).unwrap().replace("1.44M", kind);
        let descriptions = PcCore
            .describe_media("ibm-pc", document(text.as_bytes()))
            .unwrap();
        assert_eq!(descriptions[0].detail, detail);
        assert_eq!(descriptions[0].size, size);
    }
    let bytes = catalogue_config();
    assert_eq!(
        PcCore.describe_media("ibm-pc", document(&bytes)).unwrap()[0].size,
        Some(737280)
    );
}

#[test]
fn machine_export_reports_catalogue_labels_records_nvram_and_provenance() {
    use pc_config::image::{Nvram, NvramMode, Policy};
    use toml::{Table, Value};
    let mut launch = launch::decode(&catalogue_config()).unwrap();
    let digest = Digest::of(b"CMOS identity");
    launch.config.nvram = Some(Nvram {
        mode: NvramMode::Journal,
        source: "assets/cmos.bin".into(),
        head: digest,
    });
    let mut nvram = Table::new();
    nvram.insert("image".into(), "assets/cmos.bin".into());
    let mut rtc = Table::new();
    rtc.insert("nvram".into(), nvram.into());
    launch
        .config
        .machine
        .get_mut("board")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("rtc".into(), rtc.into());
    launch.content.push(launch::ContentRow {
        digest,
        sha256: digest.sha256(),
        size: 64,
    });
    launch.config.media[0].policy = Policy::Journal;
    launch.config.firmware[0].source_archive = Some(Digest::of(b"source archive"));
    launch.records = vec![
        launch::RecordRow {
            medium: "disk".into(),
            name: "disk.upfj".into(),
            keep_generations: 3,
        },
        launch::RecordRow {
            medium: "cmos".into(),
            name: "cmos.upfj".into(),
            keep_generations: 5,
        },
    ];
    let text = launch::encode(&launch).unwrap();
    let disk = Bytes(vec![2; 6144]);
    let floppy = Bytes(vec![3; 737280]);
    let media = [
        ExportMedium::new(SlotId(2), disk.size(), &disk),
        ExportMedium::new(SlotId(0), floppy.size(), &floppy),
    ];
    let mut sink = MemoryExportSink::new();
    let report = PcCore
        .export(
            "ibm-pc",
            document(&text),
            ExportRequest::new("machine", &media),
            &mut sink,
        )
        .unwrap();
    let outputs = sink.outputs();
    let text = std::str::from_utf8(&outputs["machine.toml"]).unwrap();
    let parsed = pc_config::parse_toml("machine.toml", text).unwrap();
    assert_eq!(
        parsed.board.unwrap().rtc.unwrap().nvram,
        Some(pc_config::schema::v1::Nvram::Derived)
    );
    let table: Table = text.parse().unwrap();
    let floppy_entry = &table["card"].as_array().unwrap()[0]["drives"]
        .as_array()
        .unwrap()[0]["media"]
        .as_array()
        .unwrap()[0];
    assert_eq!(floppy_entry["image"].as_str(), Some("slot-0.img"));
    assert_eq!(
        floppy_entry["sha256"].as_str(),
        Some(Digest::of(&floppy.0).hex().as_str())
    );
    assert!(floppy_entry.as_table().unwrap().get("name").is_none());
    for loss in [
        "Custom inserted",
        "other",
        "keep_generations=3",
        "keep_generations=5",
        "NVRAM",
        "source_archive",
        "journal",
    ] {
        assert!(
            report.notes.iter().any(|n| n.contains(loss)),
            "missing loss note for {loss}"
        );
    }
    assert_eq!(
        report
            .outputs
            .iter()
            .map(|o| o.name.as_str())
            .collect::<Vec<_>>(),
        ["slot-0.img", "slot-2.img", "machine.toml"]
    );
    assert_eq!(outputs["slot-0.img"], floppy.0);
    assert_eq!(outputs["slot-2.img"], disk.0);
    let mut second = MemoryExportSink::new();
    let mut reverse = media;
    reverse.reverse();
    assert_eq!(
        PcCore
            .export(
                "ibm-pc",
                document(&launch::encode(&launch).unwrap()),
                ExportRequest::new("machine", &reverse),
                &mut second
            )
            .unwrap(),
        report
    );
    assert_eq!(second.outputs(), outputs);
    assert!(matches!(table["board"], Value::Table(_)));
}

#[test]
fn every_length_is_checked_before_creating_the_first_output() {
    let config = catalogue_config();
    let disk = Bytes(vec![2; 6144]);
    let bad_floppy = Bytes(vec![3; 512]);
    let media = [
        ExportMedium::new(SlotId(2), disk.size(), &disk),
        ExportMedium::new(SlotId(0), bad_floppy.size(), &bad_floppy),
    ];
    let mut sink = MemoryExportSink::new();
    assert_eq!(
        PcCore
            .export(
                "ibm-pc",
                document(&config),
                ExportRequest::new("machine", &media),
                &mut sink
            )
            .unwrap_err()
            .kind,
        ErrorKind::Content
    );
    assert!(sink.outputs().is_empty());
}

#[test]
fn disks_are_streamed_and_failed_later_chunks_are_not_emitted() {
    use std::cell::Cell;
    struct Reader {
        calls: Cell<usize>,
        fail: bool,
    }
    impl ContentReader for Reader {
        fn size(&self) -> u64 {
            98304
        }
        fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<(), Error> {
            assert!(bytes.len() <= 65536);
            self.calls.set(self.calls.get() + 1);
            bytes.fill((offset / 65536) as u8);
            if self.fail && offset != 0 {
                Err(Error::new(ErrorKind::Content, "second chunk failed"))
            } else {
                Ok(())
            }
        }
    }
    let text = String::from_utf8(config())
        .unwrap()
        .replace("cylinders = 2", "cylinders = 32")
        .replace("size = 6144", "size = 98304");
    for fail in [false, true] {
        let reader = Reader {
            calls: Cell::new(0),
            fail,
        };
        let media = [ExportMedium::new(SlotId(2), reader.size(), &reader)];
        let mut sink = MemoryExportSink::new();
        let result = PcCore.export(
            "ibm-pc",
            document(text.as_bytes()),
            ExportRequest::new("raw", &media),
            &mut sink,
        );
        assert_eq!(reader.calls.get(), 2);
        let outputs = sink.outputs();
        let bytes = &outputs["slot-2.img"];
        assert_eq!(&bytes[..65536], vec![0; 65536]);
        if fail {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Content);
            assert_eq!(bytes.len(), 65536);
        } else {
            assert!(result.is_ok());
            assert_eq!(&bytes[65536..], vec![1; 32768]);
        }
    }
}

#[test]
fn empty_machine_exports_just_a_loadable_document() {
    let mut launch = launch::decode(&config()).unwrap();
    launch
        .config
        .machine
        .insert("card".into(), Vec::<toml::Value>::new().into());
    launch.config.slots.clear();
    launch.config.media.clear();
    launch.content.pop();
    let bytes = launch::encode(&launch).unwrap();
    assert!(PcCore
        .describe_media("ibm-pc", document(&bytes))
        .unwrap()
        .is_empty());
    assert!(PcCore
        .export_slots("ibm-pc", document(&bytes), "machine")
        .unwrap()
        .is_empty());
    let mut sink = MemoryExportSink::new();
    let report = PcCore
        .export(
            "ibm-pc",
            document(&bytes),
            ExportRequest::new("machine", &[]),
            &mut sink,
        )
        .unwrap();
    assert_eq!(report.outputs.len(), 1);
    let outputs = sink.outputs();
    pc_config::parse_toml(
        "machine.toml",
        std::str::from_utf8(&outputs["machine.toml"]).unwrap(),
    )
    .unwrap();
}

#[test]
fn raw_floppies_use_the_selected_media_length_and_keep_drive_settings_as_notes() {
    let bytes = catalogue_config();
    let reader = Bytes(vec![0xa5; 737280]);
    let media = [ExportMedium::new(SlotId(0), reader.size(), &reader)];
    let mut sink = MemoryExportSink::new();
    let report = PcCore
        .export(
            "ibm-pc",
            document(&bytes),
            ExportRequest::new("raw", &media),
            &mut sink,
        )
        .unwrap();
    assert_eq!(sink.outputs()["slot-0.img"], reader.0);
    assert!(report
        .notes
        .iter()
        .any(|n| n.contains("drive type") && n.contains("CHS")));
    assert!(report
        .notes
        .iter()
        .any(|n| n.contains("machine.board.model")));
}
