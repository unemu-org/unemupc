// SPDX-License-Identifier: AGPL-3.0-or-later
//! Local Windows 95 startup on an Intel486 ISA board; user-media.md §Windows 95.
//! ATA-2, Command Register; IBM PS/2 Hardware Interface Technical Reference,
//! Common Interfaces, Keyboard and Auxiliary Device Controller;
//! IBM VGA/XGA Technical Reference, Video BIOS mode 12h;
//! Intel 8259A, ICW/OCW programming;
//! Intel 8237A, register set; NEC uPD765A, command phase.

use assembly::{DeviceId, DeviceKind, DriveKind, ImageSource, WriteMode};
use cpu::Cpu;
use device::{AccessDir, TraceEvent};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::PathBuf, process::Command, time::Instant};

#[path = "support/private_diagnostics.rs"]
mod private_diagnostics;

#[derive(Default, Debug)]
struct Ports {
    accesses: BTreeMap<&'static str, u64>,
    writes: BTreeMap<(&'static str, u16, u32), u64>,
    aux_data_next: bool,
    recent: Vec<String>,
}

impl Ports {
    fn observe(&mut self, mut events: Vec<TraceEvent>) {
        events.retain(|event| matches!(event, TraceEvent::Port { .. }));
        events.sort_by_key(|event| match event {
            TraceEvent::Port { at, .. } => *at,
            _ => unreachable!(),
        });
        for event in &events {
            let TraceEvent::Port {
                port, dir, value, ..
            } = event
            else {
                unreachable!()
            };
            let device = match port {
                0x1f0..=0x1f7 | 0x3f6 => "ATA",
                0x60 | 0x64 => "8042",
                0x20..=0x21 | 0xa0..=0xa1 => "PIC",
                0x00..=0x0f | 0x80..=0x8f | 0xc0..=0xde => "DMA",
                0x3f0..=0x3f5 | 0x3f7 => "FDC",
                _ => "other",
            };
            *self.accesses.entry(device).or_default() += 1;
            if *dir != AccessDir::Write {
                continue;
            }
            let command = match port {
                0x1f7 => Some("ATA command"),
                0x64 => {
                    self.aux_data_next = *value == 0xd4;
                    Some("8042 command")
                }
                0x60 if self.aux_data_next => {
                    self.aux_data_next = false;
                    Some("8042 aux data")
                }
                0x20 | 0xa0 if value & 0x10 != 0 => Some("PIC ICW1"),
                0x20 | 0xa0 if value & 0x18 == 8 => Some("PIC OCW3"),
                0x20 | 0x21 | 0xa0 | 0xa1 => Some("PIC other (ICW2-4/OCW1-2)"),
                _ if device == "DMA" || device == "FDC" => Some(device),
                _ => None,
            };
            if let Some(command) = command {
                *self.writes.entry((command, *port, *value)).or_default() += 1;
            }
        }
        // ATA data words can contain batch-file secrets. Retain access metadata
        // without retaining payloads in any printable diagnostic structure.
        self.recent.extend(events.iter().skip(events.len().saturating_sub(64)).map(|event| {
            match event {
                TraceEvent::Port { at, port: port @ (0x1f0 | 0x170), size, dir, .. } =>
                    format!("Port {{ at: {at:?}, port: {port:04x}, size: {size}, dir: {dir:?}, value: [payload omitted] }}"),
                _ => format!("{event:x?}"),
            }
        }));
        let excess = self.recent.len().saturating_sub(64);
        self.recent.drain(..excess);
    }
}

fn setup_keys() -> Vec<u8> {
    let mut keys = Vec::new();
    for c in "c:\\win95\\setup /is /iq c:\\msbatch.inf\n".chars() {
        let code = match c {
            'c' => 0x2e,
            ':' => {
                keys.push(0x2a);
                0x27
            }
            '\\' => 0x2b,
            'w' => 0x11,
            'i' => 0x17,
            'n' => 0x31,
            '9' => 0x0a,
            '5' => 0x06,
            's' => 0x1f,
            'e' => 0x12,
            't' => 0x14,
            'u' => 0x16,
            'p' => 0x19,
            ' ' => 0x39,
            '/' => 0x35,
            'q' => 0x10,
            'm' => 0x32,
            'b' => 0x30,
            'a' => 0x1e,
            'h' => 0x23,
            '.' => 0x34,
            'f' => 0x21,
            '\n' => 0x1c,
            _ => unreachable!(),
        };
        keys.extend([code, code | 0x80]);
        if c == ':' {
            keys.push(0xaa);
        }
    }
    keys
}

fn decoded_text(assembled: &assembly::AssembledMachine) -> String {
    match assembled.capture_text(DeviceId::Vga).unwrap().frame {
        video::Frame::Text(grid) => grid.rows_as_strings().join("\n"),
        _ => String::new(),
    }
}

fn first_screen(assembled: &assembly::AssembledMachine, text: &str) -> bool {
    let mut mode = [0];
    assembled.machine.cpu().read_mem(0x449, &mut mode).unwrap();
    if let video::Frame::Pixels(pixels) = assembled.capture_text(DeviceId::Vga).unwrap().frame {
        let mut counts = [0usize; 256];
        for index in &pixels.indices {
            counts[*index as usize] += 1;
        }
        let distinct = counts.iter().filter(|count| **count != 0).count();
        let minority = pixels.indices.len() - counts.into_iter().max().unwrap_or(0);
        return mode[0] == 0x12
            && (pixels.width, pixels.height) == (640, 480)
            && distinct >= 3
            && minority >= 4096;
    }
    // These short identifiers qualify a full-screen setup dialog, rather than
    // the shell's echoed command or the real-mode initialization banner.
    text.contains("Windows 95 Setup")
        && (text.contains("Continue") || text.contains("ENTER"))
        && text.lines().filter(|line| !line.trim().is_empty()).count() >= 8
}

#[test]
#[ignore = "local-only commercial media; set UNEMUPC_USER_WIN95_DIR and UNEMUPC_USER_DOS_FLOPPY"]
fn user_media_win95_stage1() {
    let (Some(_), Some(floppy)) = (
        std::env::var_os("UNEMUPC_USER_WIN95_DIR"),
        std::env::var_os("UNEMUPC_USER_DOS_FLOPPY"),
    ) else {
        eprintln!("skipping: set UNEMUPC_USER_WIN95_DIR and UNEMUPC_USER_DOS_FLOPPY");
        return;
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let built = Command::new("bash")
        .arg(root.join("scripts/user-media-disk.sh"))
        .arg("--win95")
        .output()
        .expect("start private disk builder");
    assert!(built.status.success(), "private disk builder failed");
    // Retained outside the checkout for the next stage, including on a stop.
    let image = PathBuf::from(String::from_utf8(built.stdout).unwrap().trim());
    let directory = image.parent().unwrap();
    let mut config = machine_file::load_for_assembly(
        root.join("machines/generic-486-ide.toml"),
        Default::default(),
    )
    .unwrap()
    .config;
    for device in &mut config.devices {
        if let DeviceKind::Keyboard(kbc) = &mut device.kind {
            kbc.mouse = true;
        }
    }
    for drive in &mut config.drives {
        let path = if matches!(drive.kind, DriveKind::HardDisk { .. }) {
            drive.kind = DriveKind::HardDisk {
                geometry: assembly::HardDiskGeometry {
                    cylinders: 1015,
                    heads: 16,
                    sectors_per_track: 63,
                    bytes_per_sector: 512,
                },
            };
            drive.write = WriteMode::journal(directory.join("ide.journal"));
            image.clone()
        } else {
            drive.write = WriteMode::ReadOnly;
            PathBuf::from(&floppy)
        };
        // Hash every byte, including sparse extents. Neither digest is printed.
        let mut file =
            std::fs::File::open(&path).unwrap_or_else(|_| panic!("private media open failed"));
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash).unwrap();
        drive.expected_sha256 = Some(hash.finalize().into());
        drive.image = ImageSource::File(path);
    }
    let policy = assembly::ObservationPolicy {
        trace: device::TracePolicy::Ring { capacity: 65_536 },
        routes: vec![assembly::ObservationRequest {
            device: DeviceId::Vga,
            output: assembly::OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..Default::default()
    };
    let mut assembled = machine_file::assemble_with_observation(&config, &policy)
        .unwrap_or_else(|_| panic!("private media assembly failed"));
    assembled.machine.enable_delivery_log().unwrap();
    let keyboard = assembled.device_index[&DeviceId::Keyboard];
    let mut ports = Ports::default();
    let started = Instant::now();
    let mut launched = None;
    let mut boot_menu_selected = false;
    let mut date_accepted = false;
    let mut next_progress = 25_000_000;
    let mut success = false;
    let mut execution_error = false;
    let reason;
    // The retired budget is independent of wall time. A second virtual-time
    // ceiling bounds HLT waits, which can consume no retired instructions.
    loop {
        let cap = assembled.machine.virtual_time() + 10_000;
        let result = assembled.machine.run_bounded(Some(cap));
        let mut events = assembled.machine.bus_mut().take_open_bus_trace();
        for index in assembled.device_index.values() {
            let device = assembled.machine.bus_mut().device_mut(*index).unwrap();
            assert_eq!(
                device.trace_dropped(),
                0,
                "private evidence trace overflowed"
            );
            events.extend(device.take_trace());
        }
        ports.observe(events);
        assembled
            .check_persistence()
            .expect("private journal output failed");
        let retired = assembled.machine.cpu().retired();
        let text = decoded_text(&assembled);
        match result {
            Err(error) => {
                reason = format!(
                    "execution stop: {}",
                    private_diagnostics::error_summary(&error)
                );
                execution_error = true;
                break;
            }
            Ok(machine::RunOutcome::Halted) => {
                reason = "CPU halted".into();
                break;
            }
            Ok(machine::RunOutcome::ResetRequested) => {
                reason = "guest reset requested".into();
                break;
            }
            _ => {}
        }
        if !boot_menu_selected && text.contains("Startup Menu") && text.contains("without CD-ROM") {
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x03, 0x83, 0x1c, 0x9c]);
            boot_menu_selected = true;
            eprintln!(
                "Windows 95 floppy no-CD boot option: retired={retired}; wall={:?}",
                started.elapsed()
            );
        }
        if !date_accepted && text.contains("Enter new date") {
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &[0x1c, 0x9c, 0x1c, 0x9c]);
            date_accepted = true;
        }
        if launched.is_none() && text.lines().any(|line| line.trim_end().ends_with("A:\\>")) {
            assembly::inject_scancodes(&mut assembled.machine, keyboard, &setup_keys());
            launched = Some(retired);
            eprintln!(
                "Windows 95 DOS prompt / setup command: retired={retired}; wall={:?}",
                started.elapsed()
            );
        } else if launched.is_some() && first_screen(&assembled, &text) {
            success = true;
            reason = "setup first screen".into();
            break;
        }
        if retired >= next_progress {
            eprintln!(
                "Windows 95 progress: retired={retired}; wall={:?}; command={}",
                started.elapsed(),
                launched.is_some()
            );
            next_progress += 25_000_000;
        }
        let budget = if launched.is_some() {
            300_000_000
        } else {
            200_000_000
        };
        if retired.saturating_sub(launched.unwrap_or(0)) >= budget
            || assembled.machine.virtual_time() >= 600_000_000
        {
            reason = "instruction/virtual-time budget exhausted".into();
            break;
        }
    }
    let state = assembled.machine.cpu().diagnostic_state();
    let system = state.system.unwrap();
    let mode = if system.virtual_8086 {
        "V86"
    } else if system.protected() {
        "protected"
    } else {
        "real"
    };
    let eip = assembled.machine.cpu().reg(cpu::CpuReg::Rip);
    let linear = state.cs().base.wrapping_add(eip as u32);
    eprintln!("FINDINGS: {reason}; command={}; mode={mode}; CS:EIP={:04x}:{:08x}; linear={:08x}; CPL={}; CR0={:08x}; retired={}; wall={:?}", launched.is_some(), state.cs().selector, eip, linear, system.cpl, system.cr0, state.retired, started.elapsed());
    eprintln!(
        "FINDINGS counters: device accesses={:?}; deliveries={}",
        ports.accesses,
        assembled
            .machine
            .recent_deliveries()
            .map_or(0, |log| log.entries.len())
    );
    if execution_error {
        // A CPU failure must not be published as a successfully finished session.
        // Checkpoint only completed sectors for diagnosis/restart; no CPU state
        // is resumed by a disk generation.
        for handle in &mut assembled.persistence {
            handle
                .checkpoint(&mut assembled.machine)
                .expect("private stop checkpoint failed");
        }
    } else {
        assembled
            .finish_persistence()
            .expect("private stage generation failed");
    }
    eprintln!(
        "Windows 95 private stage directory: {}",
        directory.display()
    );
    assert!(
        success,
        "Windows 95 stage 1 recorded its first stop before setup's first screen"
    );
}

