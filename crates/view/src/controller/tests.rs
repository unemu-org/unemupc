// SPDX-License-Identifier: AGPL-3.0-or-later
//! Host routing tested through the scripted core, without window or audio devices.

use super::*;
use std::rc::Rc;
use unemu_core_api::{
    test_support::{MemoryContent, MemoryStorage},
    Content, ContentSource, Core, CreateRequest, Error, InputEvent, Key, KeyState, OfferedContent,
    PortId, Status,
};
use unemu_test_core::{TestCore, BATTERY_RECORD};
use winit::{
    event::ElementState::{Pressed, Released},
    keyboard::KeyCode::*,
};

struct Source(Vec<u8>);
impl ContentSource for Source {
    fn open(&self, _digest: &[u8; 32], size: u64) -> std::result::Result<Content, Error> {
        assert_eq!(size, self.0.len() as u64);
        Ok(Content::Bytes(self.0.clone()))
    }
}

fn controller(extra: &str) -> (Controller, MemoryStorage) {
    let text = format!(
        "unemu-test-core 1\nkeyboard 0\npointer 1 3\necho-input\nresets soft,hard\n{extra}\n"
    );
    let bytes = text.into_bytes();
    let source = Source(bytes.clone());
    let storage = MemoryStorage::new();
    let identified = TestCore
        .identify(&OfferedContent::new(
            "test.utc",
            [1; 32],
            bytes.len() as u64,
            &MemoryContent::new(bytes),
        ))
        .unwrap()
        .unwrap();
    let machine = TestCore
        .create(&CreateRequest::new(
            identified.platform,
            identified.config.as_document(),
            &source,
            Rc::new(storage.clone()),
        ))
        .unwrap();
    let mut controller = Controller::new(machine, Speed::Normal, &mut Vec::new()).unwrap();
    controller
        .session
        .audio
        .refuse_device("tests never open an audio device");
    controller.session.full_slices = true;
    (controller, storage)
}

fn key(controller: &mut Controller, key: KeyCode, state: ElementState) -> bool {
    controller
        .key(key, state, Duration::from_millis(20), &mut Vec::new())
        .unwrap()
}

fn chord(controller: &mut Controller, code: KeyCode) -> bool {
    key(controller, ControlLeft, Pressed);
    key(controller, AltLeft, Pressed);
    let quit = key(controller, code, Pressed);
    key(controller, code, Released);
    key(controller, AltLeft, Released);
    key(controller, ControlLeft, Released);
    quit
}

