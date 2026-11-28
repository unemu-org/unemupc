// SPDX-License-Identifier: AGPL-3.0-or-later
//! The codec over a config, rows and content in memory: no store anywhere.

use super::*;
use std::collections::BTreeMap;
use unemu_core_api::{Content, ContentReader, ContentSource, ErrorKind};

use crate::fixtures;

/// Content in memory by SHA-256.
struct Memory(BTreeMap<[u8; 32], Vec<u8>>);
impl Memory {
    fn new<'a>(blobs: impl IntoIterator<Item = &'a [u8]>) -> Self {
        Self(
            blobs
                .into_iter()
                .map(|bytes| (Digest::of(bytes).sha256(), bytes.to_vec()))
                .collect(),
        )
    }
}
impl ContentSource for Memory {
    fn open(
        &self,
        sha256: &[u8; 32],
        size: u64,
    ) -> std::result::Result<Content, unemu_core_api::Error> {
        self.0
            .get(sha256)
            .filter(|bytes| bytes.len() as u64 == size)
            .map(|bytes| Content::Bytes(bytes.clone()))
            .ok_or_else(|| unemu_core_api::Error::new(ErrorKind::Content, "not in memory"))
    }
}

/// A stored config of an AT on the open Kiselev BIOS with a journalled
/// 360K floppy and a journalled CMOS image, and its three blobs; `None`
/// when the BIOS fixture is missing.
fn image() -> Option<(Config, [Vec<u8>; 3])> {
    let bios = fixtures::fixture("kiselev-bios/bios-xi8088.bin")?;
    let floppy = vec![0xF6; 368_640];
    let cmos = vec![0; 64];
    let [b, f, c] = [&bios, &floppy, &cmos].map(|bytes| Digest::of(bytes));
    let text = format!(
        r#"schema = 1
platform = "ibm-pc"
[machine]
schema = 1
[machine.board]
model = "ibm-5170"
[machine.board.rtc.nvram]
image = "assets/cmos.bin"
[machine.firmware.bios]
image = "assets/bios.bin"
sha256 = "{bios}"
[[machine.card]]
kind = "fdc-765"
[[machine.card.drives]]
unit = 0
type = "360K"
[[firmware]]
key = "firmware.bios"
source = "assets/bios.bin"
blob = "{b}"
[[media]]
id = "a"
source = "assets/a.img"
head = "{f}"
policy = "journal"
geometry = {{ cylinders = 40, heads = 2, sectors = 9, sector_bytes = 512 }}
[[slots]]
controller = "fdc-765"
unit = 0
catalogue = ["a"]
catalogue_mode = false
inserted = "a"
[nvram]
mode = "journal"
source = "assets/cmos.bin"
head = "{c}"
"#,
        bios = b.hex(),
    );
    let config = Config::decode(text.parse().unwrap()).unwrap();
    Some((config, [bios, floppy, cmos]))
}
fn rows(blobs: &[Vec<u8>; 3]) -> BTreeMap<Digest, ContentRow> {
    blobs
        .iter()
        .map(|bytes| {
            let digest = Digest::of(bytes);
            let row = ContentRow {
                sha256: digest.sha256(),
                size: bytes.len() as u64,
            };
            (digest, row)
        })
        .collect()
}

/// What the codec half knows of a medium's size comes from its row, and
/// whether the platform is this provider's from its argument.
#[test]
fn the_codec_half_refuses_from_its_rows_and_arguments() {
    let Some((config, blobs)) = image() else {
        return;
    };
    let small = Memory::new([&blobs[0][..], &blobs[2][..]]);
    let lower = |rows: &BTreeMap<Digest, ContentRow>, platform_known| {
        lower(
            config.clone(),
            rows,
            &small,
            platform_known,
            assembly::AssemblyOptions::default(),
        )
        .err()
        .map(|e| e.to_string())
    };
    let floppy = Digest::of(&blobs[1]);
    let mut short = rows(&blobs);
    short.get_mut(&floppy).unwrap().size -= 512;
    assert_eq!(
        lower(&short, true).as_deref(),
        Some("invalid PC image: medium geometry differs from payload length")
    );
    let mut missing = rows(&blobs);
    missing.remove(&floppy);
    assert_eq!(
        lower(&missing, true),
        Some(format!(
            "invalid PC image: missing content row for {floppy}"
        ))
    );
    assert_eq!(
        lower(&rows(&blobs), false).as_deref(),
        Some("invalid PC image: unsupported PC config envelope")
    );
}

/// A source that answers the row's digest with content of another length:
/// bytes one short, or a reader that claims the largest size there is.
struct Liar {
    honest: Memory,
    reader: bool,
}
struct Huge;
impl ContentReader for Huge {
    fn size(&self) -> u64 {
        u64::MAX
    }
    fn read_at(
        &self,
        _offset: u64,
        buf: &mut [u8],
    ) -> std::result::Result<(), unemu_core_api::Error> {
        buf.fill(0);
        Ok(())
    }
}
impl ContentSource for Liar {
    fn open(
        &self,
        sha256: &[u8; 32],
        size: u64,
    ) -> std::result::Result<Content, unemu_core_api::Error> {
        if self.reader {
            return Ok(Content::Reader(Box::new(Huge)));
        }
        match self.honest.open(sha256, size)? {
            Content::Bytes(mut bytes) => {
                bytes.pop();
                Ok(Content::Bytes(bytes))
            }
            other => Ok(other),
        }
    }
}

