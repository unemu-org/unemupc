// SPDX-License-Identifier: AGPL-3.0-or-later
//! A bare picture window for the PC core, with host I/O confined to this application.
//! See backlog/docs/commands.md § Viewing a machine. Guest time stays in the core.

mod args;
mod controller;
mod meter;
mod screen;

use args::Args;
use controller::Controller;
use std::{
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{DeviceEvent, DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::PhysicalKey,
    window::{Window, WindowId},
};

struct App {
    controller: Controller,
    title: String,
    /// The title last given to the window, so it is set only when it changes
    /// (the rates refresh about once a second, not every frame).
    shown_title: String,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    failure: Option<String>,
    cursor: Option<(f32, f32)>,
}

impl App {
    fn elapsed(&self) -> Duration {
        self.controller
            .session
            .started
            .map_or(Duration::ZERO, |start| start.elapsed())
    }

    fn close(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.controller.close(&mut std::io::stderr()) {
            self.failure.get_or_insert_with(|| error.to_string());
        }
        event_loop.exit();
    }

    fn fail(&mut self, error: impl std::fmt::Display, event_loop: &ActiveEventLoop) {
        eprintln!("{error}");
        self.failure = Some(error.to_string());
        self.close(event_loop);
    }

    fn pointer(&mut self) {
        let size = self.window.as_ref().map(|window| window.inner_size());
        let position = size
            .and_then(|size| {
                self.controller
                    .session
                    .picture()
                    .and_then(|p| screen::geometry(p, size.width, size.height))
            })
            .and_then(|shown| self.cursor.and_then(|(x, y)| shown.pointer_position(x, y)));
        self.controller.pointer(position);
    }

    fn update_title(&mut self) {
        let Some(window) = &self.window else {
            return;
        };
        let title = self.controller.window_title(&self.title);
        if title != self.shown_title {
            window.set_title(&title);
            self.shown_title = title;
        }
    }

    fn paint(&mut self) -> anyhow::Result<()> {
        let Some(window) = &self.window else {
            return Ok(());
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };
        surface
            .resize(width, height)
            .map_err(|e| anyhow::anyhow!("resize: {e}"))?;
        let mut buffer = surface
            .buffer_mut()
            .map_err(|e| anyhow::anyhow!("pixel buffer: {e}"))?;
        screen::blit(
            self.controller.session.picture(),
            size.width,
            size.height,
            &mut buffer,
        );
        window.pre_present_notify();
        buffer
            .present()
            .map_err(|e| anyhow::anyhow!("present: {e}"))?;
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            self.controller.occlude(false, self.elapsed());
            return;
        }
        self.shown_title = self.controller.window_title(&self.title);
        let result = (|| -> anyhow::Result<_> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title(&self.shown_title)
                        .with_inner_size(LogicalSize::new(960, 720)),
                )?,
            );
            let context = softbuffer::Context::new(window.clone())
                .map_err(|e| anyhow::anyhow!("display: {e}"))?;
            let surface = softbuffer::Surface::new(&context, window.clone())
                .map_err(|e| anyhow::anyhow!("surface: {e}"))?;
            Ok((window, surface))
        })();
        match result {
            Ok((window, surface)) => {
                self.controller.session.window = Some(window.clone());
                self.controller.session.started = Some(Instant::now());
                if self
                    .controller
                    .session
                    .machine()
                    .description()
                    .audio
                    .is_some()
                {
                    self.controller.session.audio.open(&mut std::io::stderr());
                }
                self.window = Some(window);
                self.surface = Some(surface);
                self.controller.occlude(false, Duration::ZERO);
            }
            Err(error) => self.fail(error, event_loop),
        }
    }

    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        self.controller.focus(false, self.elapsed());
        self.update_title();
        self.controller.occlude(true, self.elapsed());
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.close(event_loop),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.paint() {
                    self.fail(error, event_loop);
                }
            }
            WindowEvent::Resized(size) => {
                self.controller
                    .occlude(size.width == 0 || size.height == 0, self.elapsed());
                self.pointer();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::Occluded(occluded) => self.controller.occlude(occluded, self.elapsed()),
            WindowEvent::Focused(focused) => {
                self.controller.focus(focused, self.elapsed());
                self.update_title();
            }
            #[cfg(target_os = "macos")]
            WindowEvent::ModifiersChanged(modifiers) => {
                self.controller
                    .modifiers_changed(modifiers.state(), self.elapsed());
                self.update_title();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(key) = event.physical_key {
                    match self.controller.key(
                        key,
                        event.state,
                        self.elapsed(),
                        &mut std::io::stderr(),
                    ) {
                        Ok(true) => event_loop.exit(),
                        Ok(false) => {}
                        Err(error) => self.fail(error, event_loop),
                    }
                    self.update_title();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some((position.x as f32, position.y as f32));
                self.pointer();
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor = None;
                self.pointer();
            }
            WindowEvent::MouseInput { button, state, .. } => self.controller.button(button, state),
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            self.controller.motion(dx, dy);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() || event_loop.exiting() {
            return;
        }
        match self.controller.tick(self.elapsed(), &mut std::io::stderr()) {
            Ok(true) => {
                self.pointer();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            Ok(false) => {}
            Err(error) => {
                self.fail(error, event_loop);
                return;
            }
        }
        self.update_title();
        let deadline = self.controller.session.next_paint;
        event_loop.set_control_flow(match (self.controller.session.started, deadline) {
            (Some(start), Some(deadline)) => ControlFlow::WaitUntil(start + deadline),
            _ => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Err(error) = self.controller.close(&mut std::io::stderr()) {
            eprintln!("finish: {error}");
            self.failure.get_or_insert_with(|| error.to_string());
        }
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    use assembly::{DeviceId, ObservationPolicy, ObservationRequest, OutputKind};
    let (name, loaded) = unemupc::run::load_named_machine(
        &args.machine,
        assembly::AssemblyOptions {
            allow_unlisted_firmware: args.allow_unlisted_firmware,
        },
        &args.rom_dirs,
        &mut std::io::stderr(),
    )?;
    let display = loaded
        .cards
        .iter()
        .find(|(_, device)| matches!(device, DeviceId::Mda | DeviceId::Vga))
        .map(|(_, device)| *device)
        .ok_or_else(|| anyhow::anyhow!("machine has no display"))?;
    let title = name.unwrap_or_else(|| {
        args.machine
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let assembled = loaded.assemble(&ObservationPolicy {
        routes: vec![ObservationRequest {
            device: display,
            output: OutputKind::TextScanout,
            byte_limit: 0,
        }],
        ..ObservationPolicy::default()
    })?;
    let machine = unemu_pc_core::PcMachine::from_assembled(&loaded.config, assembled)?;
    let controller = Controller::new(Box::new(machine), args.speed, &mut std::io::stderr())
        .map_err(|e| anyhow::anyhow!("start session: {e}"))?;
    let mut app = App {
        controller,
        title,
        shown_title: String::new(),
        window: None,
        surface: None,
        failure: None,
        cursor: None,
    };
    // Creating the host event loop can fail after journals have opened.
    let result = EventLoop::new().and_then(|event_loop| event_loop.run_app(&mut app));
    let finished = app.controller.close(&mut std::io::stderr());
    result?;
    finished.map_err(|e| anyhow::anyhow!("finish: {e}"))?;
    if let Some(error) = app.failure {
        anyhow::bail!(error);
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let words: Vec<_> = std::env::args_os().skip(1).collect();
    if matches!(words.as_slice(), [word] if word == "--version" || word == "--about") {
        print!(
            "unemupc-view {}\n{}",
            env!("CARGO_PKG_VERSION"),
            unemupc::notice()
        );
        return std::process::ExitCode::SUCCESS;
    }
    if words.is_empty() || matches!(words.as_slice(), [word] if word == "--help" || word == "-h") {
        print!("{}", args::USAGE);
        return std::process::ExitCode::SUCCESS;
    }
    let args = match Args::parse(&words) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{}{error}", args::USAGE);
            return 2.into();
        }
    };
    match run(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            1.into()
        }
    }
}
