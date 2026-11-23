// SPDX-License-Identifier: AGPL-3.0-or-later
//! Contract lifecycle and boundary regressions over synthetic PC programs.

use super::*;
use unemu_core_api::{Key, KeyState, PadControl, WriteTarget};

fn pc(program: Vec<u8>) -> PcMachine {
    let config = assembly::at_keyboard(program);
    PcMachine::from_assembled(&config, assembly::assemble(&config).unwrap()).unwrap()
}

fn press() -> InputEvent {
    InputEvent::key(KEYBOARD_PORT, Key::A, KeyState::Pressed)
}

fn medium() -> NewMedium<'static> {
    NewMedium::new(
        "blank",
        unemu_core_api::Content::Bytes(vec![]),
        [0; 32],
        0,
        "raw",
        WriteTarget::ReadOnly,
    )
}

#[test]
fn contract_finish_is_terminal_and_preserves_inspection() {
    let mut machine = pc(vec![0xFA, 0xF4]);
    let before = (machine.now(), machine.steps());
    assert_eq!(machine.finish(), Ok(Finished::new()));
    assert_eq!(machine.status(), Status::Finished);
    assert_eq!(machine.finish().unwrap_err().kind, ErrorKind::Usage);
    assert_eq!(machine.status(), Status::Finished);
    assert_eq!((machine.now(), machine.steps()), before);
    assert!(!machine.description().features.media_additions);
    assert!(machine.state_identity().is_none());
    machine.slot_states();
    machine.drain_messages(&mut Vec::new());
    machine.drain_audio(&mut Vec::new()).unwrap();
}

#[test]
fn contract_status_rejections_precede_unsupported_operations() {
    for status in [Status::Off, Status::Failed, Status::Finished] {
        let mut machine = pc(vec![0xEB, 0xFE]);
        machine.input(press()).unwrap();
        let queued = machine.keyboard.pending_events();
        machine.status = status;
        assert_eq!(machine.input(press()).unwrap_err().kind, ErrorKind::Usage);
        assert_eq!(machine.keyboard.pending_events(), queued);
        assert_eq!(
            machine.add_medium(SlotId(255), medium()).unwrap_err().kind,
            ErrorKind::Usage
        );
        assert_eq!(
            machine
                .change_medium(SlotId(255), MediaAction::Insert(255))
                .unwrap_err()
                .kind,
            ErrorKind::Usage
        );
        assert_eq!(
            machine.reset(ResetKind::Soft).unwrap_err().kind,
            ErrorKind::Usage
        );
        assert_eq!(
            machine.reset(ResetKind::Hard).unwrap_err().kind,
            if status == Status::Off {
                ErrorKind::Unsupported
            } else {
                ErrorKind::Usage
            }
        );
        if status != Status::Off {
            assert_eq!(
                machine
                    .run(RunLimit::steps(0).with_steps(None).with_stop_on_frame(true))
                    .unwrap_err()
                    .kind,
                ErrorKind::Usage
            );
            assert_eq!(
                machine.save_state(&mut Vec::new()).unwrap_err().kind,
                ErrorKind::Usage
            );
            assert_eq!(
                machine.load_state(&mut &[][..]).unwrap_err().kind,
                ErrorKind::Usage
            );
            assert_eq!(machine.finish().unwrap_err().kind, ErrorKind::Usage);
            machine.set_outputs(Outputs::NONE.with_audio(true));
            assert!(!machine.audio.enabled.get());
        } else {
            assert_eq!(
                machine.save_state(&mut Vec::new()).unwrap_err().kind,
                ErrorKind::Unsupported
            );
            assert_eq!(
                machine.load_state(&mut &[][..]).unwrap_err().kind,
                ErrorKind::Unsupported
            );
            machine.finish().unwrap();
            assert_eq!(machine.status(), Status::Finished);
        }
        assert_eq!(machine.keyboard.pending_events(), queued);
    }
}

