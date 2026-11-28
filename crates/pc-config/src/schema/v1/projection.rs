// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exhaustive public-field views of schema 1; machine-images §2.
//! Reserved fields remain private. New schema fields must be accounted for here
//! and in consumers instead of silently disappearing from resolved artifacts.

macro_rules! view {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? } ignore { $($private:ident),* $(,)? }) => {
        pub struct $name<'a> { $(pub $field: &'a $ty,)* }
        impl<'a> From<&'a super::$name> for $name<'a> {
            fn from(value: &'a super::$name) -> Self {
                let super::$name { $($field,)* $($private: _,)* } = value;
                Self { $($field,)* }
            }
        }
    };
}

view!(MachineFile {
    schema: super::SchemaVersion,
    name: Option<String>,
    description: Option<String>,
    extends: Option<String>,
    board: Option<super::Board>,
    cpu: Option<super::Cpu>,
    memory: Option<super::Memory>,
    firmware: Option<super::Firmware>,
    card: Option<Vec<super::Card>>,
    disks: Option<Vec<super::ZeroDisk>>,
} ignore { run, presentation, streams, mhz, frequency, clock_hz, cycles, cycle_exact, wait_states, bus_mhz, memory_wait_states, timing });

view!(Board {
    model: Option<super::BoardModel>,
    display_switch: Option<super::DisplaySwitch>,
    post_port: Option<super::NumberOrNone<65535>>,
    rtc: Option<super::Rtc>,
    mouse: Option<bool>,
} ignore { a20, kbc, keyboard, keyboard_layout, dip_switches, chipset, speaker, mhz, frequency, clock_hz, cycles, cycle_exact, wait_states, bus_mhz, memory_wait_states, timing });

view!(Cpu {
    model: Option<super::CpuModel>,
    rate: Option<super::CpuRate>,
    clock_mhz: Option<super::ClockMhz>,
    turbo: Option<super::Turbo>,
    fpu: Option<super::FpuModel>,
    engine: Option<super::CpuEngine>,
} ignore { mhz, frequency, clock_hz, cycles, cycle_exact, wait_states, bus_mhz, memory_wait_states, timing });

view!(Memory {
    conventional_kib: Option<u32>,
    extended_kib: Option<u32>,
} ignore { shadow, umb, upper });

view!(Rtc {
    start: Option<String>,
    nvram: Option<super::Nvram>,
} ignore {  });

view!(Firmware {
    bios: Option<super::Rom>,
    option_roms: Option<Vec<super::Rom>>,
} ignore { basic });

view!(Rom {
    use_id: Option<String>,
    rom: Option<String>,
    role: Option<String>,
    size: Option<u64>,
    offset: Option<usize>,
    parts: Option<Vec<super::RomPart>>,
    image: Option<String>,
    sha256: Option<super::Sha256>,
    base: Option<u32>,
    provides: Option<Vec<super::FirmwareService>>,
} ignore {  });

view!(Chs {
    cylinders: Option<u32>,
    heads: Option<u32>,
    sectors: Option<u32>,
} ignore {  });

view!(Card {
    id: Option<String>,
    kind: Option<super::CardKind>,
    controller_model: Option<super::ControllerModel>,
    model: Option<super::VgaModel>,
    memory_mib: Option<u32>,
    mouse: Option<super::SerialMouseModel>,
    port: Option<u16>,
    frame: Option<u16>,
    size_kib: Option<u32>,
    base: Option<u16>,
    dma8: Option<u8>,
    dma16: Option<u8>,
    irq: Option<super::NumberOrNone<15>>,
    dma: Option<super::NumberOrNone<3>>,
    drives: Option<Vec<super::Drive>>,
    remove: Option<bool>,
} ignore { monitor, rom, mmio, mhz, frequency, clock_hz, cycles, cycle_exact, wait_states, bus_mhz, memory_wait_states, timing });

view!(Drive {
    disk: Option<String>,
    inserted: Option<super::MediumSelector>,
    unit: Option<u8>,
    channel: Option<super::Channel>,
    drive_type: Option<super::DriveType>,
    image: Option<String>,
    sha256: Option<super::Sha256>,
    geometry: Option<super::Geometry>,
    write: Option<super::WriteMode>,
    journal: Option<String>,
    keep_generations: Option<std::num::NonZeroU32>,
    remove: Option<bool>,
    media: Option<Vec<super::Medium>>,
    empty: Option<bool>,
} ignore {  });
