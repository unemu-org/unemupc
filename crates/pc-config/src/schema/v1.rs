// SPDX-License-Identifier: AGPL-3.0-or-later
//! Schema 1 layers for PC/AT machine files, not hardware configurations.
//!
//! See `backlog/docs/design/machine-files.md` § Schema. Flat card and drive
//! tables preserve TOML error positions; kind-specific checks belong to lowering.
//! Optional fields preserve override intent until the `extends` merge.

pub mod projection;
mod values;
pub use values::*;

use serde::Deserialize;

// Timing requests must fail at their value, rather than as unexplained unknown
// keys. Expanding identical fields keeps serde's direct, positioned table parser.
macro_rules! timed_table {
    ($(#[$meta:meta])* pub struct $name:ident { $($fields:tt)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $($fields)*
            #[serde(default, deserialize_with = "values::reject_present")] mhz: Option<CycleExact>,
            #[serde(default, deserialize_with = "values::reject_present")] frequency: Option<CycleExact>,
            #[serde(default, deserialize_with = "values::reject_present")] clock_hz: Option<CycleExact>,
            #[serde(default, deserialize_with = "values::reject_present")] cycles: Option<CycleExact>,
            #[serde(default, deserialize_with = "values::reject_present")] cycle_exact: Option<CycleExact>,
            #[serde(default, deserialize_with = "values::reject_present")] wait_states: Option<CycleExact>,
            #[serde(default, deserialize_with = "values::reject_present")] bus_mhz: Option<CycleExact>,
            #[serde(default, deserialize_with = "values::reject_present")] memory_wait_states: Option<CycleExact>,
            #[serde(default, deserialize_with = "values::reject_present")] timing: Option<CycleExact>,
        }
    };
}

timed_table! {
    /// A syntax-checked layer. Completeness is checked only after inheritance.
    pub struct MachineFile {
        pub schema: SchemaVersion,
        pub name: Option<String>,
        pub description: Option<String>,
        pub extends: Option<String>,
        pub board: Option<Board>,
        pub cpu: Option<Cpu>,
        pub memory: Option<Memory>,
        pub firmware: Option<Firmware>,
        pub card: Option<Vec<Card>>,
        pub disks: Option<Vec<ZeroDisk>>,
        #[serde(default, deserialize_with = "values::reject_present")] run: Option<OutsideMachine>,
        #[serde(default, deserialize_with = "values::reject_present")] presentation: Option<OutsideMachine>,
        #[serde(default, deserialize_with = "values::reject_present")] streams: Option<OutsideMachine>,
    }
}

timed_table! {
    /// Board selection and overrides; motherboard expansion belongs to lowering.
    pub struct Board {
        pub model: Option<BoardModel>,
        pub display_switch: Option<DisplaySwitch>,
        pub post_port: Option<NumberOrNone<65535>>,
        pub rtc: Option<Rtc>,
        /// Standard PS/2 mouse on the motherboard 8042 auxiliary port.
        pub mouse: Option<bool>,
        #[serde(default, deserialize_with = "values::reject_a20")] a20: Option<Reserved>,
        #[serde(default, deserialize_with = "values::reject_kbc")] kbc: Option<Reserved>,
        #[serde(default, deserialize_with = "values::reject_keyboard")] keyboard: Option<Reserved>,
        #[serde(default, deserialize_with = "values::reject_keyboard_layout")] keyboard_layout: Option<Reserved>,
        #[serde(default, deserialize_with = "values::reject_dip_switches")] dip_switches: Option<Reserved>,
        #[serde(default, deserialize_with = "values::reject_chipset")] chipset: Option<Reserved>,
        #[serde(default, deserialize_with = "values::reject_speaker")] speaker: Option<Reserved>,
    }
}

/// RTC overrides. Calendar validity and NVRAM size are checked when lowering.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rtc {
    #[serde(default, deserialize_with = "values::rtc_start")]
    pub start: Option<String>,
    pub nvram: Option<Nvram>,
}

timed_table! {
    /// CPU selection and approximate nominal clock or exact instruction rate.
    pub struct Cpu {
        pub model: Option<CpuModel>,
        pub rate: Option<CpuRate>,
        #[serde(default, deserialize_with = "values::reject_present")]
        pub clock_mhz: Option<ClockMhz>,
        pub fpu: Option<FpuModel>,
        #[serde(default, deserialize_with = "values::reject_present")]
        pub turbo: Option<Turbo>,
        /// Host execution engine; not hardware, so the guest never sees it.
        pub engine: Option<CpuEngine>,
    }
}

/// Alternate button position; a clock is required when the table is present.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turbo {
    pub clock_mhz: ClockMhz,
}

/// RAM size overrides; alignment and board limits belong to config validation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    pub conventional_kib: Option<u32>,
    pub extended_kib: Option<u32>,
    #[serde(default, deserialize_with = "values::reject_shadow")]
    shadow: Option<Reserved>,
    #[serde(default, deserialize_with = "values::reject_umb")]
    umb: Option<Reserved>,
    #[serde(default, deserialize_with = "values::reject_upper")]
    upper: Option<Reserved>,
}

