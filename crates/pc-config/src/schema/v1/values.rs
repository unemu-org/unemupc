// SPDX-License-Identifier: AGPL-3.0-or-later
//! Positioned scalar and two-form decoders for schema 1.
//!
//! See `backlog/docs/design/machine-files.md` § Document rules and § Reserved names.

use std::{fmt, num::NonZeroU64};

use serde::{
    de::{self, value::MapAccessDeserializer, MapAccess, Visitor},
    Deserialize, Deserializer,
};

use super::{Chs, NvramImage};

macro_rules! named_values {
    ($(#[$meta:meta])* $name:ident, $category:literal {
        $($variant:ident => $value:expr),* $(,)?
    } reserved { $($reserved:literal => $reason:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant,)*
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct NameVisitor;
                impl Visitor<'_> for NameVisitor {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                        write!(f, "a {} name", $category)
                    }
                    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                        const NAMES: &[&str] = &[$($value,)* $($reserved,)*];
                        match value {
                            $(name if name == $value => Ok($name::$variant),)*
                            $($reserved => Err(E::custom(format!(
                                "{} `{value}` is reserved, not implemented (schema 1): {}", $category, $reason
                            ))),)*
                            _ => Err(E::unknown_variant(value, NAMES)),
                        }
                    }
                }
                deserializer.deserialize_str(NameVisitor)
            }
        }
    };
}

named_values! {
    /// Implemented board spellings; reserved names appear only in parse errors.
    BoardModel, "board model" { Ibm5170 => "ibm-5170", Generic386Isa => "generic-386-isa", Generic486Isa => "generic-486-isa" } reserved {
        "ibm-5160" => "needs the XT 8255 PPI keyboard interface and an 8088 CPU model",
        "ibm-5150" => "needs the XT 8255 PPI keyboard interface and an 8088 CPU model",
        "bare" => "file-based bare-board composition is not implemented; use Rust test rigs",
        "compaq-deskpro" => "needs a Compaq Deskpro motherboard model",
        "tandy-1000" => "needs a Tandy 1000 motherboard model",
        "i440fx-piix3" => "needs i440FX and PIIX3 chipset models",
        "i440bx-piix4" => "needs i440BX and PIIX4 chipset models",
    }
}

/// AT socket or integrated Intel486 DX FPU; defaults follow the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FpuModel {
    None,
    I80287,
    Integrated,
}

impl<'de> Deserialize<'de> for FpuModel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        match name.as_str() {
            "none" => Ok(Self::None),
            "80287" => Ok(Self::I80287),
            "integrated" => Ok(Self::Integrated),
            _ => Err(de::Error::custom(format!(
                "unsupported cpu.fpu `{name}`; allowed values: `none`, `80287`, `integrated`"
            ))),
        }
    }
}

named_values! {
    /// CPU model names; instruction-set and board compatibility are separate.
    CpuModel, "cpu model" { I80286 => "80286", I80386 => "80386dx", I80486Dx => "80486dx", I80486DxCpuid => "80486dx-cpuid" } reserved {
        "8086" => "needs an 8086 instruction model, which the CPU core does not have yet",
        "8088" => "needs an 8088 instruction model, which the CPU core does not have yet",
        "v20" => "needs a V20 instruction model, which the CPU core does not have yet",
        "80386sx" => "needs an 80386SX CPU model for firmware machines",
        "80486" => "select 80486dx or 80486dx-cpuid explicitly",
        "pentium" => "needs a Pentium CPU model for firmware machines",
        "pentium-ii" => "needs a Pentium II CPU model for firmware machines",
        "pentium-iii" => "needs a Pentium III CPU model for firmware machines",
    }
}

