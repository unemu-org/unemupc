// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use assembly::{ObservationPolicy, ObservationRequest};
use unemu_core_api::{
    test_support::{MemoryContent, MemoryStorage},
    ContractVersion, Core, FrameArea, Key, KeyState, OfferedContent, WriteTarget, CONTRACT,
};

pub(super) fn observed(config: &MachineConfig, display: DeviceId) -> AssembledMachine {
    assembly::assemble_with_observation_and_options(
        config,
        &ObservationPolicy {
            routes: vec![ObservationRequest {
                device: display,
                output: OutputKind::TextScanout,
                byte_limit: 0,
            }],
            ..ObservationPolicy::default()
        },
        assembly::AssemblyOptions {
            allow_unlisted_firmware: true,
        },
    )
    .unwrap()
}

fn key(key: Key, state: KeyState) -> InputEvent {
    InputEvent::key(KEYBOARD_PORT, key, state)
}

fn keyboard_state(assembled: &AssembledMachine) -> Vec<u8> {
    let index = assembled.device_index[&DeviceId::Keyboard];
    assembled.machine.bus().device(index).unwrap().save()
}

fn memory_image_machine() -> PcMachine {
    let program = vec![
        0xFA, 0x31, 0xC0, 0x8E, 0xD8, // CLI; XOR AX,AX; MOV DS,AX
        0xC7, 0x06, 0x00, 0x05, 0x5C, 0xA1, // MOV WORD [0500h],A15Ch
        0xB8, 0xFF, 0xFF, 0x8E, 0xD8, // MOV AX,FFFFh; MOV DS,AX
        0xC7, 0x06, 0x10, 0x05, 0x6D, 0xB2, // MOV WORD [0510h],B26Dh (100500h)
        0xB8, 0x00, 0xD0, 0x8E, 0xD8, // MOV AX,D000h; MOV DS,AX
        0xC7, 0x06, 0x20, 0x00, 0x7E, 0xC3, // MOV WORD [0020h],C37Eh
        0xB0, 0xD1, 0xE6, 0x64, 0xB0, 0x01, 0xE6, 0x60, // 8042: A20 off
        0xB8, 0xFF, 0xFF, 0x8E, 0xD8, // MOV AX,FFFFh; MOV DS,AX
        0xA1, 0x10, 0x05, // MOV AX,[0510h] (aliases 0500h)
        0x31, 0xDB, 0x8E, 0xDB, 0xA3, 0x04, 0x05, // XOR BX,BX; MOV DS,BX; MOV [0504h],AX
        0xFF, 0x06, 0x02, 0x05, 0xEB, 0xFA, // INC WORD [0502h]; JMP to INC
    ];
    let mut config = assembly::at_boot(program);
    config.memory.address_space_bytes = 0x110000;
    // Deliberately unordered: image regions must be in physical order.
    config.memory.ram = vec![
        assembly::RamRegion::new(0x100000, 0x10000),
        assembly::RamRegion::new(0xD0000, 0x4000),
        assembly::RamRegion::new(0, 0xA0000),
    ];
    config.memory.mmio = vec![assembly::MmioWindow::new(0xA0000, 0x20000, DeviceId::Vga)];
    config.images.push(assembly::LoadImage {
        base: 0xE0000,
        source: assembly::ImageSource::Inline(vec![0xF4; 0x1000]),
        kind: assembly::ImageKind::Rom,
        provides: Vec::new(),
    });
    let vga = config
        .devices
        .iter_mut()
        .find(|d| d.id == DeviceId::Vga)
        .unwrap();
    vga.kind = DeviceKind::VgaRegisters(assembly::VgaCfg {});
    vga.ports = assembly::VgaCfg::port_windows();
    PcMachine::from_assembled(&config, assembly::assemble(&config).unwrap()).unwrap()
}

#[test]
fn memory_image_copies_guest_writes_in_every_ram_region_with_a20_off() {
    let mut machine = memory_image_machine();
    assert_eq!(
        machine.run(RunLimit::until(Ticks(1_000))),
        Ok(Stop::Deadline)
    );
    assert!(!machine.diagnostic_report().cpu.a20);
    let image = machine.memory_image().unwrap();
    assert_eq!(
        image
            .regions
            .iter()
            .map(|r| (r.base, r.kind, r.bytes.len()))
            .collect::<Vec<_>>(),
        [
            (0, assembly::MemoryKind::Ram, 0xA0000),
            (0xD0000, assembly::MemoryKind::Ram, 0x4000),
            (0x100000, assembly::MemoryKind::Ram, 0x10000),
        ]
    );
    assert_eq!(&image.regions[0].bytes[0x500..0x502], &[0x5C, 0xA1]);
    assert_eq!(
        &image.regions[0].bytes[0x504..0x506],
        &[0x5C, 0xA1],
        "CPU reads alias with A20 off"
    );
    assert_eq!(&image.regions[1].bytes[0x20..0x22], &[0x7E, 0xC3]);
    assert_eq!(
        &image.regions[2].bytes[0x500..0x502],
        &[0x6D, 0xB2],
        "the image uses physical addresses"
    );
    assert_eq!(
        image.to_string(),
        concat!(
            "memory image\n",
            "ram      base 0x00000000, length 0xa0000\n",
            "ram      base 0x000d0000, length 0x4000\n",
            "ram      base 0x00100000, length 0x10000\n",
        )
    );
    machine.run(RunLimit::until(Ticks(2_000))).unwrap();
    assert_ne!(
        machine.memory_image().unwrap().regions[0].bytes,
        image.regions[0].bytes,
        "an owned image keeps the bytes at its capture boundary"
    );
}

fn device_states(machine: &PcMachine) -> Vec<Vec<u8>> {
    let bus = machine.assembled_ref().machine.bus();
    (0..)
        .map_while(|index| bus.device(index).map(|device| device.save()))
        .collect()
}

#[test]
fn memory_image_leaves_time_cpu_devices_and_the_subsequent_run_unchanged() {
    let mut plain = memory_image_machine();
    let mut imaged = memory_image_machine();
    for machine in [&mut plain, &mut imaged] {
        machine.enable_code_trail();
        machine.enable_delivery_log().unwrap();
        assert_eq!(
            machine.run(RunLimit::until(Ticks(1_000))),
            Ok(Stop::Deadline)
        );
        assert!(machine.assembled.type_scancodes(&[0x1E, 0x9E]));
        let bus = machine.assembled.machine.bus_mut();
        for (port, value) in [
            (0x3C4, 0x0F02),
            (0x3C4, 0x0604),
            (0x3CE, 0x0506),
            (0x3CE, 0x0005),
            (0x3CE, 0xFF08),
        ] {
            bus.port_write(port, 2, value).unwrap();
        }
        bus.mmio_write(0xA0000, 1, 0xA5).unwrap();
        assert_eq!(bus.mmio_read(0xA0000, 1).unwrap(), 0xA5);
        bus.mmio_write(0xA0001, 1, 0x5A).unwrap();
    }
    let before = (
        imaged.now(),
        imaged.steps(),
        imaged.status(),
        imaged.assembled.machine.virtual_time(),
        imaged.assembled.machine.cpu().diagnostic_state(),
        device_states(&imaged),
    );
    let image = imaged.memory_image().unwrap();
    assert_eq!(image.regions.len(), 3, "device windows are omitted");
    assert_eq!(
        before,
        (
            imaged.now(),
            imaged.steps(),
            imaged.status(),
            imaged.assembled.machine.virtual_time(),
            imaged.assembled.machine.cpu().diagnostic_state(),
            device_states(&imaged)
        )
    );
    eprintln!(
        "image capture unchanged: retired {}, steps {}, clock {} ticks; CPU and every device identical",
        before.4.retired, before.1, before.0.0
    );
    for until in [2_000, 4_000, 8_000] {
        assert_eq!(
            plain.run(RunLimit::until(Ticks(until))),
            imaged.run(RunLimit::until(Ticks(until)))
        );
        assert_eq!(plain.diagnostic_report(), imaged.diagnostic_report());
        assert_eq!(device_states(&plain), device_states(&imaged));
        assert_eq!(
            plain.memory_image().unwrap(),
            imaged.memory_image().unwrap()
        );
    }
    eprintln!(
        "subsequent twins identical: retired {}, steps {}, clock {} ticks; reports, devices and RAM equal",
        imaged.assembled.machine.cpu().diagnostic_state().retired,
        imaged.steps(),
        imaged.now().0
    );
}

