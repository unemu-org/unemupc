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
    controller_at_speed(extra, Speed::Normal)
}

fn controller_at_speed(extra: &str, speed: Speed) -> (Controller, MemoryStorage) {
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
    let mut controller = Controller::new(machine, speed, &mut Vec::new()).unwrap();
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
    c.focus(false, Duration::from_millis(20));
    c.focus(true, Duration::from_millis(20));
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

#[test]
fn fast_hold_restores_each_startup_speed_and_its_audio_policy() {
    for speed in Speed::ALL {
        let (mut c, _) = controller_at_speed("", speed);
        key(&mut c, ControlLeft, Pressed);
        key(&mut c, AltLeft, Pressed);
        key(&mut c, KeyT, Pressed);
        assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
        assert!(c.session.audio.muted());
        key(&mut c, KeyT, Pressed);
        assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
        key(&mut c, KeyT, Released);
        assert_eq!(c.session.pacing.effective(), speed);
        assert_eq!(c.session.audio.muted(), !speed.audio_enabled());
    }
}

#[test]
fn fast_focus_loss_ends_hold_and_clears_chord_modifiers() {
    let (mut c, _) = controller_at_speed("", Speed::Double);
    key(&mut c, ControlLeft, Pressed);
    key(&mut c, AltLeft, Pressed);
    key(&mut c, KeyT, Pressed);
    assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
    c.focus(false, Duration::from_millis(20));
    assert_eq!(c.session.pacing.effective(), Speed::Double);
    c.focus(true, Duration::from_millis(20));
    key(&mut c, KeyT, Released);
    let messages = tick(&mut c, 20);
    assert!(!messages.contains("key: Key(23)"), "{messages}");
    key(&mut c, KeyT, Pressed);
    assert_eq!(c.session.pacing.effective(), Speed::Double);
}

#[test]
fn fast_toggle_restores_each_startup_speed_and_ignores_repeats() {
    for speed in Speed::ALL {
        let (mut c, _) = controller_at_speed("", speed);
        key(&mut c, ControlLeft, Pressed);
        key(&mut c, AltLeft, Pressed);
        key(&mut c, KeyU, Pressed);
        assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
        key(&mut c, KeyU, Pressed);
        assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
        key(&mut c, KeyU, Released);
        assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
        c.focus(false, Duration::from_millis(20));
        c.focus(true, Duration::from_millis(20));
        assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
        chord(&mut c, KeyU);
        assert_eq!(c.session.pacing.effective(), speed);
        assert_eq!(c.session.audio.muted(), !speed.audio_enabled());
    }
}

#[test]
fn fast_hold_and_toggle_keep_their_states_independent() {
    let (mut c, _) = controller_at_speed("", Speed::Quadruple);
    key(&mut c, ControlLeft, Pressed);
    key(&mut c, AltLeft, Pressed);
    key(&mut c, KeyT, Pressed);
    key(&mut c, KeyU, Pressed);
    key(&mut c, KeyU, Released);
    key(&mut c, KeyT, Released);
    assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
    key(&mut c, KeyT, Pressed);
    key(&mut c, KeyU, Pressed);
    key(&mut c, KeyU, Released);
    assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
    key(&mut c, KeyT, Released);
    assert_eq!(c.session.pacing.effective(), Speed::Quadruple);
}

#[test]
fn fast_chord_letters_never_reach_guest_but_plain_letters_do() {
    let (mut c, _) = controller("");
    chord(&mut c, KeyT);
    chord(&mut c, KeyU);
    chord(&mut c, KeyU);
    let messages = tick(&mut c, 20);
    for letter in [Key::T, Key::U] {
        for state in [KeyState::Pressed, KeyState::Released] {
            let event = format!("{:?}", InputEvent::key(PortId(0), letter, state));
            assert!(!messages.contains(&event), "{messages}");
        }
    }
    for letter in [KeyT, KeyU] {
        key(&mut c, letter, Pressed);
        key(&mut c, letter, Released);
    }
    let messages = tick(&mut c, 40);
    for letter in [Key::T, Key::U] {
        for state in [KeyState::Pressed, KeyState::Released] {
            let event = format!("{:?}", InputEvent::key(PortId(0), letter, state));
            assert!(messages.contains(&event), "{messages}");
        }
    }
}

#[test]
fn fast_held_guest_letter_is_not_reinterpreted_as_a_chord() {
    for letter in [KeyT, KeyU] {
        let (mut c, _) = controller("");
        key(&mut c, letter, Pressed);
        key(&mut c, ControlLeft, Pressed);
        key(&mut c, AltLeft, Pressed);
        key(&mut c, letter, Pressed);
        assert_eq!(c.session.pacing.effective(), Speed::Normal);
        key(&mut c, letter, Released);
        assert_eq!(c.session.pacing.effective(), Speed::Normal);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn fast_command_chords_hold_toggle_and_consume_letters_on_macos() {
    let (mut c, _) = controller("");
    key(&mut c, SuperLeft, Pressed);
    key(&mut c, KeyT, Pressed);
    assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
    key(&mut c, SuperLeft, Released);
    assert_eq!(c.session.pacing.effective(), Speed::Normal);
    key(&mut c, KeyT, Released);
    key(&mut c, SuperRight, Pressed);
    key(&mut c, KeyU, Pressed);
    key(&mut c, KeyU, Released);
    assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
    key(&mut c, KeyU, Pressed);
    key(&mut c, KeyU, Released);
    assert_eq!(c.session.pacing.effective(), Speed::Normal);
    key(&mut c, SuperRight, Released);
    let messages = tick(&mut c, 20);
    assert!(!messages.contains("key: Key(23)"), "{messages}");
    assert!(!messages.contains("key: Key(24)"), "{messages}");
}

#[test]
fn fast_window_title_tracks_effective_speed_and_returns_to_machine_name() {
    for (speed, expected) in [
        (Speed::Normal, "at-games-open"),
        (Speed::Double, "at-games-open — set 200%"),
        (Speed::Quadruple, "at-games-open — set 400%"),
        (Speed::Unlimited, "at-games-open — unlimited"),
    ] {
        let (mut c, _) = controller_at_speed("", speed);
        assert_eq!(c.window_title("at-games-open"), expected);
        key(&mut c, ControlLeft, Pressed);
        key(&mut c, AltLeft, Pressed);
        key(&mut c, KeyT, Pressed);
        assert_eq!(c.window_title("at-games-open"), "at-games-open — unlimited");
        key(&mut c, KeyT, Released);
        assert_eq!(c.window_title("at-games-open"), expected);
        chord(&mut c, KeyU);
        assert_eq!(c.window_title("at-games-open"), "at-games-open — unlimited");
        chord(&mut c, KeyU);
        assert_eq!(c.window_title("at-games-open"), expected);
    }
}

#[test]
fn fast_releasing_a_modifier_ends_hold_without_forwarding_t() {
    for modifier in [ControlLeft, AltLeft] {
        let (mut c, _) = controller("");
        key(&mut c, ControlLeft, Pressed);
        key(&mut c, AltLeft, Pressed);
        key(&mut c, KeyT, Pressed);
        assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
        key(&mut c, modifier, Released);
        assert_eq!(c.session.pacing.effective(), Speed::Normal);
        key(&mut c, KeyT, Pressed);
        key(&mut c, KeyT, Released);
        let messages = tick(&mut c, 20);
        assert!(!messages.contains("key: Key(23)"), "{messages}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn fast_command_modifier_notification_alone_ends_hold() {
    use winit::keyboard::ModifiersState;
    let (mut c, _) = controller("");
    key(&mut c, SuperLeft, Pressed);
    key(&mut c, KeyT, Pressed);
    assert_eq!(c.session.pacing.effective(), Speed::Unlimited);
    c.modifiers_changed(ModifiersState::empty(), Duration::from_millis(20));
    assert_eq!(c.session.pacing.effective(), Speed::Normal);
    key(&mut c, KeyT, Released);
    let messages = tick(&mut c, 20);
    assert!(!messages.contains("key: Key(23)"), "{messages}");
}

/// A guest that redraws 18 times a virtual second.
const DOOM_LIKE: &str = "clock 18000000\nframe 1000000";

/// The 60 Hz paint deadline `frame` of the session's host time.
fn frame_time(frame: u64) -> Duration {
    Duration::from_nanos((frame * 1_000_000_000).div_ceil(60))
}

/// Present the 60 Hz frames `frames` of the session's host time.
fn present(c: &mut Controller, frames: std::ops::RangeInclusive<u64>) {
    for frame in frames {
        assert!(
            c.tick(frame_time(frame), &mut Vec::new()).unwrap(),
            "frame {frame}"
        );
    }
}

/// Present `frames` and check that none of them shows rates yet.
fn present_without_rates(c: &mut Controller, frames: std::ops::RangeInclusive<u64>) {
    for frame in frames {
        present(c, frame..=frame);
        assert_eq!(c.window_title("doom"), "doom", "frame {frame}");
    }
}

/// Press or release a whole Ctrl+Alt chord at host time `elapsed`.
fn chord_at(c: &mut Controller, code: KeyCode, elapsed: Duration) {
    for (key, state) in [
        (ControlLeft, Pressed),
        (AltLeft, Pressed),
        (code, Pressed),
        (code, Released),
        (AltLeft, Released),
        (ControlLeft, Released),
    ] {
        c.key(key, state, elapsed, &mut Vec::new()).unwrap();
    }
}

#[test]
fn title_shows_speed_and_fps_only_after_a_full_second() {
    let (mut c, _) = controller(DOOM_LIKE);
    present_without_rates(&mut c, 0..=59);
    present(&mut c, 60..=60);
    assert_eq!(c.window_title("doom"), "doom — 100% · 18 fps");
    // The reading holds until the next window closes.
    present(&mut c, 61..=119);
    assert_eq!(c.window_title("doom"), "doom — 100% · 18 fps");
}

#[test]
fn title_marks_the_selected_speed_apart_from_the_measured_one() {
    let (mut c, _) = controller_at_speed(DOOM_LIKE, Speed::Double);
    present(&mut c, 0..=60);
    assert_eq!(c.window_title("doom"), "doom — set 200% · 200% · 36 fps");
    // A new pace shows only its label until a window at that pace closes.
    chord(&mut c, KeyU);
    assert_eq!(c.window_title("doom"), "doom — unlimited");
    chord(&mut c, KeyU);
    assert_eq!(c.window_title("doom"), "doom — set 200%");
}

#[test]
fn paused_title_shows_the_label_and_no_rates_until_a_new_second() {
    let (mut c, _) = controller(DOOM_LIKE);
    present(&mut c, 0..=60);
    chord(&mut c, KeyP);
    assert!(c.paused);
    assert_eq!(c.window_title("doom"), "doom — paused");
    // Resumed with no frame presented while paused: the pause itself must
    // drop the reading and the window it was measuring.
    chord_at(&mut c, KeyP, Duration::from_secs(5));
    assert!(!c.paused);
    assert_eq!(c.window_title("doom"), "doom");
    // The paused seconds do not count against the kept clock.
    present_without_rates(&mut c, 300..=359);
    present(&mut c, 360..=360);
    assert_eq!(c.window_title("doom"), "doom — 100% · 18 fps");
}

#[test]
fn hidden_window_drops_the_rates_until_a_full_second_after_it_shows_again() {
    let (mut c, _) = controller(DOOM_LIKE);
    present(&mut c, 0..=60);
    c.occlude(true, frame_time(60));
    assert_eq!(c.window_title("doom"), "doom");
    c.occlude(false, Duration::from_secs(5));
    present_without_rates(&mut c, 300..=359);
    present(&mut c, 360..=360);
    assert_eq!(c.window_title("doom"), "doom — 100% · 18 fps");
}

#[test]
fn releasing_a_fast_forward_hold_drops_the_rates_measured_during_it() {
    let (mut c, _) = controller(DOOM_LIKE);
    present(&mut c, 0..=60);
    for key in [ControlLeft, AltLeft, KeyT] {
        c.key(key, Pressed, frame_time(60), &mut Vec::new())
            .unwrap();
    }
    assert_eq!(c.window_title("doom"), "doom — unlimited");
    // An Unlimited slice that runs fully never ends, so the hold runs on the
    // host's slice budget; only the shape of its reading is checked.
    c.session.full_slices = false;
    present(&mut c, 61..=121);
    let held = c.window_title("doom");
    assert!(held.starts_with("doom — unlimited · "), "{held}");
    assert!(held.ends_with(" fps"), "{held}");
    c.key(KeyT, Released, frame_time(121), &mut Vec::new())
        .unwrap();
    assert_eq!(c.session.pacing.effective(), Speed::Normal);
    assert_eq!(c.window_title("doom"), "doom");
}

#[test]
fn a_machine_that_powers_off_loses_its_rates_on_the_next_tick() {
    let (mut c, _) = controller(&format!("{DOOM_LIKE}\nat 20000000 power-off"));
    present(&mut c, 0..=60);
    assert_eq!(c.window_title("doom"), "doom — 100% · 18 fps");
    let mut frame = 60;
    while c.session.running {
        frame += 1;
        assert!(frame < 120, "the machine never powered off");
        present(&mut c, frame..=frame);
    }
    assert!(matches!(
        c.session.machine().status(),
        Status::Off | Status::Finished
    ));
    c.tick(frame_time(frame + 1), &mut Vec::new()).unwrap();
    assert_eq!(c.window_title("doom"), "doom");
}