#[test]
fn contract_input_refuses_invalid_ports_kinds_and_buttons_without_queue_changes() {
    let mut machine = pc(vec![0xEB, 0xFE]);
    // Advertise the relative pointer to exercise its controls independently of wiring.
    machine.description.inputs.push(InputPortDescription::new(
        POINTER_PORT,
        InputKind::relative_pointer(3),
        "Mouse",
    ));
    machine.input(press()).unwrap();
    machine
        .input(InputEvent::pointer_motion(POINTER_PORT, 5, 6))
        .unwrap();
    for event in [
        InputEvent::key(PortId(255), Key::A, KeyState::Pressed),
        InputEvent::key(POINTER_PORT, Key::A, KeyState::Pressed),
        InputEvent::pointer_motion(KEYBOARD_PORT, 1, 2),
        InputEvent::pointer_button(POINTER_PORT, 3, true),
        InputEvent::pointer_button(PortId(255), 0, true),
        InputEvent::pointer_position(POINTER_PORT, Some((1, 2))),
        InputEvent::pad(KEYBOARD_PORT, PadControl::A, true),
    ] {
        assert_eq!(
            machine.input(event.clone()).unwrap_err().kind,
            ErrorKind::Usage,
            "{event:?}"
        );
        assert_eq!(machine.keyboard.pending_events(), 1);
    }
    assert_eq!(
        machine.mouse.deliver(|_| true),
        [crate::MouseInput {
            dx: 5,
            dy: 6,
            buttons: 0
        }]
    );
    let mut absent = pc(vec![0xEB, 0xFE]);
    assert_eq!(
        absent
            .input(InputEvent::pointer_motion(POINTER_PORT, 1, 2))
            .unwrap_err()
            .kind,
        ErrorKind::Usage
    );
}

#[test]
fn contract_unbounded_run_accepts_a_guest_halt() {
    let mut machine = pc(vec![0xFA, 0xF4]);
    assert_eq!(
        machine.run(RunLimit::steps(0).with_steps(None)),
        Ok(Stop::Halted)
    );
    assert_eq!(machine.status(), Status::Halted);
    assert_eq!(machine.steps(), 2);
}

#[test]
fn contract_halted_and_off_runs_skip_validation_and_input() {
    for status in [Status::Halted, Status::Off] {
        let mut machine = pc(vec![0xEB, 0xFE]);
        machine.input(press()).unwrap();
        machine.status = status;
        let before = (machine.now(), machine.steps(), keyboard_state(&machine));
        assert_eq!(
            machine.run(RunLimit::steps(0).with_steps(None).with_stop_on_frame(true)),
            Ok(if status == Status::Halted {
                Stop::Halted
            } else {
                Stop::PoweredOff
            })
        );
        assert_eq!(machine.keyboard.pending_events(), 1);
        assert_eq!(
            (machine.now(), machine.steps(), keyboard_state(&machine)),
            before
        );
    }
}

fn keyboard_state(machine: &PcMachine) -> Vec<u8> {
    let index = machine.assembled.device_index[&DeviceId::Keyboard];
    machine
        .assembled
        .machine
        .bus()
        .device(index)
        .unwrap()
        .save()
}

#[test]
fn contract_reached_limits_deliver_input_without_execution() {
    for deadline in [false, true] {
        let mut machine = pc(vec![0xEB, 0xFE]);
        machine.input(press()).unwrap();
        let before = keyboard_state(&machine);
        assert_eq!(
            machine.run(if deadline {
                RunLimit::until(Ticks(0))
            } else {
                RunLimit::steps(0)
            }),
            Ok(if deadline {
                Stop::Deadline
            } else {
                Stop::Steps
            })
        );
        assert_eq!((machine.now(), machine.steps()), (Ticks(0), 0));
        assert_eq!(machine.keyboard.pending_events(), 0);
        assert_ne!(keyboard_state(&machine), before);
    }
}

#[test]
fn contract_ready_reset_clears_queued_and_held_controls_and_keeps_counters() {
    let mut machine = pc(vec![0xEB, 0xFE]);
    machine.run(RunLimit::steps(3)).unwrap();
    machine.input(press()).unwrap();
    machine.mouse.button(0, true);
    machine.mouse.motion(5, 6);
    let before = (machine.now(), machine.steps());
    machine.reset(ResetKind::Soft).unwrap();
    assert_eq!(machine.status(), Status::Ready);
    assert_eq!((machine.now(), machine.steps()), before);
    assert_eq!(machine.keyboard.pending_events(), 0);
    machine.input(InputEvent::ReleaseAll).unwrap();
    assert_eq!(
        machine.keyboard.pending_events(),
        0,
        "reset released held keys"
    );
    assert!(
        machine.mouse.deliver(|_| true).is_empty(),
        "reset cleared mouse state and motion"
    );
}