/// The same keys and the same chunks, once through `dyn Machine` and once
/// through the assembled machine with the encoder and the cap the desktop
/// drove it with before the contract existed, end in the same machine.
#[test]
fn a_session_through_the_contract_matches_the_assembled_machine() {
    let config = assembly::at_keyboard(vec![0xEB, 0xFE]);
    let mut machine =
        PcMachine::from_assembled(&config, assembly::assemble(&config).unwrap()).unwrap();
    let mut direct = assembly::assemble(&config).unwrap();
    direct.machine.set_idle_limit(Some(u64::MAX));
    let mut keyboard = HostKeyboard::default();
    let rate = CpuRate {
        numerator: NonZeroU64::new(config.clock.retired_per_pit_clock).unwrap(),
        denominator: NonZeroU64::new(config.clock.retired_per_pit_clock_denominator).unwrap(),
    };
    let script = [
        (Key::SHIFT_LEFT, KeyState::Pressed),
        (Key::A, KeyState::Pressed),
        (Key::A, KeyState::Repeated),
        (Key::A, KeyState::Released),
        (Key::SHIFT_LEFT, KeyState::Released),
        (Key::ARROW_UP, KeyState::Pressed),
        (Key::ARROW_UP, KeyState::Released),
    ];
    let pristine = keyboard_state(&direct);
    for (round, (pressed, state)) in script.into_iter().enumerate() {
        let contract: &mut dyn Machine = &mut machine;
        contract.input(key(pressed, state)).unwrap();
        keyboard.key_event(pressed, state);
        let until = (round as u64 + 1) * 5_000;
        assert_eq!(
            contract.run(RunLimit::until(Ticks(until))),
            Ok(Stop::Deadline)
        );
        keyboard.inject_at_boundary(&mut direct);
        let clock = direct.machine.clock().0;
        let time = direct.machine.virtual_time();
        let cap = run_cap(until, clock, time, rate);
        assert_eq!(
            direct.machine.run_bounded(Some(cap)).unwrap(),
            RunOutcome::RetiredCapReached
        );
        assert_eq!(contract.now(), Ticks(direct.machine.clock().0));
        assert_eq!(contract.steps(), direct.machine.virtual_time());
        assert_eq!(contract.status(), Status::Ready);
        assert_eq!(
            keyboard_state(machine.assembled_ref()),
            keyboard_state(&direct)
        );
    }
    assert_ne!(
        keyboard_state(&direct),
        pristine,
        "the keys reached the 8042"
    );
}

#[test]
fn the_picture_is_the_rasterised_text_of_the_completed_boundary() {
    let config = assembly::at_vga_text(vec![0xC7, 0x06, 0, 0, 0xDB, 0x1F, 0xF4]);
    let mut machine = PcMachine::from_assembled(&config, observed(&config, DeviceId::Vga)).unwrap();
    assert_eq!(machine.description().displays.len(), 1);
    assert_eq!(machine.description().displays[0].label, "VGA");
    assert_eq!(machine.run(RunLimit::until(Ticks(1_000))), Ok(Stop::Halted));
    assert_eq!(machine.status(), Status::Halted);
    assert_eq!(machine.run(RunLimit::until(Ticks(2_000))), Ok(Stop::Halted));
    let frame = machine.frame(DisplayId(0)).unwrap();
    assert_eq!((frame.width, frame.height), (1280, 800));
    assert_eq!(frame.aspect, (4, 3));
    // The PC states no overscan: the whole picture is the visible part.
    assert_eq!(frame.visible, FrameArea::new(0, 0, 1280, 800));
    let first = frame.serial;
    for y in 0..800 {
        for x in 0..1280 {
            assert_eq!(
                frame.pixels[y * 1280 + x],
                if y < 32 && x < 16 { 0xFFFFFF } else { 0 }
            );
        }
    }
    assert_eq!(
        machine.frame(DisplayId(0)).unwrap().serial,
        first,
        "an unchanged screen keeps its serial"
    );
    assert_eq!(
        machine
            .run(RunLimit::until(Ticks(3_000)).with_stop_on_frame(true))
            .unwrap(),
        Stop::Halted
    );
    assert!(!machine.description().features.frame_stops);
    assert_eq!(
        machine.frame(DisplayId(1)).unwrap_err().kind,
        ErrorKind::Usage
    );
}

#[test]
fn the_reset_button_restarts_a_halted_pc() {
    let mut rom = vec![0; 0x8000];
    rom[..5].copy_from_slice(&[0xB0, 0xFE, 0xE6, 0x64, 0xF4]);
    rom[0x7FF0..0x7FF4].copy_from_slice(&[0xB0, 0x55, 0xFA, 0xF4]);
    let mut config = assembly::bios(rom, None);
    config.cpu.reset = assembly::ResetSpec::Entry { cs: 0xF800, ip: 0 };
    let mut machine = PcMachine::from_assembled(&config, observed(&config, DeviceId::Mda)).unwrap();
    assert_eq!(
        machine.reset(ResetKind::Hard).unwrap_err().kind,
        ErrorKind::Unsupported
    );
    // The guest pulses the 8042 reset line, then halts with interrupts off
    // at the reset vector: the reset happened inside the run.
    assert_eq!(
        machine.run(RunLimit::until(Ticks(10_000))),
        Ok(Stop::Halted)
    );
    assert_eq!(machine.reset_requests(), 1);
    assert_eq!(machine.cpu_resets(), 1);
    let mut messages = Vec::new();
    machine.drain_messages(&mut messages);
    assert_eq!(messages, [Message::new(MessageLevel::Note, "cpu reset")]);
    machine.reset(ResetKind::Soft).unwrap();
    assert_eq!(machine.status(), Status::Ready);
    assert_eq!(machine.cpu_resets(), 2);
    assert_eq!(
        machine.reset_summary().as_deref(),
        Some("cpu resets in session: 2")
    );
    machine.drain_messages(&mut messages);
    assert_eq!(messages.len(), 1, "only the first reset is reported");
    assert_eq!(machine.finish(), Ok(Finished::new()));
}

#[test]
fn the_description_lists_the_configured_drives_ports_and_clock() {
    let mut config = assembly::bios(vec![0; 0x8000], Some(vec![0; 1_474_560]));
    config.drives[0].write = assembly::WriteMode::ReadOnly;
    let machine = PcMachine::from_assembled(&config, observed(&config, DeviceId::Mda)).unwrap();
    let description = machine.description();
    assert_eq!(description.clock, Rate::new(105_000_000, 88));
    assert_eq!(description.step_unit, "instruction");
    assert_eq!(
        description.audio,
        Some(AudioDescription::new(Rate::new(48_000, 1)))
    );
    assert_eq!(description.features, PC_FEATURES);
    let slots: Vec<_> = description
        .slots
        .iter()
        .map(|slot| (slot.id, slot.kind, slot.label.as_str(), slot.removable))
        .collect();
    assert_eq!(slots, [(SlotId(0), SlotKind::Floppy, "A:", true)]);
    assert!(description
        .inputs
        .iter()
        .any(|port| port.id == KEYBOARD_PORT && port.kind == InputKind::Keyboard));
    let mut machine = machine;
    let slots = machine.slot_states();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].inserted, Some(0));
    let writes: Vec<_> = slots[0]
        .catalogue
        .iter()
        .map(|medium| medium.writes)
        .collect();
    assert_eq!(writes, [WriteKind::ReadOnly]);
    let mut messages = Vec::new();
    machine.drain_messages(&mut messages);
    assert_eq!(messages.len(), 1);
    assert!(messages[0]
        .text
        .starts_with("warning: running unlisted firmware"));
}

