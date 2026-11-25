// SPDX-License-Identifier: AGPL-3.0-or-later
//! Private DOOM qualification; see user-media.md. ATA-2, legacy task file;
//! IBM VGA/XGA Technical Reference, BIOS mode 13h. No media or frame digests
//! leave this process.
//! Creative Sound Blaster Series Hardware Programming Guide, §2 DSP ports,
//! §3 DMA output; Intel 8237A datasheet, Current Address and Word Count Registers.

use super::{budget, freedos_config, freedos_text, games_config};
use assembly::{CpuChip, DeviceId};
use cpu::Cpu;
use device::{AccessDir, Device, TraceEvent};
use std::{path::PathBuf, process::Command, time::Instant};

struct PrivateDisk(PathBuf);

impl Drop for PrivateDisk {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

const SOUND_CONFIG: &str = "snd_sfxdevice 3\nsnd_musicdevice 3\nsnd_sbport 544\nsnd_sbirq 5\nsnd_sbdma 1\nsnd_mport -1\nsnd_channels 3\n";

fn configure_sound(disk: &PrivateDisk) {
    let path = disk.0.with_file_name("DEFAULT.CFG");
    std::fs::write(&path, SOUND_CONFIG).unwrap();
    let copied = Command::new("mcopy")
        .args(["-o", "-i"])
        .arg(format!("{}@@16384", disk.0.display()))
        .arg(&path)
        .arg("::DEFAULT.CFG")
        .env("MTOOLSRC", "/dev/null")
        .env_remove("MTOOLS_DATE_STRING")
        .env("SOURCE_DATE_EPOCH", "315532800")
        .env("TZ", "UTC")
        .output()
        .expect("start generated sound configuration copy");
    assert!(copied.status.success(), "sound configuration copy failed");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn generated_sound_configuration_replaces_defaults_without_touching_source() {
    use sha2::{Digest, Sha256};
    let source = machine_file::test_support::TempDir::new("synthetic-sound-media");
    std::fs::write(source.path().join("DOOM.EXE"), b"synthetic executable").unwrap();
    std::fs::write(source.path().join("DOOM.WAD"), b"synthetic data").unwrap();
    let defaults = source.path().join("DEFAULT.CFG");
    std::fs::write(&defaults, b"snd_sfxdevice 0\n").unwrap();
    let mut digests = Vec::new();
    for _ in 0..2 {
        let built = Command::new("bash")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/user-media-disk.sh"))
            .env("UNEMUPC_USER_DOOM_DIR", source.path())
            .output()
            .unwrap();
        assert!(built.status.success());
        let disk = PrivateDisk(PathBuf::from(
            String::from_utf8(built.stdout).unwrap().trim(),
        ));
        configure_sound(&disk);
        let read = Command::new("mtype")
            .arg("-i")
            .arg(format!("{}@@16384", disk.0.display()))
            .arg("::DEFAULT.CFG")
            .env("MTOOLSRC", "/dev/null")
            .output()
            .unwrap();
        assert!(read.status.success());
        assert_eq!(read.stdout, SOUND_CONFIG.as_bytes());
        digests.push(Sha256::digest(std::fs::read(&disk.0).unwrap()));
    }
    assert_eq!(
        digests[0], digests[1],
        "generated sound media must reproduce"
    );
    assert_eq!(std::fs::read(defaults).unwrap(), b"snd_sfxdevice 0\n");
}

fn dma_position(assembled: &assembly::AssembledMachine) -> (u32, u16) {
    let snapshot = assembled
        .machine
        .bus()
        .device(assembled.device_index[&DeviceId::Dma])
        .unwrap()
        .save();
    let mut dma = devices::Dma8237::new();
    dma.restore(&snapshot).unwrap();
    (dma.physical_address(1), dma.current_count(1))
}

#[derive(Default, Debug)]
struct SoundActivity {
    dsp_writes: u64,
    opl_writes: u64,
    dma_advanced: bool,
}

impl SoundActivity {
    fn observe_ports(&mut self, events: &[TraceEvent]) {
        for event in events {
            if let TraceEvent::Port {
                port,
                dir: AccessDir::Write,
                ..
            } = event
            {
                match port {
                    0x22C => self.dsp_writes += 1,
                    0x221 | 0x223 | 0x229 | 0x389 | 0x38B => self.opl_writes += 1,
                    _ => {}
                }
            }
        }
    }

    fn complete(&self) -> bool {
        self.dsp_writes > 0 && self.opl_writes > 0 && self.dma_advanced
    }
}

pub fn doom(chip: CpuChip, sound: bool) {
    if std::env::var_os("UNEMUPC_USER_DOOM_DIR").is_none() {
        eprintln!("skipping: set UNEMUPC_USER_DOOM_DIR to an extracted DOOM directory");
        return;
    }
    let Some(mut config) = (if sound {
        games_config(chip)
    } else {
        freedos_config(chip, true)
    }) else {
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let built = Command::new("bash")
        .arg(root.join("scripts/user-media-disk.sh"))
        .output()
        .expect("start user-media builder");
    assert!(built.status.success(), "user-media builder failed");
    let disk = PrivateDisk(PathBuf::from(
        String::from_utf8(built.stdout).unwrap().trim(),
    ));
    if sound {
        // Complete the generated base before hashing or mounting it. Guest
        // configuration writes subsequently belong to the volatile overlay.
        configure_sound(&disk);
    }
    let bytes = std::fs::metadata(&disk.0).unwrap().len();
    let drive = config
        .drives
        .iter_mut()
        .find(|d| matches!(d.kind, assembly::DriveKind::HardDisk { .. }))
        .unwrap();
    drive.kind = assembly::DriveKind::HardDisk {
        geometry: assembly::HardDiskGeometry {
            cylinders: (bytes / (4 * 32 * 512)) as u32,
            heads: 4,
            sectors_per_track: 32,
            bytes_per_sector: 512,
        },
    };
    use sha2::Digest;
    // The overlay still identifies its immutable base; its digest stays private.
    drive.expected_sha256 = Some(sha2::Sha256::digest(std::fs::read(&disk.0).unwrap()).into());
    drive.image = assembly::ImageSource::File(disk.0.clone());
    drive.write = assembly::WriteMode::Volatile;
    let policy = assembly::ObservationPolicy {
        trace: device::TracePolicy::Ring { capacity: 16 },
        routes: vec![assembly::ObservationRequest {
            device: DeviceId::Vga,
            output: assembly::OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    };
    let mut assembled = machine_file::assemble_with_observation(&config, &policy)
        .unwrap_or_else(|_| panic!("private media assembly failed"));
    if sound {
        // DSP busy-polling can fill a small ring between instruction polls.
        // Preserve the whole interval on the two evidence-producing devices.
        for id in [DeviceId::Sb16, DeviceId::Dma] {
            assembled
                .machine
                .bus_mut()
                .device_mut(assembled.device_index[&id])
                .unwrap()
                .set_trace_policy(device::TracePolicy::Ring { capacity: 65_536 });
        }
    }
    assembled.machine.enable_delivery_log().unwrap();
    let keyboard = assembled.device_index[&DeviceId::Keyboard];
    let started = Instant::now();
    let mut entered = false;
    let mut stage = 0;
    let mut last_io = None;
    let identifiers = [
        "DOOM",
        "V_Init",
        "M_LoadDefaults",
        "Z_Init",
        "W_Init",
        "R_Init",
    ];
    let mut seen = [false; 6];
    let mut dos4gw = false;
    let mut graphics = false;
    let mut launch_retired = 0;
    let mut next_progress = 40_000_000;
    let mut activity = SoundActivity::default();
    // This host probe reports wall time; only the fixed instruction allowance
    // controls guest execution. It never prints the private guest screen.
    for cap in (100_000..=budget(chip, 45_000_000)).step_by(100_000) {
        let dma_before = sound.then(|| dma_position(&assembled));
        let result = assembled.machine.run_bounded(Some(cap));
        let mut events = assembled.machine.bus_mut().take_open_bus_trace();
        let mut dma_programmed = false;
        for (id, index) in &assembled.device_index {
            let device = assembled.machine.bus_mut().device_mut(*index).unwrap();
            let trace = device.take_trace();
            if sound && matches!(id, DeviceId::Sb16 | DeviceId::Dma) {
                assert_eq!(device.trace_dropped(), 0, "sound evidence trace overflowed");
                if *id == DeviceId::Sb16 {
                    activity.observe_ports(&trace);
                } else {
                    dma_programmed = trace.iter().any(|event| {
                        matches!(
                            event,
                            TraceEvent::Port {
                                dir: AccessDir::Write,
                                ..
                            }
                        )
                    });
                }
            }
            events.extend(trace);
        }
        if let Some(before) = dma_before {
            // A channel's address/count can advance without CPU programming
            // only when the DMA controller services a transfer. Restoring a
            // snapshot into a separate chip observes this without bus reads.
            if !dma_programmed && before != dma_position(&assembled) {
                activity.dma_advanced = true;
            }
        }
        if let Some(event) = events
            .into_iter()
            .filter(|e| matches!(e, device::TraceEvent::Port { .. }))
            .max_by_key(|e| match e {
                device::TraceEvent::Port { at, .. } => *at,
                _ => unreachable!(),
            })
        {
            last_io = Some(event);
        }
        if let Err(error) = result {
            let state = assembled.machine.cpu().diagnostic_state();
            eprintln!("FINDINGS: {error}; CS:IP={:04x}:{:08x}; linear={:08x}; system={:?}; retired={}; wall={:?}; last port I/O={last_io:?}; deliveries={:?}", state.cs().selector, assembled.machine.cpu().reg(cpu::CpuReg::Rip), state.cs().base.wrapping_add(assembled.machine.cpu().reg(cpu::CpuReg::Rip) as u32), state.system, assembled.machine.cpu().retired(), started.elapsed(), assembled.machine.recent_deliveries().map(|log| log.entries.last().cloned()));
            panic!("DOOM stopped before graphics qualification");
        }
        let text = freedos_text(&assembled);
        if !entered && text.contains("Enter new date") {
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x1c, 0x9c, 0x1c, 0x9c]);
            entered = true;
        }
        if stage == 0 && text.contains("A:\\>") {
            eprintln!(
                "{} FreeDOS prompt: retired={}; wall={:?}",
                chip.name(),
                assembled.machine.cpu().retired(),
                started.elapsed()
            );
            assembly::inject_scancodes(
                &mut assembled.machine,
                keyboard,
                &[0x2e, 0xae, 0x2a, 0x27, 0xa7, 0xaa, 0x1c, 0x9c],
            );
            stage = 1;
        } else if stage == 1 && text.contains("C:\\>") {
            let keys = [0x20, 0x18, 0x18, 0x32, 0x1c];
            let keys: Vec<_> = keys.into_iter().flat_map(|key| [key, key | 0x80]).collect();
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &keys);
            launch_retired = assembled.machine.cpu().retired();
            eprintln!(
                "{} DOOM command: retired={launch_retired}; wall={:?}",
                chip.name(),
                started.elapsed()
            );
            stage = 2;
        }
        if stage != 2 {
            continue;
        }
        for (index, identifier) in identifiers.iter().enumerate() {
            if !seen[index] && text.contains(identifier) && (index != 0 || text.contains("V_Init"))
            {
                seen[index] = true;
                eprintln!(
                    "{} {identifier}: retired={}; wall={:?}",
                    chip.name(),
                    assembled.machine.cpu().retired(),
                    started.elapsed()
                );
            }
        }
        if !dos4gw && text.contains("DOS/4G") {
            dos4gw = true;
            eprintln!(
                "{} DOS/4G banner: retired={}; wall={:?}",
                chip.name(),
                assembled.machine.cpu().retired(),
                started.elapsed()
            );
        }
        if let video::Frame::Pixels(pixels) = assembled.capture_text(DeviceId::Vga).unwrap().frame {
            if (pixels.width, pixels.height) == (320, 200) {
                let mut bda_mode = [0];
                assembled
                    .machine
                    .cpu()
                    .read_mem(0x449, &mut bda_mode)
                    .unwrap();
                if !graphics {
                    graphics = true;
                    eprintln!(
                        "{} graphics 320x200: BIOS mode={:02x}; retired={}; wall={:?}",
                        chip.name(),
                        bda_mode[0],
                        assembled.machine.cpu().retired(),
                        started.elapsed()
                    );
                }
                let mut palette = [false; 256];
                for index in pixels.indices {
                    palette[index as usize] = true;
                }
                let distinct = palette.into_iter().filter(|v| *v).count();
                if distinct >= 32 && (!sound || activity.complete()) {
                    assert_eq!(bda_mode[0], 0x13);
                    assert!(
                        seen.into_iter().all(|value| value),
                        "missing startup identifiers"
                    );
                    eprintln!("{} DOOM picture: distinct indices={distinct}; retired={}; wall={:?}; DOS/4G banner={dos4gw}", chip.name(), assembled.machine.cpu().retired(), started.elapsed());
                    if sound {
                        eprintln!("{} DOOM sound: {activity:?}", chip.name());
                    }
                    return;
                }
            }
        }
        if assembled.machine.cpu().retired() >= next_progress {
            let state = assembled.machine.cpu().diagnostic_state();
            eprintln!("{} progress: CS:IP={:04x}:{:08x}; linear={:08x}; system={:?}; retired={}; wall={:?}", chip.name(), state.cs().selector, assembled.machine.cpu().reg(cpu::CpuReg::Rip), state.cs().base.wrapping_add(assembled.machine.cpu().reg(cpu::CpuReg::Rip) as u32), state.system, assembled.machine.cpu().retired(), started.elapsed());
            next_progress += 40_000_000;
        }
        if assembled
            .machine
            .cpu()
            .retired()
            .saturating_sub(launch_retired)
            >= 250_000_000
        {
            break;
        }
    }
    let retired = assembled.machine.cpu().retired();
    if sound {
        eprintln!("FINDINGS sound: {activity:?}");
    }
    let state = assembled.machine.cpu().diagnostic_state();
    eprintln!(
        "FINDINGS location: CS:IP={:04x}:{:08x}; linear={:08x}; system={:?}; deliveries={:?}",
        state.cs().selector,
        assembled.machine.cpu().reg(cpu::CpuReg::Rip),
        state
            .cs()
            .base
            .wrapping_add(assembled.machine.cpu().reg(cpu::CpuReg::Rip) as u32),
        state.system,
        assembled
            .machine
            .recent_deliveries()
            .map(|log| log.entries.last().cloned())
    );
    eprintln!("FINDINGS: instruction budget exhausted; retired={retired}; wall={:?}; rate={:.0} instructions/s; startup={seen:?}; graphics={graphics}; last port I/O={last_io:?}", started.elapsed(), retired as f64 / started.elapsed().as_secs_f64());
    panic!("DOOM did not reach a non-trivial mode 13h picture within the bounded probe");
}