named_values! {
    /// Card spellings, independent of the eventual device assembler.
    CardKind, "card kind" {
        Mda => assembly::descriptors::CardKind::Mda.name(), VgaText => assembly::descriptors::CardKind::VgaText.name(), Vga => assembly::descriptors::CardKind::Vga.name(), Uart16550 => assembly::descriptors::CardKind::Uart16550.name(),
        Fdc765 => assembly::descriptors::CardKind::Fdc765.name(), Ide => assembly::descriptors::CardKind::Ide.name(), Sb16 => assembly::descriptors::CardKind::Sb16.name(), LotechEms => assembly::descriptors::CardKind::LotechEms.name(),
    } reserved {
        "hercules" => "needs a hercules display card model",
        "cga" => "needs a cga display card model",
        "ega" => "needs an EGA display card model",
        "svga-et4000" => "needs a svga-et4000 display card model",
        "svga-s3-trio64" => "needs a svga-s3-trio64 display card model",
        "voodoo" => "needs a voodoo display card model",
        "voodoo2" => "needs a voodoo2 display card model",
        "banshee" => "needs a banshee display card model",
        "adlib" => "needs an AdLib sound card model",
        "sb1" => "needs a sb1 sound card model",
        "sb2" => "needs a sb2 sound card model",
        "sbpro" => "needs a sbpro sound card model",
        "gus" => "needs a gus sound card model",
        "mpu-401" => "needs an MPU-401 sound card model",
        "covox" => "needs a covox sound card model",
        "xt-mfm" => "needs an xt-mfm disk controller model",
        "xt-rll" => "needs an xt-rll disk controller model",
        "xt-ide" => "needs an xt-ide disk controller model",
        "xt-cf-lite" => "needs an xt-cf-lite disk controller model",
        "lpt" => "needs a parallel-port card model",
        "game-port" => "needs a game-port card model",
        "bus-mouse" => "needs a bus-mouse card model",
        "ems" => "needs an expanded-memory card model",
        "ne2000" => "needs an NE2000 network card model",
    }
}

impl CardKind {
    /// The assembly descriptor of this card kind.
    pub fn descriptor(self) -> assembly::descriptors::CardKind {
        use assembly::descriptors::CardKind;
        match self {
            Self::Mda => CardKind::Mda,
            Self::VgaText => CardKind::VgaText,
            Self::Vga => CardKind::Vga,
            Self::Uart16550 => CardKind::Uart16550,
            Self::Fdc765 => CardKind::Fdc765,
            Self::Ide => CardKind::Ide,
            Self::Sb16 => CardKind::Sb16,
            Self::LotechEms => CardKind::LotechEms,
        }
    }
}

named_values! {
    /// Declared firmware services, not claims verified against an image catalogue.
    FirmwareService, "firmware service" {
        Floppy => "floppy", FixedDisk => "fixed-disk",
    } reserved { "video" => "needs a video firmware service model",
        "serial" => "needs a serial firmware service model",
        "network-boot" => "needs a network-boot firmware service model" }
}

named_values! {
    /// Implemented floppy geometry spellings.
    DriveType, "drive type" {
        Hd144 => assembly::descriptors::FloppyType::HD_1_44.name, Hd12 => assembly::descriptors::FloppyType::HD_1_2.name, Dd720 => assembly::descriptors::FloppyType::DD_720K.name, Dd360 => assembly::descriptors::FloppyType::DD_360K.name,
    } reserved { "2.88M" => "needs a 2.88 MB floppy geometry model",
        "atapi-cdrom" => "needs an ATAPI CD-ROM device model" }
}

named_values! {
    /// Register ownership for an IDE card; omitted means ATA.
    ControllerModel, "controller model" {
        Ata => "ata", AtAdapter => "at-adapter",
    } reserved {}
}

named_values! {
    /// IDE device selection within one controller's channel.
    Channel, "IDE channel" { Master => "master", Slave => "slave" } reserved {}
}

named_values! {
    /// Media persistence intent; parsing never creates or opens a journal.
    WriteMode, "write mode" {
        Volatile => "volatile", ReadOnly => "readonly", Journal => "journal",
    } reserved {}
}

/// Schema 1 marker, checked both during the peek and direct typed deserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaVersion;

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let version = i64::deserialize(deserializer)?;
        if version != 1 {
            return Err(de::Error::custom(format!(
                "file uses schema {version}; this build reads schema 1"
            )));
        }
        Ok(Self)
    }
}

/// An explicit disconnected line or a bounded unsigned integer.
///
/// A surrounding `Option` represents an omitted override. `None` here means
/// the file explicitly requests no connection, as in `irq = "none"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberOrNone<const MAX: u16> {
    Number(u16),
    None,
}