#[test]
fn a_drive_without_a_controller_refuses_changes_as_a_usage_error() {
    let config = assembly::at_keyboard(vec![0xEB, 0xFE]);
    let mut machine =
        PcMachine::from_assembled(&config, assembly::assemble(&config).unwrap()).unwrap();
    assert_eq!(
        machine
            .change_medium(SlotId(0), MediaAction::Next)
            .unwrap_err()
            .kind,
        ErrorKind::Usage
    );
    assert_eq!(
        machine
            .change_medium(SlotId(2), MediaAction::Eject)
            .unwrap_err()
            .kind,
        ErrorKind::Usage
    );
    assert!(machine.slot_states().is_empty());
}

#[test]
fn the_pc_core_describes_its_launch_type_and_refuses_other_types() {
    let core: &dyn Core = &crate::PcCore;
    let description = core.description();
    assert_eq!(description.implements, ContractVersion::new(0, 6));
    assert!(CONTRACT.drives(description.implements));
    assert_eq!(description.id, "unemupc");
    assert_eq!(description.platforms.len(), 1);
    assert_eq!(description.platforms[0].id, "ibm-pc");
    assert_eq!(
        description.platforms[0].config_types,
        [pc_config::launch::MEDIA_TYPE]
    );
    // A PC starts from a config, never from a file a person opens.
    assert!(description.platforms[0].content.is_empty());
    assert!(description
        .notices
        .iter()
        .any(|notice| notice.text == crate::FONT_NOTICE));
    struct Nothing;
    impl unemu_core_api::ContentSource for Nothing {
        fn open(&self, _sha256: &[u8; 32], _size: u64) -> Result<unemu_core_api::Content, Error> {
            Err(Error::new(ErrorKind::Content, "nothing"))
        }
    }
    let storage = std::rc::Rc::new(MemoryStorage::new());
    let request = unemu_core_api::CreateRequest::new(
        "ibm-pc",
        unemu_core_api::ConfigDocument::new("application/toml", b""),
        &Nothing,
        storage.clone(),
    );
    assert_eq!(
        core.create(&request).err().map(|error| error.kind),
        Some(ErrorKind::Unsupported)
    );
    assert!(
        storage.names().is_empty(),
        "a refused create writes nothing"
    );
    let bytes = MemoryContent::new(vec![0; 512]);
    let offered = OfferedContent::new("disk.img", [0; 32], 512, &bytes);
    assert!(matches!(core.identify(&offered), Ok(None)));
    assert_eq!(
        core.validate(
            "ibm-pc",
            unemu_core_api::ConfigDocument::new("application/toml", b"")
        )
        .err()
        .map(|error| error.kind),
        Some(ErrorKind::Unsupported)
    );
}

/// Media join a running PC through its provider, so the contract's
/// `add_medium` is refused whatever the medium, including one whose writes
/// are kept in a record of the machine's storage.
#[test]
fn the_pc_refuses_media_through_the_contract() {
    let config = assembly::at_keyboard(vec![0xF4]);
    let mut machine =
        PcMachine::from_assembled(&config, assembly::assemble(&config).unwrap()).unwrap();
    for writes in [
        WriteTarget::ReadOnly,
        WriteTarget::Discarded,
        WriteTarget::kept("kept-floppy-0"),
    ] {
        let medium = NewMedium::new(
            "blank.img",
            unemu_core_api::Content::Bytes(vec![0; 512]),
            [0; 32],
            512,
            "raw",
            writes,
        );
        assert_eq!(
            machine
                .add_medium(SlotId(0), medium)
                .err()
                .map(|error| error.kind),
            Some(ErrorKind::Unsupported),
            "{writes:?}"
        );
    }
}

/// A CPU refusal fails the machine with the core's one-line error, and the
/// machine keeps the stop report a front end shows beside it.
#[test]
fn a_cpu_refusal_keeps_its_stop_report() {
    let config = assembly::test_support::refused_gate_call();
    let mut machine =
        PcMachine::from_assembled(&config, assembly::assemble(&config).unwrap()).unwrap();
    assert!(machine.cpu_stop_report().is_none());
    let failure = machine.run(RunLimit::until(Ticks(100_000))).unwrap_err();
    assert_eq!(failure.kind, ErrorKind::Guest);
    assert_eq!(
        failure.message,
        "cpu: unsupported cpu system operation LegacyInstruction at 002b:7c2b"
    );
    assert_eq!(machine.status(), Status::Failed);
    let report = machine
        .cpu_stop_report()
        .expect("a refusal keeps its report");
    assert_eq!(
        (report.cs, report.ip, report.code_linear),
        (0x2B, 0x7C2B, 0x7C2B)
    );
    let system = report.system.unwrap();
    assert_eq!((system.msw & 1, system.cpl), (1, 3));
    assert_eq!((system.gdtr.base, system.gdtr.limit), (0x7D00, 0x3F));
    assert_eq!(system.tr.selector, 0x18);
    let named: Vec<_> = report
        .descriptors
        .iter()
        .map(|named| (named.role, named.selector, named.entry.map(|d| d.0)))
        .collect();
    assert_eq!(
        named,
        [
            ("target", 0x23, Ok([0x23, 0x7C, 0x08, 0, 0x1F, 0xE4, 0, 0])),
            // The far jump into protected mode set the accessed bit.
            ("gate target", 0x08, Ok([0xFF, 0xFF, 0, 0, 0, 0x9B, 0, 0])),
        ]
    );
    assert_eq!(
        report.tss.unwrap().stacks,
        [(0x38, 0x0F00), (0x30, 0x5000), (0x30, 0x4000)]
    );
    let text = report.to_string();
    for line in [
        "stop     unsupported cpu system operation LegacyInstruction at 002b:7c2b\n",
        "form     far call ptr16:16 0023:0000\n",
        "desc     target 0023 (gdt) 23 7c 08 00 1f e4 00 00: 286 call gate, selector 0008, offset 7c23, 31 words, dpl 3, present\n",
        "tss      link 0000, ss0:sp0 0038:0f00, ss1:sp1 0030:5000, ss2:sp2 0030:4000\n",
    ] {
        assert!(text.contains(line), "{line:?} missing from\n{text}");
    }
    // A failed machine does not run again, so the report stays.
    assert!(machine.run(RunLimit::until(Ticks(200_000))).is_err());
    assert!(machine.cpu_stop_report().is_some());
}