/// Firmware declarations, with no image reads or catalogue lookup during parsing.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Firmware {
    pub bios: Option<Rom>,
    pub option_roms: Option<Vec<Rom>>,
    #[serde(default, deserialize_with = "values::reject_basic")]
    basic: Option<Reserved>,
}

/// A BIOS or option ROM layer; the latter requires a base after merging.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rom {
    #[serde(rename = "use")]
    pub use_id: Option<String>,
    pub rom: Option<String>,
    pub role: Option<String>,
    pub size: Option<u64>,
    /// Skip an EPROM prefix after checking the complete file's digest.
    pub offset: Option<usize>,
    pub parts: Option<Vec<RomPart>>,
    pub image: Option<String>,
    pub sha256: Option<Sha256>,
    pub base: Option<u32>,
    pub provides: Option<Vec<FirmwareService>>,
}

/// One even/odd chip dump; paths and pins retain their own defining origins.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomPart {
    pub image: String,
    pub lane: Lane,
    #[serde(default)]
    pub offset: u64,
    pub sha256: Option<Sha256>,
}
/// EPROM byte lane in the assembled system ROM.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    Even,
    Odd,
}

timed_table! {
    /// Flat union of card fields; unsupported combinations are lowering errors.
    pub struct Card {
        pub id: Option<String>,
        pub kind: Option<CardKind>,
        pub controller_model: Option<ControllerModel>,
        /// VGA card model.
        pub model: Option<VgaModel>,
        /// Video memory of a Bochs VBE card in MiB.
        pub memory_mib: Option<u32>,
        /// Serial pointing device attached to a UART card.
        pub mouse: Option<SerialMouseModel>,
        pub port: Option<u16>,
        /// Required Lo-tech EMS segment (for example 0xD000), not a physical address.
        pub frame: Option<u16>,
        /// Populated Lo-tech EMS SRAM in KiB.
        pub size_kib: Option<u32>,
        pub base: Option<u16>,
        pub dma8: Option<u8>,
        pub dma16: Option<u8>,
        pub irq: Option<NumberOrNone<15>>,
        pub dma: Option<NumberOrNone<3>>,
        pub drives: Option<Vec<Drive>>,
        pub remove: Option<bool>,
        #[serde(default, deserialize_with = "values::reject_monitor")] monitor: Option<Reserved>,
        #[serde(default, deserialize_with = "values::reject_rom")] rom: Option<Reserved>,
        #[serde(default, deserialize_with = "values::reject_mmio")] mmio: Option<Reserved>,
    }
}

/// Flat floppy/IDE drive layer, preserving missing fields for keyed merging.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drive {
    pub disk: Option<String>,
    pub inserted: Option<MediumSelector>,
    pub unit: Option<u8>,
    pub channel: Option<Channel>,
    #[serde(rename = "type")]
    pub drive_type: Option<DriveType>,
    #[serde(default, deserialize_with = "values::drive_image")]
    pub image: Option<String>,
    pub sha256: Option<Sha256>,
    pub geometry: Option<Geometry>,
    pub write: Option<WriteMode>,
    pub journal: Option<String>,
    pub keep_generations: Option<std::num::NonZeroU32>,
    pub remove: Option<bool>,
    #[serde(default, deserialize_with = "values::media_list")]
    pub media: Option<Vec<Medium>>,
    pub empty: Option<bool>,
}

/// One removable floppy base and its overlay policy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Medium {
    #[serde(default)]
    pub image: String,
    #[serde(rename = "use")]
    pub use_id: Option<String>,
    pub medium: Option<String>,
    pub size: Option<u64>,
    pub sha256: Option<Sha256>,
    pub write: Option<WriteMode>,
    pub journal: Option<String>,
    pub keep_generations: Option<std::num::NonZeroU32>,
}

/// Explicit 512-byte-sector CHS geometry; its hardware limits are checked later.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chs {
    pub cylinders: Option<u32>,
    pub heads: Option<u32>,
    pub sectors: Option<u32>,
}

/// NVRAM image declaration. The default write mode is applied after merging.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NvramImage {
    pub image: Option<String>,
    pub sha256: Option<Sha256>,
    pub write: Option<WriteMode>,
    pub journal: Option<String>,
    pub keep_generations: Option<std::num::NonZeroU32>,
}

/// AT board primary-display switch, independent of card declaration order.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DisplaySwitch {
    Auto,
    Mono,
    Colour,
}

impl<'de> Deserialize<'de> for DisplaySwitch {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "auto" => Ok(Self::Auto),
            "mono" => Ok(Self::Mono),
            "colour" => Ok(Self::Colour),
            "color" => Err(serde::de::Error::custom(
                "unknown display switch `color`; use `colour`",
            )),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["auto", "mono", "colour"],
            )),
        }
    }
}

/// Document-owned zero base; no package or external content represents it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZeroDisk {
    pub id: String,
    pub size: u64,
}

/// A complete package medium, with optional assertions on its received identity.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediumSelector {
    #[serde(rename = "use")]
    pub use_id: Option<String>,
    pub medium: Option<String>,
    pub size: Option<u64>,
    pub sha256: Option<Sha256>,
}
