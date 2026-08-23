#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use clap::{Parser, Subcommand};
use pollster::block_on;
use shared::{schedule::SessionOverrides, user_config::load_config};
use uuid::Uuid;
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

/// The engine is spawned by the supervisor, never launched by hand, so this is a machine-facing
/// interface rather than a user-facing one.
///
/// Note what is *not* here: the session's pack and mode. Those arrive in
/// `shared::schedule::SESSION_OVERRIDES_ENV` instead, because a command line is readable by every
/// process on the machine and the name of the loaded pack is not something to publish -- see that
/// constant's doc comment.
#[derive(Parser)]
#[command(name = "lewdware-engine")]
struct Cli {
    #[command(subcommand)]
    probe: Option<Probe>,

    /// Identifies this session to the supervisor's engine-link socket.
    #[arg(long)]
    control_token: Option<String>,

    #[arg(long)]
    dev: bool,

    /// The `lw dev` stream this session belongs to, which also turns on log forwarding.
    #[arg(long)]
    dev_stream_id: Option<Uuid>,
}

/// Short-lived probes run by the config app to ask the process that will actually open the display
/// and the audio device what it can see. Each prints JSON to stdout and exits without ever
/// becoming a session.
#[derive(Subcommand)]
enum Probe {
    #[command(name = shared::monitor::LIST_MONITORS_COMMAND)]
    ListMonitors,
    #[command(name = shared::audio::LIST_AUDIO_DEVICES_COMMAND)]
    ListAudioDevices,
    #[command(name = shared::audio::TEST_AUDIO_COMMAND)]
    TestAudio {
        /// `shared::audio::TEST_AUDIO_DEFAULT` for the system default device. Hyphens are allowed
        /// through because this is a cpal device id, not something a person types.
        #[arg(allow_hyphen_values = true)]
        device: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handled before anything else: a probe is a short-lived run, not a session. It must not touch
    // the log file, the temp dir or the fd limit of a real engine that may be running at the same
    // time.
    if let Some(probe) = cli.probe {
        return match probe {
            Probe::ListMonitors => monitor::list_monitors(),
            Probe::ListAudioDevices => audio::list_audio_devices(),
            Probe::TestAudio { device } => audio::play_test_tone(
                (device != shared::audio::TEST_AUDIO_DEFAULT).then_some(device),
            ),
        };
    }

    let Cli {
        control_token,
        dev: dev_mode,
        dev_stream_id,
        ..
    } = cli;

    let stop_rx = supervisor_link::connect(control_token.clone());
    if dev_mode && dev_stream_id.is_some() {
        shared::logging::set_log_handler(|record| {
            supervisor_link::report(shared::ipc::engine::EngineToSupervisor::Log { record });
        });
    }
    let _log_guard = shared::logging::init_with_id("lewdware", control_token)?;

    // TODO: Move this to supervisor
    //
    // Now that we know we're the only instance running, it's safe to clear out any temp files
    // left behind by a previous session (see `utils::prepare_temp_dir`).
    if let Err(err) = utils::prepare_temp_dir() {
        tracing::warn!("Failed to prepare temp dir: {err}");
    }

    utils::raise_fd_limit();

    let mut config = load_config().inspect_err(|err| report_fatal_startup_error(err))?;

    // Per-rule overrides from the supervisor (`design/scheduling.md`, SessionOverrides). Empty for
    // a manual session, which uses whatever `config.json` says. Read here rather than at the top of
    // `main` so that a malformed value is reported the same way a bad config is, over a link that
    // by now exists.
    let overrides =
        SessionOverrides::from_env().inspect_err(|err| report_fatal_startup_error(err))?;

    // Applied after `load_config` so an override beats the stored setting, and before anything
    // reads either. A pack override changes which `Mode::Experience` options apply, since those are
    // scoped by the pack's own UUID -- which is the intended behaviour, not a side effect.
    if let Some(path) = overrides.pack_path {
        config.pack_path = Some(path);
    }
    if let Some(mode) = overrides.mode {
        config.mode = mode;
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