/// A guest whose exception handler faults at its first instruction retires
/// nothing, so a run bounded by virtual time would never end. The run returns
/// a guest error instead, the machine fails, and the stop report shows the
/// handler. The run happens on its own thread, so a run that never returns
/// fails the test at the deadline instead of hanging it.
#[test]
fn a_stall_in_exception_delivery_fails_the_machine_with_a_report() {
    // The ROM is the NASM output of the listing on `delivery_stall_rom`.
    assert_eq!(
        assembly::test_support::fixture_sha256(&assembly::test_support::delivery_stall_rom()),
        "3a47a441b48441cb016e85183671c7364f4f4fed7a9df4eeac96404506863235"
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let config = assembly::test_support::delivery_stall();
        let mut machine = PcMachine::from_assembled(&config, observed(&config, DeviceId::Mda))
            .expect("the machine starts");
        let failure = machine.run(RunLimit::until(Ticks(1_000_000))).unwrap_err();
        let report = machine.cpu_stop_report().map(|report| report.to_string());
        let again = machine.run(RunLimit::until(Ticks(2_000_000))).unwrap_err();
        sender
            .send((failure, machine.status(), report, again))
            .unwrap();
    });
    let (failure, status, report, again) = receiver
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the run returns");
    assert_eq!(failure.kind, ErrorKind::Guest);
    assert_eq!(
        failure.message,
        "cpu: the processor is stuck delivering exceptions: 11264 deliveries without an \
         instruction, the last to 0008:0040"
    );
    assert_eq!(status, Status::Failed);
    assert_eq!(
        again.kind,
        ErrorKind::Usage,
        "a failed machine does not run"
    );
    let report = report.expect("a stall keeps its report");
    for line in [
        "stop     the processor is stuck delivering exceptions: 11264 deliveries without an instruction, the last to 0008:0040\n",
        "cs:ip    0008:0040 (linear 0f8040)\n",
        "mode     protected, cpl 0, msw fff1, a20 on\n",
        "ss:sp    0010:9000\n",
        "idtr     base 0f8180 limit 006f\n",
        "code     8e d8 f4 00 00 00 00 00 00 00 00 00 00 00 00 00\n",
    ] {
        assert!(report.contains(line), "{line:?} missing from\n{report}");
    }
}

/// A guest whose #UD handler is itself an invalid opcode, on a stack that
/// overruns (6-byte frames from SP FFFFh), shuts down after 10,922
/// deliveries, as an 80286 does, and the board resets the processor: the ROM
/// starts again from the reset vector, counts its second start in RAM and
/// halts. No delivery stall is reported, so a guest that leaves protected
/// mode by a triple fault reaches its reset however it gets there.
#[test]
fn a_stall_that_overruns_its_stack_shuts_down_and_the_board_resets() {
    // The ROM is the NASM output of the listing on `delivery_overrun_rom`.
    let rom = assembly::test_support::delivery_overrun_rom();
    assert_eq!(
        assembly::test_support::fixture_sha256(&rom),
        "10c34266f57445874cb2c8724d536944dcb7817c9b814ec563533c47fe8f290c"
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let config = assembly::bios(rom, None);
        let mut machine = PcMachine::from_assembled(&config, observed(&config, DeviceId::Mda))
            .expect("the machine starts");
        let stop = machine.run(RunLimit::until(Ticks(1_000_000)));
        let mut starts = [0];
        machine
            .assembled
            .machine
            .cpu()
            .read_mem(0x500, &mut starts)
            .unwrap();
        sender
            .send((
                stop,
                machine.reset_requests(),
                machine.cpu_resets(),
                starts[0],
            ))
            .unwrap();
    });
    let (stop, requests, resets, starts) = receiver
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the run returns");
    assert_eq!(stop, Ok(Stop::Halted));
    assert_eq!((requests, resets, starts), (1, 1, 2));
}

/// A guest that shuts down again right after every reset takes three steps
/// of virtual time per reset, so a run capped by virtual time would do
/// nothing but reset, for as long as its deadline allowed. The machine stops that
/// storm of resets inside the run, which returns with the guest error and a
/// stop report at the last shutdown. The run happens on its own thread, so
/// one that never returns fails the test at the deadline instead of hanging
/// it.
#[test]
fn a_guest_that_resets_again_and_again_stops_inside_its_run() {
    let rom = assembly::test_support::reset_storm_rom();
    assert_eq!(
        assembly::test_support::fixture_sha256(&rom),
        assembly::test_support::RESET_STORM_ROM_SHA256
    );
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let config = assembly::test_support::reset_storm();
        let mut machine = PcMachine::from_assembled(&config, observed(&config, DeviceId::Mda))
            .expect("the machine starts");
        // One second of guest time: about 245,000 resets without the stop.
        let failure = machine.run(RunLimit::until(Ticks(1_193_182))).unwrap_err();
        let report = machine.cpu_stop_report().map(|report| report.to_string());
        sender
            .send((
                failure,
                machine.status(),
                report,
                machine.reset_requests(),
                machine.cpu_resets(),
            ))
            .unwrap();
    });
    let (failure, status, report, requests, resets) = receiver
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the run returns");
    // 63 resets; the 64th request stops the machine instead of resetting it.
    assert_eq!((requests, resets), (63, 63));
    assert_eq!(failure.kind, ErrorKind::Guest);
    assert_eq!(
        failure.message,
        "cpu: the machine resets again and again: 64 resets within 192 steps, \
         the last shutdown at f800:0007"
    );
    assert_eq!(status, Status::Failed);
    let report = report.expect("a storm keeps its report");
    for line in [
        "stop     the machine resets again and again: 64 resets within 192 steps, the last shutdown at f800:0007\n",
        "cs:ip    f800:0007 (linear 0f8007)\n",
        "code     cc f4 00 00 00 00 00 00 00 00 00 00 00 00 00 00\n",
    ] {
        assert!(report.contains(line), "{line:?} missing from\n{report}");
    }
}

/// A guest that silences the keyboard and halts with interrupts disabled
/// runs no further, and nothing reports why. The diagnostic report shows
/// it: IF clear, the CPU halted after its HLT, IRQ1 masked in the master
/// 8259, the 8042's keyboard interface disabled, and the code trail ending
/// at the HLT.
#[test]
fn a_diagnostic_report_shows_a_guest_that_masked_the_keyboard_and_halted() {
    // The ROM is the NASM output of the listing on `keyboard_masked_halt_rom`.
    assert_eq!(
        assembly::test_support::fixture_sha256(&assembly::test_support::keyboard_masked_halt_rom()),
        "367a0b615d9bc9f752b49a2890a561ffe95711ed4b36c62dde0bec7b08342b21"
    );
    let config = assembly::test_support::keyboard_masked_halt();
    let mut machine = PcMachine::from_assembled(&config, observed(&config, DeviceId::Mda))
        .expect("the machine starts");
    machine.enable_code_trail();
    assert_eq!(
        machine.run(RunLimit::until(Ticks(100_000))),
        Ok(Stop::Halted)
    );
    let retired = cpu::Cpu::retired(machine.assembled_ref().machine.cpu());
    let report = machine.diagnostic_report();
    assert!(report.halted);
    assert!(!report.cpu.interrupts_enabled());
    assert_eq!((report.cpu.cs().selector, report.cpu.ip), (0xF800, 0x001A));
    assert_eq!(report.cpu.hlt_before(), Some((0xF800, 0x0019)));
    assert_eq!(report.cpu.code_linear, 0x0F801A);
    let master = report.master_pic.expect("a master 8259");
    assert_eq!(master.imr & 0x02, 0x02, "IRQ1 masked");
    assert_eq!((master.vector_base, master.awaiting), (0x08, None));
    let keyboard = report.keyboard.expect("an 8042");
    assert!(keyboard.keyboard_disabled());
    assert_eq!(keyboard.last_command, Some(0xAD));
    let trail = report.trail.clone().expect("the trail was enabled");
    let last = trail.last().expect("the trail has entries");
    assert_eq!((last.cs, last.ip), (0xF800, 0x001A));
    // Each OUT ended a run slice: the trail passes through the instruction
    // after every one of them, in order, before the HLT.
    let addresses: Vec<u16> = trail
        .iter()
        .filter(|entry| entry.cs == 0xF800)
        .map(|entry| entry.ip)
        .collect();
    assert_eq!(addresses, [0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1A]);
    assert_eq!(report.cpu.retired, retired);
    assert_eq!(
        cpu::Cpu::retired(machine.assembled_ref().machine.cpu()),
        retired,
        "the report changes nothing"
    );
    let text = report.to_string();
    for line in [
        "machine diagnostic report\n",
        "status   halted (waits for a reset)\n",
        "halted   yes, with interrupts disabled\n",
        "cs:ip    f800:001a (linear 0f801a)\n",
        "hlt      at f800:0019, the byte before cs:ip\n",
        "mode     real, cpl 0, msw fff0 (pe 0), a20 on\n",
        "code     00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00\n",
        "trail    f800:001a\n",
        "pic      master irr 00 isr 00 imr 02 base 08 icw1 11 icw4 01 (auto-eoi off), initialized, reads irr",
        "kbc      pending none, last command ad,",
        "a20      on: 8042 output port on, port 92h",
        "cmos     shutdown status (0fh) 00, nmi unmasked, index 00\n",
    ] {
        assert!(text.contains(line), "{line:?} missing from\n{text}");
    }
    assert!(text.contains(": if 0, tf 0, df 0, iopl 0, nt 0;"), "{text}");
    assert!(
        text.contains("command byte 10: keyboard disabled,"),
        "{text}"
    );
    eprintln!("{text}");
}