impl<'de, const MAX: u16> Deserialize<'de> for NumberOrNone<MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NumberVisitor<const MAX: u16>;
        impl<const MAX: u16> Visitor<'_> for NumberVisitor<MAX> {
            type Value = NumberOrNone<MAX>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "an integer in 0..={MAX} or \"none\"")
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                match u16::try_from(value) {
                    Ok(number) if number <= MAX => Ok(NumberOrNone::Number(number)),
                    _ => Err(E::invalid_value(de::Unexpected::Unsigned(value), &self)),
                }
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                match u64::try_from(value) {
                    Ok(number) => self.visit_u64(number),
                    Err(_) => Err(E::invalid_value(de::Unexpected::Signed(value), &self)),
                }
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value == "none" {
                    Ok(NumberOrNone::None)
                } else {
                    Err(E::invalid_value(de::Unexpected::Str(value), &self))
                }
            }
        }
        deserializer.deserialize_any(NumberVisitor::<MAX>)
    }
}

/// A positive ratio of completed CPU steps to PIT input clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuRate {
    pub steps: NonZeroU64,
    pub pit_clocks: NonZeroU64,
}

/// Nominal MHz stored exactly as integer kHz, from 0.001 through 1000 MHz.
/// The numeric decoder's shortest decimal must have at most three fractional
/// digits. No rounding is performed and strings are deliberately rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockMhz(pub std::num::NonZeroU32);

impl<'de> Deserialize<'de> for ClockMhz {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ClockVisitor;
        impl Visitor<'_> for ClockVisitor {
            type Value = ClockMhz;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "numeric MHz in 0.001..=1000 with at most three fractional decimal digits",
                )
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                value
                    .checked_mul(1000)
                    .filter(|n| *n <= 1_000_000)
                    .and_then(|n| std::num::NonZeroU32::new(n as u32))
                    .map(ClockMhz)
                    .ok_or_else(|| E::invalid_value(de::Unexpected::Unsigned(value), &self))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                match u64::try_from(value) {
                    Ok(n) => self.visit_u64(n),
                    Err(_) => Err(E::invalid_value(de::Unexpected::Signed(value), &self)),
                }
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                let text = value.to_string();
                let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
                let khz = (|| {
                    if fraction.len() > 3
                        || !whole
                            .bytes()
                            .chain(fraction.bytes())
                            .all(|b| b.is_ascii_digit())
                    {
                        return None;
                    }
                    let whole = whole.parse::<u32>().ok()?.checked_mul(1000)?;
                    let part = if fraction.is_empty() {
                        0
                    } else {
                        fraction.parse::<u32>().ok()? * 10_u32.pow(3 - fraction.len() as u32)
                    };
                    std::num::NonZeroU32::new(whole.checked_add(part)?)
                        .filter(|n| n.get() <= 1_000_000)
                })();
                khz.map(ClockMhz)
                    .ok_or_else(|| E::invalid_value(de::Unexpected::Float(value), &self))
            }
        }
        deserializer.deserialize_any(ClockVisitor)
    }
}

impl<'de> Deserialize<'de> for CpuRate {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RateVisitor;
        impl Visitor<'_> for RateVisitor {
            type Value = CpuRate;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a positive integer or an \"A/B\" ratio with positive integer parts")
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                match NonZeroU64::new(value) {
                    Some(steps) => Ok(CpuRate {
                        steps,
                        pit_clocks: NonZeroU64::MIN,
                    }),
                    None => Err(E::invalid_value(de::Unexpected::Unsigned(value), &self)),
                }
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                match u64::try_from(value) {
                    Ok(number) => self.visit_u64(number),
                    Err(_) => Err(E::invalid_value(de::Unexpected::Signed(value), &self)),
                }
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                let positive = |part: &str| {
                    if part.bytes().all(|byte| byte.is_ascii_digit()) {
                        part.parse::<NonZeroU64>().ok()
                    } else {
                        None
                    }
                };
                if let Some((a, b)) = value.split_once('/') {
                    if let (Some(steps), Some(pit_clocks)) = (positive(a), positive(b)) {
                        return Ok(CpuRate { steps, pit_clocks });
                    }
                }
                Err(E::invalid_value(de::Unexpected::Str(value), &self))
            }
        }
        deserializer.deserialize_any(RateVisitor)
    }
}

/// Either derived/blank NVRAM or an explicit image table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nvram {
    Derived,
    Blank,
    Image(NvramImage),
}