fn tick(controller: &mut Controller, millis: u64) -> String {
    let mut output = Vec::new();
    controller
        .tick(Duration::from_millis(millis), &mut output)
        .unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn physical_keys_reach_the_contract_as_usb_usages_and_ignore_repeats() {
    let (mut c, _) = controller("");
    key(&mut c, KeyA, Pressed);
    key(&mut c, KeyA, Pressed);
    key(&mut c, KeyA, Released);
    let messages = tick(&mut c, 20);
    let down = format!(
        "{:?}",
        InputEvent::key(PortId(0), Key::A, KeyState::Pressed)
    );
    let up = format!(
        "{:?}",
        InputEvent::key(PortId(0), Key::A, KeyState::Released)
    );
    assert_eq!(messages.matches(&down).count(), 1, "{messages}");
    assert_eq!(messages.matches(&up).count(), 1, "{messages}");
}

#[test]
fn capture_click_is_swallowed_motion_is_routed_and_release_chord_is_not_a_guest_g() {
    let (mut c, _) = controller("");
    c.button(MouseButton::Left, Pressed);
    assert!(!c.session.mouse.captured());
    c.pointer(Some((100, 100)));
    c.button(MouseButton::Left, Pressed);
    c.button(MouseButton::Left, Released);
    assert!(c.session.mouse.captured());
    c.motion(3.0, -2.0);
    chord(&mut c, KeyG);
    assert!(!c.session.mouse.captured());
    c.motion(100.0, 100.0);
    let messages = tick(&mut c, 20);
    let motion = format!("{:?}", InputEvent::pointer_motion(PortId(1), 3, 2));
    assert!(messages.contains(&motion), "{messages}");
    assert!(!messages.contains("PointerButton"), "{messages}");
    assert!(!messages.contains("key: Key(10)"), "{messages}");
    assert!(!messages.contains("100"), "{messages}");
}

#[cfg(target_os = "macos")]
#[test]
fn command_g_releases_capture_on_macos() {
    let (mut c, _) = controller("");
    c.pointer(Some((1, 1)));
    c.button(MouseButton::Left, Pressed);
    key(&mut c, SuperLeft, Pressed);
    key(&mut c, KeyG, Pressed);
    key(&mut c, KeyG, Released);
    key(&mut c, SuperLeft, Released);
    assert!(!c.session.mouse.captured());
    let messages = tick(&mut c, 20);
    assert!(!messages.contains("key: Key(10)"), "{messages}");
}

#[test]
fn close_finishes_and_commits_and_releases_the_record_even_while_paused() {
    let (mut c, storage) = controller("battery");
    assert!(storage.is_open(BATTERY_RECORD));
    chord(&mut c, KeyP);
    c.close(&mut Vec::new()).unwrap();
    assert_eq!(c.session.machine().status(), Status::Finished);
    assert!(!storage.is_open(BATTERY_RECORD));
    assert!(storage.durable(BATTERY_RECORD).unwrap().ends_with(b"1\n"));
    c.close(&mut Vec::new()).unwrap();
}

#[test]
fn quit_chord_finishes_before_requesting_exit() {
    let (mut c, storage) = controller("battery");
    assert!(chord(&mut c, KeyQ));
    assert_eq!(c.session.machine().status(), Status::Finished);
    assert!(!storage.is_open(BATTERY_RECORD));
}

#[test]
fn pause_releases_capture_blocks_input_and_resumes_without_catching_up() {
    let (mut c, _) = controller("");
    tick(&mut c, 20);
    c.pointer(Some((1, 1)));
    c.button(MouseButton::Left, Pressed);
    chord(&mut c, KeyP);
    assert!(c.paused);
    assert!(!c.session.mouse.captured());
    let now = c.session.machine().now();
    key(&mut c, KeyA, Pressed);
    key(&mut c, KeyA, Released);
    assert!(!c.tick(Duration::from_secs(10), &mut Vec::new()).unwrap());
    assert_eq!(c.session.machine().now(), now);
    assert!(c.session.next_paint.is_none());
    c.key(
        ControlLeft,
        Pressed,
        Duration::from_secs(10),
        &mut Vec::new(),
    )
    .unwrap();
    c.key(AltLeft, Pressed, Duration::from_secs(10), &mut Vec::new())
        .unwrap();
    c.key(KeyP, Pressed, Duration::from_secs(10), &mut Vec::new())
        .unwrap();
    assert!(!c.paused);
    let messages = tick(&mut c, 10_001);
    assert!(c.session.machine().now().0 - now.0 <= 61);
    assert!(messages.contains("ReleaseAll"), "{messages}");
    let press = format!(
        "{:?}",
        InputEvent::key(PortId(0), Key::A, KeyState::Pressed)
    );
    assert!(!messages.contains(&press), "{messages}");
}

#[test]
fn reset_chord_resumes_halted_machine_and_preserves_pause() {
    let (mut c, _) = controller("at 10 halt");
    tick(&mut c, 20);
    assert!(c.session.halted);
    let now = c.session.machine().now();
    chord(&mut c, KeyP);
    chord(&mut c, KeyR);
    assert_eq!(c.session.machine().status(), Status::Ready);
    assert!(c.session.running);
    assert_eq!(c.session.machine().now(), now);
    assert!(c.paused);
    assert!(!c.tick(Duration::from_secs(10), &mut Vec::new()).unwrap());
}

#[test]
fn held_letter_is_not_reinterpreted_as_a_new_chord_and_focus_loss_clears_modifiers() {
    let (mut c, _) = controller("");
    key(&mut c, KeyP, Pressed);
    key(&mut c, ControlLeft, Pressed);
    key(&mut c, AltLeft, Pressed);
    key(&mut c, KeyP, Pressed);
    assert!(!c.paused);
    c.focus(false);
    c.focus(true);
    key(&mut c, KeyP, Pressed);
    assert!(!c.paused);
    assert!(tick(&mut c, 20).contains("ReleaseAll"));
}

#[test]
fn pacing_requests_only_due_frames_and_occlusion_does_not_accumulate_debt() {
    let (mut c, _) = controller("");
    assert!(c.tick(Duration::ZERO, &mut Vec::new()).unwrap());
    assert!(!c.tick(Duration::from_millis(1), &mut Vec::new()).unwrap());
    assert!(c.tick(Duration::from_millis(17), &mut Vec::new()).unwrap());
    c.occlude(true, Duration::from_millis(20));
    let now = c.session.machine().now();
    assert!(!c.tick(Duration::from_secs(20), &mut Vec::new()).unwrap());
    c.occlude(false, Duration::from_secs(20));
    assert!(c
        .tick(Duration::from_millis(20_001), &mut Vec::new())
        .unwrap());
    assert_eq!(c.session.machine().now().0 - now.0, 60);
}

#[test]
fn machine_messages_go_to_diagnostics_and_host_audio_remains_closed() {
    let (mut c, _) = controller("sound 48000 440\nat 10 warning machine warning");
    let output = tick(&mut c, 20);
    assert!(output.contains("machine warning"));
    assert_eq!(c.session.audio.opened_for_test(), (false, false));
}

#[test]
fn failed_finish_is_reported_and_never_retried_by_later_exit_callbacks() {
    use unemu_core_api::{test_support::Operation, ErrorKind};
    let (mut c, storage) = controller("battery");
    storage.fail_next(
        BATTERY_RECORD,
        Operation::Replace,
        Error::new(ErrorKind::StorageUncertain, "finish failed"),
    );
    assert!(c
        .close(&mut Vec::new())
        .unwrap_err()
        .to_string()
        .contains("finish failed"));
    assert!(storage.durable(BATTERY_RECORD).unwrap().is_empty());
    c.close(&mut Vec::new()).unwrap();
    assert!(storage.durable(BATTERY_RECORD).unwrap().is_empty());
}