/// A guest that halts leaves no report.
#[test]
fn a_halt_leaves_no_stop_report() {
    let config = assembly::at_boot(vec![0xFA, 0xF4]);
    let mut machine =
        PcMachine::from_assembled(&config, assembly::assemble(&config).unwrap()).unwrap();
    assert_eq!(
        machine.run(RunLimit::until(Ticks(100_000))),
        Ok(Stop::Halted)
    );
    assert!(machine.cpu_stop_report().is_none());
}

/// The contract's rule on `drain_audio`: sound keeps its place in virtual
/// time, and at every return of `run`, once `drain_audio` has been called,
/// the next frame it will hand over has index `floor(now() × rate / clock)`,
/// within one frame. The PC hands over exactly the frames completed by
/// `now()`, so the next index is that floor exactly. The PC mixes
/// at 48 kHz off virtual time, so the deadlines are placed a tick before, on
/// and a tick after the edges of chosen frames, out to frame 9 600 (a fifth
/// of a virtual second).
#[test]
fn the_sound_handed_over_is_the_floor_of_virtual_time_times_the_rate() {
    let config = assembly::at_keyboard(vec![0xEB, 0xFE]);
    let mut machine =
        PcMachine::from_assembled(&config, assembly::assemble(&config).unwrap()).unwrap();
    machine
        .assembled()
        .add_audio_source(Box::new(audio::EdgeSource::new([1234, -2345])));
    machine.set_outputs(Outputs::NONE.with_audio(true));
    let description = machine.description().clone();
    let rate = description.audio.unwrap().rate;
    let clock = description.clock;
    // Frame k ends at tick k·clock/rate; one frame is `one_frame` in the
    // scaled units below.
    let ticks_per_frame = (
        u128::from(clock.numerator) * u128::from(rate.denominator),
        u128::from(clock.denominator) * u128::from(rate.numerator),
    );
    let mut deadlines = std::collections::BTreeSet::new();
    for k in [1u128, 2, 3, 7, 22, 100, 1_001, 4_800, 4_801, 9_599, 9_600] {
        let floor = k * ticks_per_frame.0 / ticks_per_frame.1;
        let ceil = (k * ticks_per_frame.0).div_ceil(ticks_per_frame.1);
        for tick in [floor - 1, floor, ceil, ceil + 1] {
            deadlines.insert(tick as u64);
        }
    }
    let mut handed = 0u128;
    let mut out = Vec::new();
    for until in deadlines {
        assert_eq!(
            machine.run(RunLimit::until(Ticks(until))),
            Ok(Stop::Deadline)
        );
        out.clear();
        machine.drain_audio(&mut out).unwrap();
        handed += out.len() as u128;
        let now = u128::from(machine.now().0);
        // handed ≤ now·rate/clock < handed + 1, cleared of fractions.
        let scaled_handed = handed * ticks_per_frame.0;
        let scaled_due = now * ticks_per_frame.1;
        assert!(
            scaled_handed <= scaled_due && scaled_due - scaled_handed < ticks_per_frame.0,
            "at tick {now}: {handed} frames handed over, {} due",
            scaled_due as f64 / ticks_per_frame.0 as f64
        );
    }
    assert!(handed >= 9_600, "{handed} frames");
}

mod launch_contract {
    use super::*;
    use pc_config::{image::Digest, launch};
    use std::{
        cell::RefCell,
        collections::{BTreeMap, BTreeSet},
        rc::Rc,
    };
    use unemu_core_api::{ConfigDocument, Content, ContentSource, CreateRequest, Storage};

    struct Shared(Rc<MemoryContent>);
    impl unemu_core_api::ContentReader for Shared {
        fn size(&self) -> u64 {
            self.0.size()
        }
        fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<(), Error> {
            self.0.read_at(offset, bytes)
        }
    }
    struct Source {
        blobs: BTreeMap<[u8; 32], Vec<u8>>,
        reader: Option<([u8; 32], Rc<MemoryContent>)>,
        opened: RefCell<BTreeSet<([u8; 32], u64)>>,
    }
    impl ContentSource for Source {
        fn open(&self, sha256: &[u8; 32], size: u64) -> Result<Content, Error> {
            self.opened.borrow_mut().insert((*sha256, size));
            if let Some((digest, reader)) = &self.reader {
                if digest == sha256 {
                    return Ok(Content::Reader(Box::new(Shared(reader.clone()))));
                }
            }
            self.blobs
                .get(sha256)
                .filter(|bytes| bytes.len() as u64 == size)
                .map(|bytes| Content::Reader(Box::new(MemoryContent::new(bytes.clone()))))
                .ok_or_else(|| Error::new(ErrorKind::Content, "missing test content"))
        }
    }