impl<'de> Deserialize<'de> for Nvram {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NvramVisitor;
        impl<'de> Visitor<'de> for NvramVisitor {
            type Value = Nvram;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("\"derived\", \"blank\" or an NVRAM image table")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "derived" => Ok(Nvram::Derived),
                    "blank" => Ok(Nvram::Blank),
                    _ => Err(E::unknown_variant(value, &["derived", "blank"])),
                }
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                NvramImage::deserialize(MapAccessDeserializer::new(map)).map(Nvram::Image)
            }
        }
        deserializer.deserialize_any(NvramVisitor)
    }
}

/// Explicit geometry; the string form is recognised only to diagnose `"auto"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Geometry {
    Chs(Chs),
}

impl<'de> Deserialize<'de> for Geometry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct GeometryVisitor;
        impl<'de> Visitor<'de> for GeometryVisitor {
            type Value = Geometry;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a CHS geometry table")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value == "auto" {
                    Err(E::custom(
                        "geometry `auto` is reserved, not implemented (schema 1): automatic disk geometry detection is not implemented",
                    ))
                } else {
                    Err(E::invalid_value(de::Unexpected::Str(value), &self))
                }
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                Chs::deserialize(MapAccessDeserializer::new(map)).map(Geometry::Chs)
            }
        }
        deserializer.deserialize_any(GeometryVisitor)
    }
}

/// A syntactically valid SHA-256 pin in lowercase hexadecimal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256(String);

impl Sha256 {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Sha256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(de::Error::custom(
                "sha256 must contain exactly 64 hexadecimal characters",
            ))
        }
    }
}

macro_rules! rejected_value {
    ($name:ident, $message:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub(super) enum $name {}
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                Err(de::Error::custom($message))
            }
        }
    };
}

rejected_value!(CycleExact, "cycle-exact timing is not modelled; `[cpu] rate` is a ratio of completed steps per PIT input clock");
rejected_value!(Reserved, "reserved, not implemented (schema 1)");
rejected_value!(OutsideMachine, "this option belongs outside the machine file; use run flags for run policy, window flags for presentation, and run-side host stream bindings");

// Calling the inner decoder directly prevents Option from accepting YAML null.
pub(super) fn reject_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

macro_rules! reserved_field {
    ($($function:ident => $reason:literal),* $(,)?) => {
        $(pub(super) fn $function<'de, D: Deserializer<'de>>(_: D) -> Result<Option<Reserved>, D::Error> {
            Err(de::Error::custom(concat!("reserved, not implemented (schema 1): ", $reason)))
        })*
    };
}

reserved_field! {
    reject_a20 => "selectable A20 gate control is not modelled",
    reject_kbc => "selectable AT or PS/2 keyboard controllers are not modelled",
    reject_keyboard => "selectable XT, AT or enhanced keyboard models are not implemented",
    reject_keyboard_layout => "keyboard layout belongs to a future keyboard model",
    reject_dip_switches => "XT DIP switches need an XT motherboard model",
    reject_chipset => "chipset register overrides need a chipset model",
    reject_speaker => "a speaker model is not implemented",
    reject_shadow => "ROM shadowing is not modelled",
    reject_umb => "upper-memory block configuration is not implemented",
    reject_upper => "selectable upper-memory RAM or open-bus behaviour is not implemented",
    reject_basic => "ROM BASIC firmware is not implemented",
    reject_monitor => "monitor colour selection is a future display-card feature",
    reject_rom => "card-owned option ROM declarations are not implemented",
    reject_mmio => "card MMIO window relocation is not implemented",
}

pub(super) fn rtc_start<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value == "host" {
        Err(de::Error::custom("RTC start `host` belongs outside the machine file; use the run option --rtc-start host"))
    } else {
        Ok(Some(value))
    }
}

pub(super) fn drive_image<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value == "blank" {
        Err(de::Error::custom(
            "drive image `blank` is reserved, not implemented (schema 1): creating blank drive images is not implemented",
        ))
    } else {
        Ok(Some(value))
    }
}

/// Decode an explicit list; null is not an omitted field.
pub(super) fn media_list<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<super::Medium>>, D::Error> {
    Vec::<super::Medium>::deserialize(deserializer).map(Some)
}

named_values! {
    /// Supported UART mouse endpoint; none explicitly disables inherited attachment.
    SerialMouseModel, "serial mouse" { Microsoft => "microsoft", None => "none" } reserved {}
}
