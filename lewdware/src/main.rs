#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{env::args_os, fs::File, path::PathBuf};

use anyhow::{Context, Result};
use pollster::block_on;
use shared::user_config::{Mode, load_config};
use winit::event_loop::EventLoop;

use crate::{app::LewdwareApp, utils::report_fatal_startup_error, wgpu::WgpuState};

mod app;
mod audio;
mod egui;
mod error;
mod inner_window;
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

fn main() -> Result<()> {
    let _log_guard = shared::logging::init("lewdware");

    let lock_path = dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lewdware.lock");
    let lock_file = File::create(&lock_path).context("Failed to create lock file")?;
    if lock_file.try_lock().is_err() {
        tracing::error!("Another instance of lewdware is already running");
        return Ok(());
    }

    // Now that we know we're the only instance running, it's safe to clear out any temp files
    // left behind by a previous session (see `utils::prepare_temp_dir`).
    if let Err(err) = utils::prepare_temp_dir() {
        tracing::warn!("Failed to prepare temp dir: {err}");
    }

    utils::raise_fd_limit();
    let mut args = args_os();

    let mut mode_path = None;
    let mut dev_mode = false;
    let mut control_token = None;
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
    }

    // Spawned before `load_config()` so even a config-load failure can be reported to the
    // supervisor; `stop_rx` fires once the supervisor asks this session to gracefully stop
    // (wired to the winit event loop once it exists, below).
    let stop_rx = supervisor_link::connect(control_token);

    let mut config = load_config().inspect_err(|err| report_fatal_startup_error(err))?;

    if let Some(mode_path) = mode_path {
        config.mode = Mode::File { path: mode_path };
    }

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

    // Forwards a graceful-stop request from the supervisor (see `supervisor_link::connect`
    // above) into the same `UserEvent::Exit` path a panic-key press or tray click used to feed
    // directly -- both now live in the supervisor, which is the sole owner of session lifecycle.
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
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
