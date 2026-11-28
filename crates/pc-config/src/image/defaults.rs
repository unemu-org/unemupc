// SPDX-License-Identifier: AGPL-3.0-or-later
//! Freeze schema-1 lowering defaults into image identity; machine-images §2.
//! IBM Personal Computer AT Technical Reference, System Board: defaults come
//! from the lowered configuration rather than a second set of hardware constants.

use super::{invalid, Result};
use assembly::{DeviceId, DeviceKind};
use toml::{Table, Value};

fn table<'a>(parent: &'a mut Table, key: &str) -> Result<&'a mut Table> {
    parent
        .entry(key)
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| invalid("hardware table required"))
}
fn line(value: Option<u8>) -> Value {
    value.map_or_else(|| "none".into(), |n| i64::from(n).into())
}

/// Write into `hardware` every default the lowering chose for `config`, so
/// the stored tables name them and a later lowering cannot choose others.
pub fn materialize(hardware: &mut Table, config: &assembly::MachineConfig) -> Result<()> {
    let board = table(hardware, "board")?;
    board.insert(
        "display_switch".into(),
        match config.display_switch {
            assembly::DisplaySwitch::Auto => "auto",
            assembly::DisplaySwitch::Mono => "mono",
            assembly::DisplaySwitch::Colour => "colour",
        }
        .into(),
    );
    let post = config.devices.iter().find_map(|d| {
        if let DeviceKind::Post(p) = d.kind {
            Some(p.port)
        } else {
            None
        }
    });
    board.insert(
        "post_port".into(),
        post.map_or_else(|| "none".into(), |p| i64::from(p).into()),
    );
    if let Some(mouse) = config.devices.iter().find_map(|d| {
        if let DeviceKind::Keyboard(k) = d.kind {
            Some(k.mouse)
        } else {
            None
        }
    }) {
        board.insert("mouse".into(), mouse.into());
    }
    if let Some(rtc) = config.devices.iter().find_map(|d| {
        if let DeviceKind::Rtc(r) = &d.kind {
            Some(r)
        } else {
            None
        }
    }) {
        let target = table(board, "rtc")?;
        let r = rtc.start;
        target.insert(
            "start".into(),
            format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                r.year, r.month, r.day, r.hour, r.minute, r.second
            )
            .into(),
        );
        // Metadata lowering substitutes blank CMOS for unread images, so only
        // fill an omitted source and preserve any explicit image binding.
        match rtc.nvram {
            assembly::NvramSource::Derived => {
                target.entry("nvram").or_insert_with(|| "derived".into());
            }
            assembly::NvramSource::Blank => {
                target.entry("nvram").or_insert_with(|| "blank".into());
            }
            _ => {}
        }
    }
    let cpu = table(hardware, "cpu")?;
    cpu.insert(
        "model".into(),
        match config.cpu.chip {
            assembly::CpuChip::I80286 => "80286",
            assembly::CpuChip::I80386 => "80386dx",
            assembly::CpuChip::I80486Dx => "80486dx",
            assembly::CpuChip::I80486DxCpuid => "80486dx-cpuid",
        }
        .into(),
    );
    if !cpu.contains_key("clock_mhz") {
        cpu.insert(
            "rate".into(),
            format!(
                "{}/{}",
                config.clock.retired_per_pit_clock, config.clock.retired_per_pit_clock_denominator
            )
            .into(),
        );
    }
    cpu.insert(
        "fpu".into(),
        match config.cpu.fpu {
            assembly::FpuChip::None => "none",
            assembly::FpuChip::I80287 => "80287",
            assembly::FpuChip::I80387 => "80387",
            assembly::FpuChip::Integrated => "integrated",
        }
        .into(),
    );
    let memory = table(hardware, "memory")?;
    memory
        .entry("conventional_kib")
        .or_insert_with(|| i64::from(assembly::descriptors::CONVENTIONAL_KIB.default).into());
    memory.insert(
        "extended_kib".into(),
        ((config.memory.address_space_bytes - 0x100000) / 1024)
            .try_into()
            .map(Value::Integer)
            .map_err(|_| invalid("memory size out of range"))?,
    );
    if let Some(cards) = hardware.get_mut("card").and_then(Value::as_array_mut) {
        for card in cards {
            let card = card
                .as_table_mut()
                .ok_or_else(|| invalid("card table required"))?;
            let kind = card
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("card kind required"))?
                .to_owned();
            let id = match kind.as_str() {
                "mda" => DeviceId::Mda,
                "vga-text" | "vga" => DeviceId::Vga,
                "uart-16550" => DeviceId::Uart,
                "fdc-765" => DeviceId::Fdc,
                "ide" => DeviceId::Ide,
                "sb16" => DeviceId::Sb16,
                "lotech-ems" => DeviceId::LotechEms,
                _ => return Err(invalid("unsupported card kind")),
            };
            let d = config
                .devices
                .iter()
                .find(|d| d.id == id)
                .ok_or_else(|| invalid("missing lowered card"))?;
            match &d.kind {
                DeviceKind::Uart(c) => {
                    card.insert("port".into(), i64::from(c.base).into());
                    card.insert(
                        "mouse".into(),
                        if c.mouse { "microsoft" } else { "none" }.into(),
                    );
                    card.insert("irq".into(), line(d.irq));
                }
                DeviceKind::Ide(c) => {
                    card.insert("port".into(), i64::from(c.base).into());
                    card.insert(
                        "controller_model".into(),
                        match c.model {
                            assembly::AtaControllerModel::Ata => "ata",
                            assembly::AtaControllerModel::AtAdapter => "at-adapter",
                        }
                        .into(),
                    );
                    card.insert("irq".into(), line(d.irq));
                }
                DeviceKind::Sb16(c) => {
                    card.insert("base".into(), i64::from(c.base).into());
                    card.insert("dma16".into(), i64::from(c.dma16).into());
                    card.insert(
                        "dma8".into(),
                        i64::from(d.dma.ok_or_else(|| invalid("missing SB16 DMA"))?.channel).into(),
                    );
                    card.insert("irq".into(), line(d.irq));
                }
                DeviceKind::Fdc(_) => {
                    card.insert("irq".into(), line(d.irq));
                    card.insert("dma".into(), line(d.dma.map(|d| d.channel)));
                }
                DeviceKind::BochsVbe(c) => {
                    card.insert("model".into(), "bochs-vbe".into());
                    card.insert("memory_mib".into(), i64::from(c.memory_mib).into());
                }
                DeviceKind::LotechEms(c) => {
                    card.insert("port".into(), i64::from(c.port).into());
                    card.insert("frame".into(), i64::from(c.frame).into());
                    card.insert("size_kib".into(), i64::from(c.size_kib).into());
                }
                _ => {}
            }
        }
    }
    let mut image_index = 0;
    if let Some(firmware) = hardware.get_mut("firmware").and_then(Value::as_table_mut) {
        if let Some(rom) = firmware.get_mut("bios").and_then(Value::as_table_mut) {
            materialize_rom(rom, &config.images[image_index], true)?;
            image_index += 1;
        }
        for rom in firmware
            .get_mut("option_roms")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            materialize_rom(
                rom.as_table_mut()
                    .ok_or_else(|| invalid("ROM table required"))?,
                &config.images[image_index],
                false,
            )?;
            image_index += 1;
        }
    }
    Ok(())
}
fn materialize_rom(rom: &mut Table, image: &assembly::LoadImage, bios: bool) -> Result<()> {
    rom.insert(
        "base".into(),
        i64::try_from(image.base)
            .map_err(|_| invalid("ROM address out of range"))?
            .into(),
    );
    rom.insert(
        "provides".into(),
        image
            .provides
            .iter()
            .map(|s| s.to_string().into())
            .collect::<Vec<Value>>()
            .into(),
    );
    if !bios {
        rom.insert(
            "offset".into(),
            match &image.source {
                assembly::ImageSource::RomContainer { offset, .. } => *offset as i64,
                _ => 0,
            }
            .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn materializing_defaults_preserves_nominal_and_turbo_clocks_without_rate_conflict() {
        let text =
            "schema=1\n[board]\nmodel='ibm-5170'\n[cpu]\nclock_mhz=4.77\nturbo={clock_mhz=8}\n";
        let doc = crate::parse_toml("clock.toml", text).unwrap();
        let mut hardware = super::super::encode::machine(&doc);
        let mut config = assembly::bios(vec![0xf4; 32768], Some(vec![0; 1474560]));
        config.clock = assembly::ClockConfig::from_khz(
            assembly::CpuChip::I80286,
            std::num::NonZeroU32::new(4770).unwrap(),
        );
        super::materialize(&mut hardware, &config).unwrap();
        assert!(!hardware["cpu"].as_table().unwrap().contains_key("rate"));
        let restored =
            crate::parse_toml("resolved.toml", &toml::to_string(&hardware).unwrap()).unwrap();
        let cpu = restored.cpu.unwrap();
        let original = doc.cpu.unwrap();
        assert_eq!(cpu.clock_mhz, original.clock_mhz);
        assert_eq!(cpu.turbo, original.turbo);
    }
}
