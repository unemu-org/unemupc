// SPDX-License-Identifier: AGPL-3.0-or-later
//! Lossless schema-1 field projection after inheritance; machine-images §2.
//! Default values are materialized from the existing lowering implementation.

use crate::schema::v1::projection;
use crate::schema::v1::*;
use toml::{Table, Value};

/// Retain schema-owned spellings without introducing another document parser.
trait Field {
    fn value(&self) -> Value;
}
impl Field for String {
    fn value(&self) -> Value {
        self.clone().into()
    }
}
impl Field for bool {
    fn value(&self) -> Value {
        (*self).into()
    }
}
macro_rules! number {
    ($($ty:ty),*) => {$(impl Field for $ty {
        fn value(&self) -> Value { i64::from(*self).into() }
    })*};
}
number!(u8, u16, u32);
impl Field for usize {
    fn value(&self) -> Value {
        Value::Integer(*self as i64)
    }
}
impl Field for Sha256 {
    fn value(&self) -> Value {
        self.as_str().into()
    }
}
impl Field for CpuRate {
    fn value(&self) -> Value {
        format!("{}/{}", self.steps, self.pit_clocks).into()
    }
}
impl<const MAX: u16> Field for NumberOrNone<MAX> {
    fn value(&self) -> Value {
        match self {
            Self::Number(n) => i64::from(*n).into(),
            Self::None => "none".into(),
        }
    }
}
impl<T: Field> Field for Vec<T> {
    fn value(&self) -> Value {
        Value::Array(self.iter().map(Field::value).collect())
    }
}
macro_rules! names {
    ($ty:ident { $($variant:ident => $name:expr),* $(,)? }) => {
        impl Field for $ty {
            fn value(&self) -> Value { match self { $(Self::$variant => Value::from($name)),* } }
        }
    };
}
names!(BoardModel { Ibm5170 => "ibm-5170", Generic386Isa => "generic-386-isa", Generic486Isa => "generic-486-isa" });
names!(CpuModel { I80286 => "80286", I80386 => "80386dx", I80486Dx => "80486dx", I80486DxCpuid => "80486dx-cpuid" });
names!(FpuModel { None => "none", I80287 => "80287", Integrated => "integrated" });
names!(DisplaySwitch { Auto => "auto", Mono => "mono", Colour => "colour" });
names!(CardKind { Mda => "mda", VgaText => "vga-text", Vga => "vga", Uart16550 => "uart-16550", Fdc765 => "fdc-765", Ide => "ide", Sb16 => "sb16", LotechEms => "lotech-ems" });
names!(ControllerModel { Ata => "ata", AtAdapter => "at-adapter" });
names!(Channel { Master => "master", Slave => "slave" });
names!(WriteMode { Volatile => "volatile", ReadOnly => "readonly", Journal => "journal" });
names!(DriveType { Hd144 => "1.44M", Hd12 => "1.2M", Dd720 => "720K", Dd360 => "360K" });
names!(FirmwareService { Floppy => "floppy", FixedDisk => "fixed-disk" });
names!(Lane { Even => "even", Odd => "odd" });
names!(SerialMouseModel { Microsoft => "microsoft", None => "none" });
fn put<T: Field>(t: &mut Table, key: &str, value: &Option<T>) {
    if let Some(value) = value {
        t.insert(key.into(), value.value());
    }
}
macro_rules! table {
    ($ty:ident { $($field:ident),* $(,)? } $(ignore { $($ignored:ident),* $(,)? })?) => {
        impl Field for $ty {
            fn value(&self) -> Value {
                let projection::$ty { $($field,)* $($($ignored: _,)*)? } = projection::$ty::from(self);
                let mut t = Table::new();
                $(put(&mut t, stringify!($field), $field);)*
                Value::Table(t)
            }
        }
    };
}
table!(Board {
    model,
    display_switch,
    post_port,
    rtc,
    mouse
});
table!(Cpu { model, rate, fpu });
table!(Memory {
    conventional_kib,
    extended_kib
});
table!(Rtc { start, nvram });
table!(Firmware { bios, option_roms });
table!(Rom {
    offset,
    parts,
    image,
    sha256,
    base,
    provides
});
table!(Chs {
    cylinders,
    heads,
    sectors
});
table!(Card {
    id,
    kind,
    controller_model,
    mouse,
    port,
    frame,
    size_kib,
    base,
    dma8,
    dma16,
    irq,
    dma,
    drives
} ignore { remove });
impl Field for Geometry {
    fn value(&self) -> Value {
        match self {
            Self::Chs(chs) => chs.value(),
        }
    }
}
impl Field for RomPart {
    fn value(&self) -> Value {
        let RomPart {
            image,
            lane,
            offset,
            sha256,
        } = self;
        let mut t = Table::new();
        t.insert("image".into(), image.value());
        t.insert("lane".into(), lane.value());
        t.insert("offset".into(), Value::Integer(*offset as i64));
        put(&mut t, "sha256", sha256);
        t.into()
    }
}
impl Field for Nvram {
    fn value(&self) -> Value {
        match self {
            Self::Derived => "derived".into(),
            Self::Blank => "blank".into(),
            Self::Image(NvramImage {
                image,
                sha256,
                write,
                journal,
                keep_generations,
            }) => {
                let mut t = Table::new();
                put(&mut t, "image", image);
                put(&mut t, "sha256", sha256);
                put(&mut t, "write", write);
                put(&mut t, "journal", journal);
                if let Some(n) = *keep_generations {
                    t.insert("keep_generations".into(), i64::from(n.get()).into());
                }
                t.into()
            }
        }
    }
}
impl Field for Drive {
    fn value(&self) -> Value {
        let mut t = Table::new();
        let projection::Drive {
            unit,
            channel,
            drive_type,
            geometry,
            image: _,
            sha256: _,
            write: _,
            journal: _,
            keep_generations: _,
            remove: _,
            media: _,
            empty: _,
        } = projection::Drive::from(self);
        put(&mut t, "unit", unit);
        put(&mut t, "channel", channel);
        put(&mut t, "type", drive_type);
        put(&mut t, "geometry", geometry);
        t.into()
    }
}

/// The hardware tables of a resolved machine document, with each drive table
/// in its controller-local unit order: the `machine` table of a stored config.
pub fn machine(document: &MachineFile) -> Table {
    let projection::MachineFile {
        schema: _,
        name: _,
        description: _,
        extends: _,
        board,
        cpu,
        memory,
        firmware,
        card,
    } = projection::MachineFile::from(document);
    let mut t = Table::new();
    t.insert("schema".into(), 1.into());
    put(&mut t, "board", board);
    put(&mut t, "cpu", cpu);
    put(&mut t, "memory", memory);
    put(&mut t, "firmware", firmware);
    put(&mut t, "card", card);
    // The loader wires drives by controller-local unit, regardless of declaration order.
    if let Some(cards) = t.get_mut("card").and_then(Value::as_array_mut) {
        for card in cards {
            if let Some(drives) = card.get_mut("drives").and_then(Value::as_array_mut) {
                drives.sort_by_key(|drive| {
                    drive
                        .get("unit")
                        .and_then(Value::as_integer)
                        .unwrap_or_else(|| {
                            i64::from(drive.get("channel").and_then(Value::as_str) == Some("slave"))
                        })
                });
            }
        }
    }
    t
}
