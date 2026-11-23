// SPDX-License-Identifier: AGPL-3.0-or-later
//! Core-owned compatibility rows for stored PC configurations; machine-images §5.6.
//! IBM Personal Computer AT Technical Reference, System Board: hardware vocabulary
//! and firmware accepted by the core are rated independently of package formats.

use crate::{
    image::{self, NvramMode, Policy},
    schema::v1 as schema,
};
use std::fmt;

/// A core release, independent of the front end and its storage codecs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Release(pub u64, pub u64, pub u64);
impl Release {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self(major, minor, patch)
    }
}
impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}
type AppVersion = Release;
pub type Row = (&'static str, Release);
pub const BASELINE: Release = Release(0, 0, 1);
pub const NEXT_RELEASE: Release = Release(0, 0, 5);
pub const NEWEST_ROW: Release = NEXT_RELEASE;
/// Core config release. Application composition supplies CoreDescription::version to readers.
pub fn package() -> Release {
    Release(
        env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(0),
        env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(0),
        env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or(0),
    )
}

/// Accepted core platform spellings retain their historical release ratings.
pub const PLATFORMS: &[Row] = &[("ibm-pc-at", BASELINE), ("ibm-pc", Release(0, 0, 3))];

pub fn lookup(rows: &[&[Row]], key: &str) -> Option<Release> {
    rows.iter()
        .flat_map(|rows| rows.iter())
        .find(|(name, _)| *name == key)
        .map(|(_, release)| *release)
}
fn row(rows: &[Row], what: &str, value: &str) -> image::Result<Release> {
    lookup(&[rows], value)
        .ok_or_else(|| image::invalid(format!("no compatibility row for {what} {value}")))
}
pub fn platform(id: &str) -> image::Result<Release> {
    row(PLATFORMS, "core platform", id)
}

/// Keys of the PC machine config: the bundle tables and, under `machine`,
/// the flattened schema-1 machine. A bundle never holds the descriptive or
/// inheritance keys, the drive media keys or NVRAM image policy keys of a
/// machine file, so those have no row.
pub const MACHINE_KEYS: &[Row] = &[
    ("schema", BASELINE),
    ("platform", BASELINE),
    ("machine", BASELINE),
    ("firmware", BASELINE),
    ("firmware.key", BASELINE),
    ("firmware.source", BASELINE),
    ("firmware.blob", BASELINE),
    ("firmware.source_archive", BASELINE),
    ("media", BASELINE),
    ("media.id", BASELINE),
    ("media.source", BASELINE),
    ("media.head", BASELINE),
    ("media.name", BASELINE),
    ("media.policy", BASELINE),
    ("media.geometry", BASELINE),
    ("media.geometry.cylinders", BASELINE),
    ("media.geometry.heads", BASELINE),
    ("media.geometry.sectors", BASELINE),
    ("media.geometry.sector_bytes", BASELINE),
    ("slots", BASELINE),
    ("slots.controller", BASELINE),
    ("slots.unit", BASELINE),
    ("slots.catalogue", BASELINE),
    ("slots.catalogue_mode", BASELINE),
    ("slots.inserted", BASELINE),
    ("nvram", BASELINE),
    ("nvram.mode", BASELINE),
    ("nvram.source", BASELINE),
    ("nvram.head", BASELINE),
    ("machine.schema", BASELINE),
    ("machine.board", BASELINE),
    ("machine.board.model", BASELINE),
    ("machine.board.display_switch", BASELINE),
    ("machine.board.post_port", BASELINE),
    ("machine.board.rtc", BASELINE),
    ("machine.board.rtc.start", BASELINE),
    ("machine.board.rtc.nvram", BASELINE),
    ("machine.board.rtc.nvram.image", BASELINE),
    ("machine.board.mouse", BASELINE),
    ("machine.cpu", BASELINE),
    ("machine.cpu.model", BASELINE),
    ("machine.cpu.rate", BASELINE),
    ("machine.cpu.fpu", BASELINE),
    ("machine.memory", BASELINE),
    ("machine.memory.conventional_kib", BASELINE),
    ("machine.memory.extended_kib", BASELINE),
    ("machine.firmware", BASELINE),
    ("machine.firmware.bios", BASELINE),
    ("machine.firmware.bios.offset", BASELINE),
    ("machine.firmware.bios.parts", BASELINE),
    ("machine.firmware.bios.parts.image", BASELINE),
    ("machine.firmware.bios.parts.lane", BASELINE),
    ("machine.firmware.bios.parts.offset", BASELINE),
    ("machine.firmware.bios.parts.sha256", BASELINE),
    ("machine.firmware.bios.image", BASELINE),
    ("machine.firmware.bios.sha256", BASELINE),
    ("machine.firmware.bios.base", BASELINE),
    ("machine.firmware.bios.provides", BASELINE),
    ("machine.firmware.option_roms", BASELINE),
    ("machine.firmware.option_roms.offset", BASELINE),
    ("machine.firmware.option_roms.parts", BASELINE),
    ("machine.firmware.option_roms.parts.image", BASELINE),
    ("machine.firmware.option_roms.parts.lane", BASELINE),
    ("machine.firmware.option_roms.parts.offset", BASELINE),
    ("machine.firmware.option_roms.parts.sha256", BASELINE),
    ("machine.firmware.option_roms.image", BASELINE),
    ("machine.firmware.option_roms.sha256", BASELINE),
    ("machine.firmware.option_roms.base", BASELINE),
    ("machine.firmware.option_roms.provides", BASELINE),
    ("machine.card", BASELINE),
    ("machine.card.id", BASELINE),
    ("machine.card.kind", BASELINE),
    ("machine.card.controller_model", BASELINE),
    ("machine.card.mouse", BASELINE),
    ("machine.card.port", BASELINE),
    ("machine.card.frame", BASELINE),
    ("machine.card.size_kib", BASELINE),
    ("machine.card.base", BASELINE),
    ("machine.card.dma8", BASELINE),
    ("machine.card.dma16", BASELINE),
    ("machine.card.irq", BASELINE),
    ("machine.card.dma", BASELINE),
    ("machine.card.remove", BASELINE),
    ("machine.card.drives", BASELINE),
    ("machine.card.drives.unit", BASELINE),
    ("machine.card.drives.channel", BASELINE),
    ("machine.card.drives.type", BASELINE),
    ("machine.card.drives.geometry", BASELINE),
    ("machine.card.drives.geometry.cylinders", BASELINE),
    ("machine.card.drives.geometry.heads", BASELINE),
    ("machine.card.drives.geometry.sectors", BASELINE),
    ("machine.card.drives.remove", BASELINE),
];

/// Firmware catalogue entries, by id (`assembly::BoardModel::firmware_catalogue`). A
/// release that does not list an entry refuses its bytes.
pub const CATALOGUE: &[Row] = &[
    ("seabios-1.17.0-isa", NEXT_RELEASE),
    ("kiselev-xi8088-1.0.2", BASELINE),
    ("kiselev-xi8088-1.0.2-atreset", BASELINE),
    ("unemu-at-bios-1.0.2-unemu.3", BASELINE),
    ("unemu-at-bios-1.0.2-unemu.2", BASELINE),
    ("ibm-5170-bios-1984-01-10", BASELINE),
    ("ibm-5170-bios-1985-06-10", BASELINE),
    ("ibm-5170-bios-1985-11-15", BASELINE),
    ("landmark-rom-post-at-1.2.0", BASELINE),
    ("ibm-vga-bios-1987-32k", BASELINE),
    ("vgabios-0.9d-unemu.1-std", BASELINE),
    ("vgabios-0.8a-unemu.1-std", BASELINE),
    ("xub-r638-at", BASELINE),
    ("xub-r638-xtp-ata1f0", BASELINE),
];

/// Rate the config keys, hardware vocabulary and accepted platform spelling.
pub fn machine(table: &toml::Table) -> image::Result<Release> {
    let mut release = keys(table, &[MACHINE_KEYS], &[])?;
    let config = image::Config::decode(table.clone())?;
    release = release.max(platform(&config.platform)?);
    let machine: schema::MachineFile = toml::Value::Table(config.machine.clone())
        .try_into()
        .map_err(|e: toml::de::Error| image::invalid(e.to_string()))?;
    release = release.max(machine_values(&machine));
    for medium in &config.media {
        release = release.max(policy(medium.policy));
    }
    if let Some(nvram) = &config.nvram {
        release = release.max(nvram_mode(&nvram.mode));
    }
    Ok(release)
}
/// Rate ROM bytes by the catalogue the core actually accepts, whole or split.
pub fn firmware(sha256: &str, size: u64) -> image::Result<Release> {
    let mut release = BASELINE;
    for entry in [
        assembly::BoardModel::Ibm5170,
        assembly::BoardModel::Generic386Isa,
    ]
    .into_iter()
    .flat_map(|board| board.firmware_catalogue())
    {
        let chip = entry
            .parts
            .iter()
            .flat_map(|layout| layout.iter())
            .any(|part| part.sha256 == sha256 && part.size == size);
        if chip || (entry.sha256 == sha256 && entry.size == size) {
            release = release.max(row(CATALOGUE, "firmware catalogue entry", entry.id)?);
        }
    }
    Ok(release)
}
/// The newest release the vocabulary values of a flattened machine need.
fn machine_values(machine: &schema::MachineFile) -> AppVersion {
    let mut values = Vec::new();
    if let Some(board) = &machine.board {
        values.extend(board.model.map(board_model));
        values.extend(board.display_switch.map(display_switch));
        values.extend(board.post_port.map(number_or_none));
        values.extend(
            board
                .rtc
                .as_ref()
                .and_then(|rtc| rtc.nvram.as_ref())
                .map(rtc_nvram),
        );
    }
    if let Some(cpu) = &machine.cpu {
        values.extend(cpu.model.map(cpu_model));
        values.extend(cpu.fpu.map(fpu_model));
    }
    if let Some(firmware) = &machine.firmware {
        let roms = firmware
            .bios
            .iter()
            .chain(firmware.option_roms.iter().flatten());
        for rom in roms {
            values.extend(rom.provides.iter().flatten().map(|s| firmware_service(*s)));
            values.extend(rom.parts.iter().flatten().map(|p| lane(p.lane)));
        }
    }
    for card in machine.card.iter().flatten() {
        values.extend(card.kind.map(card_kind));
        values.extend(card.controller_model.map(controller_model));
        values.extend(card.mouse.map(serial_mouse));
        values.extend(card.irq.map(number_or_none));
        values.extend(card.dma.map(number_or_none));
        for drive in card.drives.iter().flatten() {
            values.extend(drive.channel.map(channel));
            values.extend(drive.drive_type.map(drive_type));
            values.extend(drive.write.map(write_mode));
            values.extend(
                drive
                    .media
                    .iter()
                    .flatten()
                    .filter_map(|m| m.write.map(write_mode)),
            );
            values.extend(drive.geometry.as_ref().map(geometry));
        }
    }
    values.into_iter().fold(BASELINE, AppVersion::max)
}

/// The release that first reads a board model.
pub fn board_model(value: schema::BoardModel) -> AppVersion {
    match value {
        schema::BoardModel::Ibm5170 => BASELINE,
        schema::BoardModel::Generic386Isa => NEXT_RELEASE,
    }
}

/// The release that first reads a CPU model.
pub fn cpu_model(value: schema::CpuModel) -> AppVersion {
    match value {
        schema::CpuModel::I80286 => BASELINE,
        schema::CpuModel::I80386 => NEXT_RELEASE,
    }
}

/// The release that first reads a coprocessor choice.
pub fn fpu_model(value: schema::FpuModel) -> AppVersion {
    match value {
        schema::FpuModel::None | schema::FpuModel::I80287 => BASELINE,
    }
}

/// The release that first reads a card kind.
pub fn card_kind(value: schema::CardKind) -> AppVersion {
    use schema::CardKind::*;
    match value {
        Mda | VgaText | Vga | Uart16550 | Fdc765 | Ide | Sb16 | LotechEms => BASELINE,
    }
}

/// The release that first reads an IDE controller model.
pub fn controller_model(value: schema::ControllerModel) -> AppVersion {
    match value {
        schema::ControllerModel::Ata | schema::ControllerModel::AtAdapter => BASELINE,
    }
}

/// The release that first reads a serial mouse model.
pub fn serial_mouse(value: schema::SerialMouseModel) -> AppVersion {
    match value {
        schema::SerialMouseModel::Microsoft | schema::SerialMouseModel::None => BASELINE,
    }
}

/// The release that first reads a floppy drive type.
pub fn drive_type(value: schema::DriveType) -> AppVersion {
    use schema::DriveType::*;
    match value {
        Hd144 | Hd12 | Dd720 | Dd360 => BASELINE,
    }
}

/// The release that first reads an IDE channel.
pub fn channel(value: schema::Channel) -> AppVersion {
    match value {
        schema::Channel::Master | schema::Channel::Slave => BASELINE,
    }
}

/// The release that first reads a media write mode.
pub fn write_mode(value: schema::WriteMode) -> AppVersion {
    use schema::WriteMode::*;
    match value {
        Volatile | ReadOnly | Journal => BASELINE,
    }
}

/// The release that first reads a split ROM's byte lane.
pub fn lane(value: schema::Lane) -> AppVersion {
    match value {
        schema::Lane::Even | schema::Lane::Odd => BASELINE,
    }
}

/// The release that first reads a declared firmware service.
pub fn firmware_service(value: schema::FirmwareService) -> AppVersion {
    match value {
        schema::FirmwareService::Floppy | schema::FirmwareService::FixedDisk => BASELINE,
    }
}

/// The release that first reads a primary-display switch setting.
pub fn display_switch(value: schema::DisplaySwitch) -> AppVersion {
    use schema::DisplaySwitch::*;
    match value {
        Auto | Mono | Colour => BASELINE,
    }
}

/// The release that first reads a line number or its explicit `"none"`.
pub fn number_or_none<const MAX: u16>(value: schema::NumberOrNone<MAX>) -> AppVersion {
    match value {
        schema::NumberOrNone::Number(_) | schema::NumberOrNone::None => BASELINE,
    }
}

/// The release that first reads a form of the RTC NVRAM setting.
pub fn rtc_nvram(value: &schema::Nvram) -> AppVersion {
    match value {
        schema::Nvram::Derived | schema::Nvram::Blank => BASELINE,
        schema::Nvram::Image(image) => image.write.map_or(BASELINE, write_mode),
    }
}

/// The release that first reads a fixed-disk geometry form.
pub fn geometry(value: &schema::Geometry) -> AppVersion {
    match value {
        schema::Geometry::Chs(_) => BASELINE,
    }
}

/// The release that first reads a bundle medium's policy.
pub fn policy(value: Policy) -> AppVersion {
    match value {
        Policy::Readonly | Policy::Volatile | Policy::Journal => BASELINE,
    }
}

fn nvram_mode(value: &NvramMode) -> AppVersion {
    match value {
        NvramMode::Image | NvramMode::Journal => BASELINE,
    }
}

pub fn keys(config: &toml::Table, rows: &[&[Row]], foreign: &[&str]) -> image::Result<AppVersion> {
    fn walk(
        table: &toml::Table,
        prefix: &str,
        rows: &[&[Row]],
        foreign: &[&str],
        release: &mut AppVersion,
    ) -> image::Result<()> {
        for (key, value) in table {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            let since = lookup(rows, &path).ok_or_else(|| {
                image::invalid(format!("no compatibility row for config key {path}"))
            })?;
            *release = (*release).max(since);
            if foreign.contains(&path.as_str()) {
                continue;
            }
            match value {
                toml::Value::Table(table) => walk(table, &path, rows, foreign, release)?,
                toml::Value::Array(items) => {
                    for table in items.iter().filter_map(toml::Value::as_table) {
                        walk(table, &path, rows, foreign, release)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    let mut release = BASELINE;
    walk(config, "", rows, foreign, &mut release)?;
    Ok(release)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn struct_fields<T: serde::de::DeserializeOwned>() -> &'static [&'static str] {
        use serde::de::{Deserializer, Visitor};
        #[derive(Debug)]
        struct Fields(&'static [&'static str]);
        impl fmt::Display for Fields {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("struct fields")
            }
        }
        impl std::error::Error for Fields {}
        impl serde::de::Error for Fields {
            fn custom<M: fmt::Display>(_: M) -> Self {
                Self(&[])
            }
        }
        struct Probe;
        impl<'de> Deserializer<'de> for Probe {
            type Error = Fields;
            fn deserialize_any<V: Visitor<'de>>(
                self,
                _: V,
            ) -> std::result::Result<V::Value, Fields> {
                Err(Fields(&[]))
            }
            fn deserialize_struct<V: Visitor<'de>>(
                self,
                _: &'static str,
                fields: &'static [&'static str],
                _: V,
            ) -> std::result::Result<V::Value, Fields> {
                Err(Fields(fields))
            }
            serde::forward_to_deserialize_any! {
                bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
                bytes byte_buf option unit unit_struct newtype_struct seq tuple
                tuple_struct map enum identifier ignored_any
            }
        }
        match T::deserialize(Probe) {
            Err(Fields(fields)) => fields,
            Ok(_) => &[],
        }
    }

    fn paths(prefix: &str, fields: &[&str]) -> Vec<String> {
        fields
            .iter()
            .map(|field| {
                if prefix.is_empty() {
                    (*field).to_owned()
                } else {
                    format!("{prefix}.{field}")
                }
            })
            .collect()
    }

    /// Timing requests: schema 1 refuses them in the machine, board, CPU and
    /// card tables.
    const TIMING: &[&str] = &[
        "mhz",
        "frequency",
        "clock_hz",
        "cycles",
        "cycle_exact",
        "wait_states",
        "bus_mhz",
        "memory_wait_states",
        "timing",
    ];

    /// Every field of every config struct has a row or is a key a bundle's
    /// machine never holds, and every row names a field, so a new optional
    /// config key cannot ship without a row. Schema 1 refuses the reserved
    /// names (timing, unmodelled hardware, run and presentation tables); the
    /// bundle refuses descriptive and inheritance keys, a drive's media keys
    /// and the NVRAM image's policy keys.
    #[test]
    fn every_machine_config_key_has_a_row() {
        let mut expected = Vec::new();
        let mut check = |prefix: &str, fields: &[&str], refused: &[&[&str]]| {
            for (field, path) in fields.iter().zip(paths(prefix, fields)) {
                if refused.iter().any(|list| list.contains(field)) {
                    assert!(
                        lookup(&[MACHINE_KEYS], &path).is_none(),
                        "{path} has a row but no bundle holds it"
                    );
                    continue;
                }
                assert!(
                    lookup(&[MACHINE_KEYS], &path).is_some(),
                    "no row for config key {path}"
                );
                expected.push(path);
            }
        };
        check("", struct_fields::<image::Config>(), &[]);
        check("firmware", struct_fields::<image::Firmware>(), &[]);
        check("media", struct_fields::<image::Medium>(), &[]);
        check("media.geometry", struct_fields::<image::Geometry>(), &[]);
        check("slots", struct_fields::<image::Slot>(), &[]);
        check("nvram", struct_fields::<image::Nvram>(), &[]);
        check(
            "machine",
            struct_fields::<schema::MachineFile>(),
            &[
                TIMING,
                &[
                    "run",
                    "presentation",
                    "streams",
                    "name",
                    "description",
                    "extends",
                ],
            ],
        );
        check(
            "machine.board",
            struct_fields::<schema::Board>(),
            &[
                TIMING,
                &[
                    "a20",
                    "kbc",
                    "keyboard",
                    "keyboard_layout",
                    "dip_switches",
                    "chipset",
                    "speaker",
                ],
            ],
        );
        check("machine.board.rtc", struct_fields::<schema::Rtc>(), &[]);
        check(
            "machine.board.rtc.nvram",
            struct_fields::<schema::NvramImage>(),
            &[&["sha256", "write", "journal", "keep_generations"]],
        );
        check(
            "machine.cpu",
            struct_fields::<schema::Cpu>(),
            &[TIMING, &["turbo"]],
        );
        check(
            "machine.memory",
            struct_fields::<schema::Memory>(),
            &[&["shadow", "umb", "upper"]],
        );
        check(
            "machine.firmware",
            struct_fields::<schema::Firmware>(),
            &[&["basic"]],
        );
        for rom in ["bios", "option_roms"] {
            let prefix = format!("machine.firmware.{rom}");
            check(&prefix, struct_fields::<schema::Rom>(), &[]);
            check(
                &format!("{prefix}.parts"),
                struct_fields::<schema::RomPart>(),
                &[],
            );
        }
        check(
            "machine.card",
            struct_fields::<schema::Card>(),
            &[TIMING, &["monitor", "rom", "mmio"]],
        );
        check(
            "machine.card.drives",
            struct_fields::<schema::Drive>(),
            &[&[
                "image",
                "sha256",
                "write",
                "journal",
                "keep_generations",
                "media",
                "empty",
            ]],
        );
        check(
            "machine.card.drives.geometry",
            struct_fields::<schema::Chs>(),
            &[],
        );
        for (row, _) in MACHINE_KEYS {
            assert!(expected.iter().any(|p| p == row), "row {row} names no key");
        }
    }
    fn vocabulary() -> Vec<AppVersion> {
        use schema::*;
        let mut all = vec![
            board_model(BoardModel::Ibm5170),
            board_model(BoardModel::Generic386Isa),
            cpu_model(CpuModel::I80386),
            cpu_model(CpuModel::I80286),
        ];
        all.extend([FpuModel::None, FpuModel::I80287].map(fpu_model));
        all.extend(
            [
                CardKind::Mda,
                CardKind::VgaText,
                CardKind::Vga,
                CardKind::Uart16550,
                CardKind::Fdc765,
                CardKind::Ide,
                CardKind::Sb16,
                CardKind::LotechEms,
            ]
            .map(card_kind),
        );
        all.extend([ControllerModel::Ata, ControllerModel::AtAdapter].map(controller_model));
        all.extend([SerialMouseModel::Microsoft, SerialMouseModel::None].map(serial_mouse));
        all.extend(
            [
                DriveType::Hd144,
                DriveType::Hd12,
                DriveType::Dd720,
                DriveType::Dd360,
            ]
            .map(drive_type),
        );
        all.extend([Channel::Master, Channel::Slave].map(channel));
        all.extend([WriteMode::Volatile, WriteMode::ReadOnly, WriteMode::Journal].map(write_mode));
        all.extend([Lane::Even, Lane::Odd].map(lane));
        all.extend([FirmwareService::Floppy, FirmwareService::FixedDisk].map(firmware_service));
        all.extend(
            [
                DisplaySwitch::Auto,
                DisplaySwitch::Mono,
                DisplaySwitch::Colour,
            ]
            .map(display_switch),
        );
        all.extend([NumberOrNone::<15>::Number(0), NumberOrNone::<15>::None].map(number_or_none));
        all.extend([Nvram::Derived, Nvram::Blank].iter().map(rtc_nvram));
        all.extend([Policy::Readonly, Policy::Volatile, Policy::Journal].map(policy));
        all.extend(
            [NvramMode::Image, NvramMode::Journal]
                .iter()
                .map(nvram_mode),
        );
        all
    }

    #[test]
    fn newest_row_and_catalogue_coverage_are_core_owned() {
        let catalogue: Vec<_> = [
            assembly::BoardModel::Ibm5170,
            assembly::BoardModel::Generic386Isa,
        ]
        .into_iter()
        .flat_map(|board| board.firmware_catalogue())
        .collect();
        for entry in &catalogue {
            assert!(lookup(&[CATALOGUE], entry.id).is_some(), "{}", entry.id);
        }
        assert_eq!(CATALOGUE.len(), catalogue.len());
        let newest = [MACHINE_KEYS, CATALOGUE, PLATFORMS]
            .iter()
            .flat_map(|rows| rows.iter().map(|(_, release)| *release))
            .chain(vocabulary())
            .fold(BASELINE, Release::max);
        assert_eq!(newest, NEWEST_ROW);
        let package: Vec<u64> = env!("CARGO_PKG_VERSION")
            .split('.')
            .map(|part| part.parse().unwrap())
            .collect();
        assert!(NEXT_RELEASE > Release(package[0], package[1], package[2]));
        assert!(platform("unknown-platform").is_err());
    }
}