    fn fixture() -> (Vec<u8>, Source) {
        let mut bios = vec![0xf4; 32768];
        let program = [
            0xfa, 0xb0, 0x10, 0xe6, 0x70, 0xe4, 0x71, 0xfe, 0xc0, 0xe6, 0x71, 0xb0, 0xb6, 0xe6,
            0x43, 0xb0, 0xa9, 0xe6, 0x42, 0xb0, 0x04, 0xe6, 0x42, 0xb0, 0x03, 0xe6, 0x61, 0xba,
            0xf2, 0x03, 0xb0, 0x1c, 0xee, 0xb8, 0x00, 0xb0, 0x8e, 0xd8, 0xc7, 0x06, 0x00, 0x00,
            0x41, 0x07, 0xeb, 0xfe,
        ];
        bios[..program.len()].copy_from_slice(&program);
        bios[0x7ff0..0x7ff5].copy_from_slice(&[0xea, 0, 0, 0, 0xf8]);
        let cmos = vec![0; 64];
        let disk = vec![0xf6; 368640];
        let [db, dc, dd] = [&bios, &cmos, &disk].map(|bytes| Digest::of(bytes));
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
provides = ["floppy"]
sha256 = "{bios_hash}"
[[machine.card]]
kind = "mda"
[[machine.card]]
kind = "vga-text"
[[machine.card]]
kind = "fdc-765"
[[machine.card.drives]]
unit = 0
type = "360K"
[[firmware]]
key = "firmware.bios"
source = "assets/bios.bin"
blob = "{db}"
[[media]]
id = "a"
source = "assets/a.img"
name = "Disk A"
head = "{dd}"
policy = "journal"
geometry = {{ cylinders = 40, heads = 2, sectors = 9, sector_bytes = 512 }}
[[slots]]
controller = "fdc-765"
unit = 0
catalogue = ["a"]
catalogue_mode = true
inserted = "a"
[nvram]
mode = "journal"
source = "assets/cmos.bin"
head = "{dc}"
[[content]]
digest = "{db}"
sha256 = "{bios_hash}"
size = 32768
[[content]]
digest = "{dd}"
sha256 = "{disk_hash}"
size = 368640
[[content]]
digest = "{dc}"
sha256 = "{cmos_hash}"
size = 64
[[record]]
medium = "a"
name = "a.upfj"
keep_generations = 3
[[record]]
medium = "cmos"
name = "cmos.upfj"
keep_generations = 3
"#,
            bios_hash = db.hex(),
            disk_hash = dd.hex(),
            cmos_hash = dc.hex()
        );
        (
            text.into_bytes(),
            Source {
                blobs: [bios, cmos, disk]
                    .into_iter()
                    .map(|bytes| (Digest::of(&bytes).sha256(), bytes))
                    .collect(),
                reader: None,
                opened: RefCell::default(),
            },
        )
    }
    fn request<'a>(
        bytes: &'a [u8],
        source: &'a Source,
        storage: Rc<dyn Storage>,
    ) -> CreateRequest<'a> {
        CreateRequest::new(
            "ibm-pc",
            ConfigDocument::new(launch::MEDIA_TYPE, bytes),
            source,
            storage,
        )
        .with_options(&[
            ("allow-unlisted-firmware", "true"),
            ("speaker-trace", "true"),
        ])
    }

    #[test]
    fn launch_validate_lists_exactly_the_content_create_opens() {
        let (bytes, source) = fixture();
        let core: &dyn Core = &crate::PcCore;
        let validated = core
            .validate("ibm-pc", ConfigDocument::new(launch::MEDIA_TYPE, &bytes))
            .unwrap();
        assert!(source.opened.borrow().is_empty());
        let mut machine = core
            .create(&request(&bytes, &source, Rc::new(MemoryStorage::new())))
            .unwrap();
        assert_eq!(
            *source.opened.borrow(),
            validated
                .content
                .iter()
                .map(|row| (row.sha256, row.size))
                .collect()
        );
        let d = machine.description();
        assert_eq!(
            d.displays
                .iter()
                .map(|d| d.label.as_str())
                .collect::<Vec<_>>(),
            ["MDA", "VGA"]
        );
        assert!(d.audio.as_ref().unwrap().rate_in_bounds());
        assert_eq!(machine.steps(), 0);
        assert_eq!(machine.now(), Ticks(0));
        let resets = d.features.resets.to_vec();
        machine.set_outputs(Outputs::NONE.with_audio(true));
        machine.input(InputEvent::ReleaseAll).unwrap();
        for reset in resets {
            machine.reset(reset).unwrap();
        }
        assert_eq!(machine.run(RunLimit::steps(2000)).unwrap(), Stop::Steps);
        for id in [DisplayId(0), DisplayId(1)] {
            let frame = machine.frame(id).unwrap();
            assert!(frame.visible.is_within(frame.width, frame.height));
            assert_eq!(
                frame.pixels.len(),
                frame.width as usize * frame.height as usize
            );
        }
        let mut audio = Vec::new();
        machine.drain_audio(&mut audio).unwrap();
        assert!(!audio.is_empty());
        machine
            .change_medium(SlotId(0), MediaAction::Eject)
            .unwrap();
        machine
            .change_medium(SlotId(0), MediaAction::Insert(0))
            .unwrap();
        let mut messages = Vec::new();
        machine.drain_messages(&mut messages);
        assert!(messages
            .iter()
            .any(|message| message.text.contains("speaker trace on")));
        machine.finish().unwrap();
    }

    #[test]
    fn launch_options_refuse_unknown_invalid_and_repeated_keys_before_content() {
        let (bytes, source) = fixture();
        for options in [
            vec![("unknown", "true")],
            vec![("speaker-trace", "yes")],
            vec![("turbo", "yes")],
            vec![("turbo", "true"), ("turbo", "false")],
            vec![("speaker-trace", "true"), ("speaker-trace", "false")],
        ] {
            let storage = Rc::new(MemoryStorage::new());
            let error = crate::PcCore
                .create(&request(&bytes, &source, storage.clone()).with_options(&options))
                .err()
                .unwrap();
            assert_eq!(error.kind, ErrorKind::Usage, "{error}");
            assert!(storage.names().is_empty());
            assert!(source.opened.borrow().is_empty());
        }
    }

    #[test]
    fn launch_failed_creates_leave_no_session_and_can_be_retried() {
        let (bytes, mut source) = fixture();
        let storage = Rc::new(MemoryStorage::new());
        let disk = source
            .blobs
            .remove(&Digest::of(&vec![0xf6; 368640]).sha256())
            .unwrap();
        assert_eq!(
            crate::PcCore
                .create(&request(&bytes, &source, storage.clone()))
                .err()
                .unwrap()
                .kind,
            ErrorKind::Content
        );
        assert!(storage.names().is_empty());
        source.blobs.insert(Digest::of(&disk).sha256(), disk);
        let held = storage.open("a.upfj").unwrap();
        assert_eq!(
            crate::PcCore
                .create(&request(&bytes, &source, storage.clone()))
                .err()
                .unwrap()
                .kind,
            ErrorKind::Usage
        );
        assert!(storage
            .names()
            .iter()
            .all(|name| storage.contents(name).unwrap().is_empty()));
        drop(held);
        let bad = request(b"not toml", &source, storage.clone());
        let error = crate::PcCore.create(&bad).err().unwrap();
        assert_eq!(error.kind, ErrorKind::Config);
        assert_eq!(
            crate::PcCore
                .validate(bad.platform, bad.config)
                .unwrap_err(),
            error
        );
        let mut machine = crate::PcCore
            .create(&request(&bytes, &source, storage.clone()))
            .unwrap();
        machine.run(RunLimit::steps(200)).unwrap();
        machine.finish().unwrap();
        drop(machine);
        assert!(!storage.is_open("a.upfj"));
        assert!(!storage.is_open("cmos.upfj"));
        let mut machine = crate::PcCore
            .create(&request(&bytes, &source, storage))
            .unwrap();
        machine.finish().unwrap();
    }

    #[test]
    fn launch_replay_and_second_create_keep_the_journalled_cmos() {
        let (bytes, source) = fixture();
        let run = |storage: Rc<MemoryStorage>| {
            let mut machine = crate::PcCore
                .create(&request(&bytes, &source, storage))
                .unwrap();
            machine.set_outputs(Outputs::NONE.with_audio(true));
            let mut log = Vec::new();
            for steps in [100, 200, 4000] {
                machine.input(InputEvent::ReleaseAll).unwrap();
                let stop = machine.run(RunLimit::steps(steps)).unwrap();
                let frame = machine.frame(DisplayId(0)).unwrap();
                let picture = (frame.pixels.to_vec(), frame.serial);
                let mut sound = Vec::new();
                machine.drain_audio(&mut sound).unwrap();
                let mut messages = Vec::new();
                machine.drain_messages(&mut messages);
                log.push((
                    stop,
                    machine.now(),
                    machine.steps(),
                    picture,
                    sound,
                    messages,
                ));
            }
            machine.finish().unwrap();
            log
        };
        let first = Rc::new(MemoryStorage::new());
        let second = Rc::new(MemoryStorage::new());
        assert_eq!(run(first.clone()), run(second.clone()));
        for name in first.names() {
            assert_eq!(first.contents(&name), second.contents(&name));
        }
        let previous = first.contents("cmos.upfj").unwrap();
        run(first.clone());
        let next = first.contents("cmos.upfj").unwrap();
        assert!(next.len() > previous.len());
        assert!(next.starts_with(&previous));
    }

    #[test]
    fn launch_bounds_and_bad_documents_fail_before_any_content_or_record() {
        let (bytes, source) = fixture();
        let text = String::from_utf8(bytes).unwrap();
        for bytes in [
            b"not toml".to_vec(),
            text.replace("size = 32768", "size = 16777217").into_bytes(),
            text.replace("size = 64", "size = 65").into_bytes(),
            vec![b' '; launch::MAX_DOCUMENT_BYTES + 1],
        ] {
            let storage = Rc::new(MemoryStorage::new());
            let req = request(&bytes, &source, storage.clone());
            let expected = crate::PcCore
                .validate(req.platform, req.config)
                .unwrap_err();
            assert_eq!(expected.kind, ErrorKind::Config);
            assert_eq!(crate::PcCore.create(&req).err().unwrap(), expected);
            assert!(source.opened.borrow().is_empty());
            assert!(storage.names().is_empty());
        }
        for platform in ["nes", "ibm-pc-at"] {
            let mut req = request(text.as_bytes(), &source, Rc::new(MemoryStorage::new()));
            req.platform = platform;
            assert_eq!(
                crate::PcCore.create(&req).err().unwrap().kind,
                ErrorKind::Unsupported
            );
            assert_eq!(
                crate::PcCore
                    .validate(platform, req.config)
                    .unwrap_err()
                    .kind,
                ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn launch_retention_warning_does_not_mask_later_cmos_uncertain_write() {
        use unemu_core_api::test_support::Operation;
        let (bytes, source) = fixture();
        let storage = Rc::new(MemoryStorage::new());
        for _ in 0..3 {
            let mut machine = crate::PcCore
                .create(&request(&bytes, &source, storage.clone()))
                .unwrap();
            machine.finish().unwrap();
        }
        let mut machine = crate::PcCore
            .create(&request(&bytes, &source, storage.clone()))
            .unwrap();
        storage.fail_next(
            "a.upfj",
            Operation::Replace,
            Error::new(ErrorKind::Storage, "floppy retention refused"),
        );
        machine
            .change_medium(SlotId(0), MediaAction::Eject)
            .unwrap();
        assert_eq!(machine.slot_states()[0].inserted, None);
        machine.run(RunLimit::steps(100)).unwrap();
        let fatal = Error::new(ErrorKind::StorageUncertain, "CMOS write uncertain");
        storage.fail_next("cmos.upfj", Operation::Append, fatal.clone());
        assert_eq!(machine.finish().unwrap_err(), fatal);
        assert_eq!(machine.status(), Status::Failed);
        let mut messages = Vec::new();
        machine.drain_messages(&mut messages);
        assert!(messages.iter().any(|message| {
            message.level == MessageLevel::Warning
                && message.text.contains("floppy retention refused")
        }));
    }

    #[test]
    fn launch_checkpoint_warning_does_not_mask_a_later_error_on_the_same_record() {
        use unemu_core_api::test_support::Operation;
        let (bytes, source) = fixture();
        let storage = Rc::new(MemoryStorage::new());
        for _ in 0..3 {
            let mut machine = crate::PcCore
                .create(&request(&bytes, &source, storage.clone()))
                .unwrap();
            machine.finish().unwrap();
        }
        let mut machine = crate::PcCore
            .create_machine(&request(&bytes, &source, storage.clone()))
            .unwrap();
        storage.fail_next(
            "a.upfj",
            Operation::Replace,
            Error::new(ErrorKind::Storage, "floppy retention refused"),
        );
        let assembled = machine.assembled();
        assembled
            .persistence
            .iter_mut()
            .find(|handle| handle.owner() == "drive 0")
            .unwrap()
            .checkpoint(&mut assembled.machine)
            .unwrap();
        let fatal = Error::new(ErrorKind::StorageUncertain, "floppy write uncertain");
        storage.fail_next("a.upfj", Operation::Append, fatal.clone());
        assert_eq!(machine.finish().unwrap_err(), fatal);
    }

    #[test]
    fn launch_finish_preserves_the_error_of_the_first_failed_record() {
        use unemu_core_api::test_support::Operation;
        let (bytes, source) = fixture();
        let storage = Rc::new(MemoryStorage::new());
        let mut machine = crate::PcCore
            .create(&request(&bytes, &source, storage.clone()))
            .unwrap();
        machine.run(RunLimit::steps(100)).unwrap();
        let first = Error::new(ErrorKind::StorageUncertain, "CMOS write uncertain");
        storage.fail_next("cmos.upfj", Operation::Append, first.clone());
        storage.fail_next(
            "a.upfj",
            Operation::Append,
            Error::new(ErrorKind::Storage, "floppy close refused"),
        );
        assert_eq!(machine.finish().unwrap_err(), first);
    }

    #[test]
    fn launch_preserves_storage_and_uncertain_write_failures() {
        use unemu_core_api::test_support::Operation;
        let (bytes, source) = fixture();
        for kind in [ErrorKind::Storage, ErrorKind::StorageUncertain] {
            let storage = Rc::new(MemoryStorage::new());
            let mut machine = crate::PcCore
                .create(&request(&bytes, &source, storage.clone()))
                .unwrap();
            let error = Error::new(kind, "injected journal failure");
            storage.fail_next("cmos.upfj", Operation::Append, error.clone());
            machine.run(RunLimit::steps(100)).unwrap();
            assert_eq!(machine.finish().unwrap_err(), error);
            assert_eq!(machine.status(), Status::Failed);
            assert_eq!(
                machine.run(RunLimit::steps(200)).unwrap_err().kind,
                ErrorKind::Usage
            );
            assert_eq!(machine.finish().unwrap_err().kind, ErrorKind::Usage);
            drop(machine);
            assert!(!storage.is_open("cmos.upfj"));
            let mut retry = crate::PcCore
                .create(&request(&bytes, &source, storage))
                .unwrap();
            retry.finish().unwrap();
        }
    }

    #[test]
    fn launch_turbo_option_selects_only_the_initial_button_position() {
        let (bytes, source) = fixture();
        let text = String::from_utf8(bytes).unwrap().replace(
            "[machine.board]",
            "[machine.cpu]\nrate=1\nturbo={clock_mhz=8}\n[machine.board]",
        );
        let run = |turbo| {
            let options = [("allow-unlisted-firmware", "true"), ("turbo", turbo)];
            let mut machine = crate::PcCore
                .create_machine(
                    &request(text.as_bytes(), &source, Rc::new(MemoryStorage::new()))
                        .with_options(&options),
                )
                .unwrap();
            assert_eq!(machine.assembled.machine.turbo_enabled(), turbo == "true");
            machine.run(RunLimit::steps(100)).unwrap();
            let clock = machine.assembled.machine.clock();
            machine.run(RunLimit::until(Ticks(1000))).unwrap();
            assert!((1000..=1001).contains(&machine.assembled.machine.clock().0));
            machine.finish().unwrap();
            clock
        };
        assert!(run("true") < run("false"));
        let (bytes, source) = fixture();
        let options = [("allow-unlisted-firmware", "true"), ("turbo", "true")];
        let error = crate::PcCore
            .create_machine(
                &request(&bytes, &source, Rc::new(MemoryStorage::new())).with_options(&options),
            )
            .err()
            .unwrap();
        assert_eq!(error.kind, ErrorKind::Usage);
        assert!(error.message.contains("requires cpu.turbo.clock_mhz"));
    }

    #[test]
    fn launch_options_change_only_their_permission_or_diagnostics() {
        let (bytes, source) = fixture();
        let req = request(&bytes, &source, Rc::new(MemoryStorage::new()));
        assert_eq!(
            crate::PcCore
                .create(&req.with_options(&[]))
                .err()
                .unwrap()
                .kind,
            ErrorKind::Config
        );
        for trace in ["false", "true"] {
            let opts = [
                ("allow-unlisted-firmware", "true"),
                ("speaker-trace", trace),
            ];
            let mut machine = crate::PcCore
                .create(
                    &request(&bytes, &source, Rc::new(MemoryStorage::new())).with_options(&opts),
                )
                .unwrap();
            machine.run(RunLimit::steps(100)).unwrap();
            let mut messages = Vec::new();
            machine.drain_messages(&mut messages);
            assert_eq!(
                messages
                    .iter()
                    .any(|message| message.text.contains("speaker trace on")),
                trace == "true"
            );
            machine.finish().unwrap();
        }
    }

    #[test]
    fn launch_content_reader_failures_keep_their_kind_at_the_next_boundary() {
        use device::Device;
        let (bytes, mut source) = fixture();
        let digest = Digest::of(&vec![0xf6; 368640]).sha256();
        let reader = Rc::new(MemoryContent::new(source.blobs[&digest].clone()));
        source.reader = Some((digest, reader.clone()));
        let storage = Rc::new(MemoryStorage::new());
        let mut machine = crate::PcCore
            .create_machine(&request(&bytes, &source, storage))
            .unwrap();
        let error = Error::new(ErrorKind::Content, "disk content changed");
        reader.fail_next(error.clone());
        let fdc = machine.fdc().unwrap();
        fdc.port_write(0x3f7, 1, 2).unwrap();
        fdc.port_write(0x3f2, 1, 0x1c).unwrap();
        for byte in [0x06, 0, 0, 0, 1, 2, 1, 0x1b, 0xff] {
            fdc.port_write(0x3f5, 1, byte).unwrap();
        }
        assert_eq!(machine.run(RunLimit::steps(100)).unwrap_err(), error);
        assert_eq!(machine.status(), Status::Failed);
    }

    #[test]
    fn launch_failed_closing_append_restores_the_outgoing_session() {
        use unemu_core_api::test_support::Operation;
        let (bytes, source) = fixture();
        let storage = Rc::new(MemoryStorage::new());
        let mut machine = crate::PcCore
            .create(&request(&bytes, &source, storage.clone()))
            .unwrap();
        let before = machine.slot_states()[0].clone();
        let error = Error::new(ErrorKind::Storage, "injected closing append failure");
        storage.fail_next("a.upfj", Operation::Append, error.clone());
        assert_eq!(
            machine.change_medium(SlotId(0), MediaAction::Eject),
            Err(error)
        );
        assert_eq!(machine.status(), Status::Ready);
        assert_eq!(machine.slot_states()[0], before);
        assert!(storage.is_open("a.upfj"), "outgoing session was reopened");
        machine.run(RunLimit::steps(100)).unwrap();
        machine
            .change_medium(SlotId(0), MediaAction::Eject)
            .unwrap();
        machine
            .change_medium(SlotId(0), MediaAction::Insert(0))
            .unwrap();
        machine.finish().unwrap();
    }

    #[test]
    fn assembly_reselect_signals_disk_change_and_closes_the_journal() {
        use device::Device;
        for action in [
            assembly::FloppyAction::Insert(0),
            assembly::FloppyAction::Next,
        ] {
            let (bytes, source) = fixture();
            let storage = Rc::new(MemoryStorage::new());
            let mut machine = crate::PcCore
                .create_machine(&request(&bytes, &source, storage.clone()))
                .unwrap();
            assert_eq!(
                machine.fdc().unwrap().port_read(0x3f7, 1).unwrap() & 0x80,
                0
            );
            machine
                .assembled
                .change_floppy(DeviceId::Fdc, 0, action)
                .unwrap();
            assert_eq!(
                machine.fdc().unwrap().port_read(0x3f7, 1).unwrap() & 0x80,
                0x80
            );
            let report = machine
                .assembled
                .persistence
                .iter()
                .find(|h| h.floppy == Some((DeviceId::Fdc, 0, 0)))
                .unwrap()
                .mount_report();
            assert_eq!(report.generations.len(), 1);
            assert!(storage.is_open("a.upfj"));
            machine.run(RunLimit::steps(100)).unwrap();
            machine.finish().unwrap();
        }
    }

    #[test]
    fn launch_failed_selection_preserves_the_inserted_medium() {
        use unemu_core_api::test_support::Operation;
        let (bytes, source) = fixture();
        let mut launch = launch::decode(&bytes).unwrap();
        let mut second = launch.config.media[0].clone();
        second.id = "b".into();
        second.source = "assets/b.img".into();
        launch.config.media.push(second);
        launch.config.slots[0].catalogue.push("b".into());
        let mut record = launch.records[0].clone();
        record.medium = "b".into();
        record.name = "b.upfj".into();
        launch.records.push(record);
        let bytes = launch::encode(&launch).unwrap();
        for kind in [ErrorKind::Storage, ErrorKind::StorageUncertain] {
            let storage = Rc::new(MemoryStorage::new());
            let mut machine = crate::PcCore
                .create(&request(&bytes, &source, storage.clone()))
                .unwrap();
            let before = machine.slot_states()[0].clone();
            let error = Error::new(kind, "injected opening failure");
            storage.fail_next("b.upfj", Operation::Open, error.clone());
            assert_eq!(
                machine.change_medium(SlotId(0), MediaAction::Insert(1)),
                Err(error)
            );
            let after = machine.slot_states()[0].clone();
            assert_eq!(after.inserted, before.inserted);
            assert_eq!(after.catalogue, before.catalogue);
            assert!(storage.is_open("a.upfj"));
            assert_eq!(
                machine.status(),
                if kind == ErrorKind::StorageUncertain {
                    Status::Failed
                } else {
                    Status::Ready
                }
            );
            if kind == ErrorKind::Storage {
                machine.run(RunLimit::steps(100)).unwrap();
                machine
                    .change_medium(SlotId(0), MediaAction::Insert(1))
                    .unwrap();
                machine.finish().unwrap();
            }
        }
    }

    #[test]
    fn launch_catalogue_records_open_only_on_insert_and_remain_available_after_create() {
        let (bytes, source) = fixture();
        let mut launch = launch::decode(&bytes).unwrap();
        launch.config.slots[0].inserted = None;
        let bytes = launch::encode(&launch).unwrap();
        let storage = Rc::new(MemoryStorage::new());
        let held = storage.open("a.upfj").unwrap();
        let mut machine = crate::PcCore
            .create(&request(&bytes, &source, storage.clone()))
            .unwrap();
        assert_eq!(machine.slot_states()[0].inserted, None);
        assert_eq!(
            machine
                .change_medium(SlotId(0), MediaAction::Insert(0))
                .unwrap_err()
                .kind,
            ErrorKind::Usage
        );
        drop(held);
        drop(source);
        machine
            .change_medium(SlotId(0), MediaAction::Insert(0))
            .unwrap();
        assert!(storage.is_open("a.upfj"));
        machine.run(RunLimit::steps(100)).unwrap();
        machine.finish().unwrap();
    }

    #[test]
    fn launch_refuses_a_content_size_lie_or_a_broken_rom_pin_without_records() {
        let (bytes, mut source) = fixture();
        let bios_key = source
            .blobs
            .iter()
            .find(|(_, bytes)| bytes.len() == 32768)
            .map(|(key, _)| *key)
            .unwrap();
        let wrong_size = Rc::new(MemoryContent::new(vec![0; 1]));
        source.reader = Some((bios_key, wrong_size));
        let storage = Rc::new(MemoryStorage::new());
        assert_eq!(
            crate::PcCore
                .create(&request(&bytes, &source, storage.clone()))
                .err()
                .unwrap()
                .kind,
            ErrorKind::Content
        );
        assert!(storage.names().is_empty());
        source.reader = None;
        source.blobs.get_mut(&bios_key).unwrap()[0] ^= 1;
        assert_eq!(
            crate::PcCore
                .create(&request(&bytes, &source, storage.clone()))
                .err()
                .unwrap()
                .kind,
            ErrorKind::Content
        );
        assert!(storage.names().is_empty());
    }
}