/// Content of another length than its row is refused before it is read or
/// lowered, whether it comes as bytes or as a reader.
#[test]
fn content_of_another_length_than_its_row_is_refused() {
    let Some((config, blobs)) = image() else {
        return;
    };
    for reader in [false, true] {
        let source = Liar {
            honest: Memory::new([&blobs[0][..], &blobs[2][..]]),
            reader,
        };
        let error = lower(
            config.clone(),
            &rows(&blobs),
            &source,
            true,
            assembly::AssemblyOptions::default(),
        )
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        assert!(
            error.contains("content size differs from its row"),
            "reader {reader}: {error}"
        );
    }
}

/// The digest's text form is the image store's, both ways.
#[test]
fn a_digest_reads_and_writes_the_store_text_form() {
    let text = "sha256:00ff10a0b1c2d3e4f5061728394a5b6c7d8e9fa0b1c2d3e4f5061728394a5b6c";
    let digest: Digest = text.parse().unwrap();
    assert_eq!(digest.to_string(), text);
    assert_eq!(digest.hex(), &text[7..]);
    assert_eq!(Digest::from_sha256(digest.sha256()), digest);
    assert_eq!(
        Digest::of(b"").to_string(),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    for bad in [
        &text[7..],
        "sha256:00FF10a0b1c2d3e4f5061728394a5b6c7d8e9fa0b1c2d3e4f5061728394a5b6c",
        "sha256:00ff",
        "sha512:00ff10a0b1c2d3e4f5061728394a5b6c7d8e9fa0b1c2d3e4f5061728394a5b6c",
    ] {
        assert_eq!(
            bad.parse::<Digest>().unwrap_err().to_string(),
            "invalid image: digest must be sha256: followed by 64 lowercase hex digits"
        );
    }
}

#[test]
fn optional_80387_survives_projection_defaults_and_image_roundtrip() {
    let document = crate::parse_toml(
        "probe.toml",
        "schema = 1\n[board]\nmodel = 'generic-386-isa'\n[cpu]\nfpu = '80387'\n",
    )
    .unwrap();
    let mut hardware = encode::machine(&document);
    assert_eq!(hardware["cpu"]["fpu"].as_str(), Some("80387"));
    let mut config =
        assembly::capabilities::Draft::for_board(assembly::BoardModel::Generic386Isa).config;
    config.cpu.fpu = assembly::FpuChip::I80387;
    defaults::materialize(&mut hardware, &config).unwrap();
    assert_eq!(hardware["cpu"]["fpu"].as_str(), Some("80387"));
    let image = Config {
        schema: 1,
        platform: "ibm-pc".into(),
        machine: hardware,
        firmware: vec![],
        media: vec![],
        slots: vec![],
        nvram: None,
    };
    let decoded = Config::decode(image.encode().unwrap().parse().unwrap()).unwrap();
    assert_eq!(decoded.machine, image.machine);
    assert_eq!(
        crate::compat::machine(&decoded.encode().unwrap().parse().unwrap()).unwrap(),
        crate::compat::Release(0, 0, 6)
    );
}

#[test]
fn bochs_vbe_card_survives_projection_defaults_and_image_roundtrip() {
    let document = crate::parse_toml(
        "probe.toml",
        "schema = 1\n[board]\nmodel = 'generic-486-isa'\n\
         [[card]]\nkind = 'vga'\nmodel = 'bochs-vbe'\nmemory_mib = 8\n",
    )
    .unwrap();
    let mut hardware = encode::machine(&document);
    assert_eq!(hardware["card"][0]["model"].as_str(), Some("bochs-vbe"));
    assert_eq!(hardware["card"][0]["memory_mib"].as_integer(), Some(8));
    let mut config =
        assembly::capabilities::Draft::for_board(assembly::BoardModel::Generic486Isa).config;
    config
        .devices
        .retain(|device| device.id != assembly::DeviceId::Vga);
    config.devices.push(assembly::DeviceConfig::new(
        assembly::DeviceId::Vga,
        assembly::DeviceKind::BochsVbe(assembly::BochsVbeCfg { memory_mib: 8 }),
        assembly::BochsVbeCfg::port_windows(),
    ));
    hardware["card"][0]
        .as_table_mut()
        .unwrap()
        .remove("memory_mib");
    defaults::materialize(&mut hardware, &config).unwrap();
    assert_eq!(hardware["card"][0]["model"].as_str(), Some("bochs-vbe"));
    assert_eq!(
        hardware["card"][0]["memory_mib"].as_integer(),
        Some(8),
        "defaults name the lowered memory size"
    );
    let image = Config {
        schema: 1,
        platform: "ibm-pc".into(),
        machine: hardware,
        firmware: vec![],
        media: vec![],
        slots: vec![],
        nvram: None,
    };
    let decoded = Config::decode(image.encode().unwrap().parse().unwrap()).unwrap();
    assert_eq!(decoded.machine, image.machine);
    assert_eq!(
        crate::compat::machine(&decoded.encode().unwrap().parse().unwrap()).unwrap(),
        crate::compat::Release(0, 0, 6)
    );
}