#[test]
fn contract_halted_reset_clears_mouse_queue_and_held_buttons() {
    let mut machine = pc(vec![0xFA, 0xF4]);
    machine.run(RunLimit::steps(0).with_steps(None)).unwrap();
    machine.input(press()).unwrap();
    machine.mouse.button(0, true);
    machine.mouse.motion(5, 6);
    let before = (machine.now(), machine.steps());
    machine.reset(ResetKind::Soft).unwrap();
    assert_eq!((machine.now(), machine.steps()), before);
    assert_eq!(machine.keyboard.pending_events(), 0);
    machine.mouse.release();
    assert!(machine.mouse.deliver(|_| true).is_empty());
}

#[test]
fn contract_invalid_display_is_usage() {
    let config = assembly::at_vga_text(vec![0xFA, 0xF4]);
    let mut machine =
        PcMachine::from_assembled(&config, super::tests::observed(&config, DeviceId::Vga)).unwrap();
    assert_eq!(
        machine.frame(DisplayId(255)).unwrap_err().kind,
        ErrorKind::Usage
    );
}

#[test]
fn contract_halt_takes_precedence_over_a_simultaneous_step_limit() {
    let mut machine = pc(vec![0xFA, 0xF4]);
    assert_eq!(machine.run(RunLimit::steps(2)), Ok(Stop::Halted));
    assert_eq!(machine.status(), Status::Halted);
}

#[test]
fn contract_unsupported_frame_stop_preserves_pending_input() {
    let mut machine = pc(vec![0xEB, 0xFE]);
    machine.input(press()).unwrap();
    assert_eq!(
        machine
            .run(RunLimit::steps(0).with_stop_on_frame(true))
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    assert_eq!(machine.keyboard.pending_events(), 1);
    assert_eq!((machine.now(), machine.steps()), (Ticks(0), 0));
}

#[test]
fn contract_reset_releases_buttons_already_delivered_to_real_mice() {
    for serial in [false, true] {
        for halted in [false, true] {
            let mut config = assembly::bios(vec![0xF4; 0x8000], Some(vec![0; 1_474_560]));
            for entry in &mut config.devices {
                match &mut entry.kind {
                    DeviceKind::Keyboard(cfg) => cfg.mouse = !serial,
                    DeviceKind::Uart(cfg) => cfg.mouse = serial,
                    _ => {}
                }
            }
            let assembled = assembly::assemble_with_options(
                &config,
                assembly::AssemblyOptions {
                    allow_unlisted_firmware: true,
                },
            )
            .unwrap();
            let mut machine = PcMachine::from_assembled(&config, assembled).unwrap();
            machine
                .input(InputEvent::pointer_button(POINTER_PORT, 0, true))
                .unwrap();
            assert_eq!(
                machine.mouse.inject_at_boundary(&mut machine.assembled)[0].buttons,
                1
            );
            let buttons = |machine: &mut PcMachine| {
                let index = machine.assembled.mouse_device_index().unwrap();
                let dev = machine
                    .assembled
                    .machine
                    .bus_mut()
                    .device_mut(index)
                    .unwrap()
                    .as_any_mut()
                    .unwrap();
                if serial {
                    dev.downcast_mut::<devices::Uart16550>()
                        .unwrap()
                        .serial_device_mut()
                        .unwrap()
                        .save()[3]
                } else {
                    dev.downcast_mut::<devices::I8042>()
                        .unwrap()
                        .aux_device_mut()
                        .unwrap()
                        .save()[6]
                }
            };
            assert_eq!(buttons(&mut machine), 1, "press reached the emulated mouse");
            if halted {
                machine.status = Status::Halted;
            }
            machine.reset(ResetKind::Soft).unwrap();
            assert_eq!(buttons(&mut machine), 0, "reset released device buttons");
            machine.input(InputEvent::ReleaseAll).unwrap();
            assert!(machine
                .mouse
                .inject_at_boundary(&mut machine.assembled)
                .is_empty());
            assert_eq!(buttons(&mut machine), 0);
        }
    }
}