#[test]
fn port_summary_keeps_aux_routing_across_poll_boundaries() {
    let event = |port, value| TraceEvent::Port {
        at: device::VClock(0),
        port,
        size: 1,
        dir: AccessDir::Write,
        value,
    };
    let mut ports = Ports::default();
    ports.observe(vec![
        event(0x64, 0xd4),
        event(0x1f7, 0xec),
        event(0x20, 0x11),
    ]);
    ports.observe(vec![
        event(0x60, 0xf4),
        event(0x20, 0x0b),
        event(0x3f5, 0x08),
    ]);
    assert_eq!(ports.writes[&("8042 aux data", 0x60, 0xf4)], 1);
    assert_eq!(ports.writes[&("ATA command", 0x1f7, 0xec)], 1);
    assert_eq!(ports.writes[&("PIC ICW1", 0x20, 0x11)], 1);
    assert_eq!(ports.writes[&("PIC OCW3", 0x20, 0x0b)], 1);
}

#[test]
fn private_port_diagnostics_cannot_reconstruct_disk_payloads() {
    let diagnostics = |payload: &[u8]| {
        let mut ports = Ports::default();
        for port in [0x1f0, 0x170] {
            for dir in [AccessDir::Read, AccessDir::Write] {
                for size in [1, 2, 4] {
                    // Stop in the middle of a transfer, with key bytes still in
                    // the recent history. Compare the whole printable summary.
                    ports.observe(
                        payload
                            .chunks(size)
                            .enumerate()
                            .map(|(index, bytes)| {
                                let mut word = [0; 4];
                                word[..bytes.len()].copy_from_slice(bytes);
                                TraceEvent::Port {
                                    at: device::VClock(index as u64 + 100),
                                    port,
                                    size: size as u8,
                                    dir,
                                    value: u32::from_le_bytes(word),
                                }
                            })
                            .collect(),
                    );
                }
            }
        }
        format!("{ports:x?}")
    };
    let private = diagnostics(b"SECRET-KEY");
    assert_eq!(
        private,
        diagnostics(b"OTHER-DATA"),
        "payload must not affect any diagnostic field"
    );
    for metadata in [
        "01f0",
        "0170",
        "Read",
        "Write",
        "size: 1",
        "size: 2",
        "size: 4",
        "VClock(100)",
    ] {
        assert!(
            private.contains(metadata),
            "missing port metadata: {metadata}"
        );
    }
}
