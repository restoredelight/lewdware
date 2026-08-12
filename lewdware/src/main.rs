#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{env::args_os, path::PathBuf};

use anyhow::{Context, Result};
use pollster::block_on;
use shared::user_config::{Mode, load_config};
use winit::event_loop::EventLoop;

use crate::{app::LewdwareApp, utils::report_fatal_startup_error, wgpu::WgpuState};

mod app;
mod audio;
mod error;
mod lua;
mod media;
mod monitor;
mod supervisor_link;
mod text_font;
mod utils;
mod video;
mod wgpu;
mod window;
mod zero_copy;

/// The argument following `flag`, if the flag was passed at all.
///
/// Only used by the probe flags handled before the main argument loop below, which needs to read
/// its own values from a single pass over the arguments.
fn flag_value(flag: &str) -> Option<std::ffi::OsString> {
    let mut args = args_os();
    args.find(|arg| arg == flag)?;
    args.next()
}

fn main() -> Result<()> {
    // Handled before anything else: this is a short-lived probe run by the config app, not a
    // session. It must not touch the log file, the temp dir or the fd limit of a real engine that
    // may be running at the same time.
    if args_os().any(|arg| arg == shared::monitor::LIST_MONITORS_FLAG) {
        return monitor::list_monitors();
    }

    // The audio equivalents, and short-lived probes for the same reason -- see
    // `shared::audio::LIST_AUDIO_DEVICES_FLAG`.
    if args_os().any(|arg| arg == shared::audio::LIST_AUDIO_DEVICES_FLAG) {
        return audio::list_audio_devices();
    }

    if let Some(device) = flag_value(shared::audio::TEST_AUDIO_FLAG) {
        let device = device.to_string_lossy().into_owned();
        return audio::play_test_tone(
            (device != shared::audio::TEST_AUDIO_DEFAULT).then_some(device),
        );
    }

    let mut args = args_os();

    let mut mode_path = None;
    let mut dev_mode = false;
    let mut control_token = None;
    let mut dev_stream_id = None;
    while let Some(arg) = args.next() {
        if &arg == "--mode-path" {
            mode_path = Some(PathBuf::from(args.next().context("No mode path provided")?));
        }

        if &arg == "--dev" {
            dev_mode = true;
        }

        if &arg == "--control-token" {
            control_token = Some(
                args.next()
                    .context("No control token provided")?
                    .to_string_lossy()
                    .into_owned(),
            );
        }

        if &arg == "--dev-stream-id" {
            dev_stream_id = Some(
                args.next()
                    .context("No development stream ID provided")?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }

    let stop_rx = supervisor_link::connect(control_token.clone());
    if dev_mode && dev_stream_id.is_some() {
        shared::logging::set_record_sink(|record| {
            supervisor_link::report(shared::ipc::EngineToSupervisor::Log { record });
        });
    }
    let _log_guard = shared::logging::init_with_session("lewdware", control_token);

    // TODO: Move this to supervisor
    //
    // Now that we know we're the only instance running, it's safe to clear out any temp files
    // left behind by a previous session (see `utils::prepare_temp_dir`).
    if let Err(err) = utils::prepare_temp_dir() {
        tracing::warn!("Failed to prepare temp dir: {err}");
    }

    utils::raise_fd_limit();

    let mut config = load_config().inspect_err(|err| report_fatal_startup_error(err))?;

    if let Some(mode_path) = mode_path {
        config.mode = Mode::File { path: mode_path };
    }

    // Published before any media can start, since it is read once and cached the first time a sink
    // is opened.
    // Only the id: the stored name is a label for the config app's picker and has no bearing on
    // which device gets opened.
    audio::set_output_device(config.audio_device.as_ref().map(|device| device.id.clone()));

    tracing::debug!("{:?}", config);

    let mut event_loop_builder = EventLoop::with_user_event();

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;

        // Wayland doesn't support a bunch of stuff we need (e.g. setting the position of windows).
        // So we use XWayland instead.
        event_loop_builder.with_x11();
    }

    let event_loop = event_loop_builder
        .build()
        .inspect_err(|err| report_fatal_startup_error(err))?;

    #[cfg(target_vendor = "apple")]
    utils::opt_in_secure_restorable_state();

    let proxy = event_loop.create_proxy();

    let wgpu_state = match block_on(WgpuState::new(event_loop.owned_display_handle())) {
        Ok(state) => Some(std::sync::Arc::new(state)),
        Err(err) => {
            tracing::warn!("GPU initialisation failed, falling back to software rendering: {err}");
            None
        }
    };

    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            // The supervisor has told us to exit.
            if stop_rx.recv().is_ok() {
                let _ = proxy.send_event(app::UserEvent::Exit);
            }
        });
    }

    let mut app = LewdwareApp::new(wgpu_state, proxy, config, dev_mode)
        .inspect_err(|err| report_fatal_startup_error(err))?;
    event_loop.run_app(&mut app)?;

    Ok(())
}
