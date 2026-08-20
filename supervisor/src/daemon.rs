use std::fs::File;

use anyhow::{Context, Result};
use shared::user_config::AppConfig;
use tokio::sync::mpsc;

use crate::control::{Control, ControlMessage};

pub fn run() -> Result<()> {
    let _log_guard = shared::logging::init("lewdware-supervisor")?;

    let lock_path = dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lewdware-supervisor.lock");
    let lock_file = File::create(&lock_path).context("failed to create lock file")?;
    if lock_file.try_lock().is_err() {
        tracing::info!("another supervisor instance is already running");
        return Ok(());
    }

    // If a previous run died mid-episode it may have left a pack's wallpaper on the desktop.
    crate::wallpaper::recover_stranded();

    // Loaded once, synchronously, before the tokio runtime exists -- both the panic key
    // (unchanged) and the schedule engine (new) need their initial values before anything else
    // spins up.
    let config = shared::user_config::load_config().unwrap_or_else(|err| {
        tracing::warn!("failed to load user config, falling back to defaults: {err}");
        AppConfig::default()
    });

    #[cfg(target_os = "linux")]
    return run_linux(config);
    #[cfg(not(target_os = "linux"))]
    return run_non_linux(config);
}

#[cfg(target_os = "linux")]
fn run_linux(config: AppConfig) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let (control_tx, control_rx) = mpsc::channel::<ControlMessage>(64);
        crate::tray::create_tray_icon(control_tx.clone())?;
        run_services(control_tx, control_rx, config).await;
        Ok(())
    })
}

#[cfg(not(target_os = "linux"))]
fn run_non_linux(config: AppConfig) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    let (control_tx, control_rx) = mpsc::channel::<ControlMessage>(64);

    let tray_tx = control_tx.clone();
    std::thread::spawn(move || {
        rt.block_on(run_services(control_tx, control_rx, config));
    });

    crate::tray::create_tray_icon(tray_tx)?;

    let event_loop = tao::event_loop::EventLoop::new();
    event_loop.run(move |_event, _target, control_flow| {
        *control_flow = tao::event_loop::ControlFlow::Wait;
    });
}

async fn run_services(
    control_tx: mpsc::Sender<ControlMessage>,
    control_rx: mpsc::Receiver<ControlMessage>,
    config: AppConfig,
) {
    crate::panic_key::spawn_panic_thread(control_tx.clone(), config.panic_button);

    // Push side of `Request::Subscribe`: Control publishes, the IPC server streams to
    // subscribers. Seeded with an idle status so a subscriber that connects before Control's
    // first publish still gets a coherent snapshot.
    let (status_tx, status_rx) = tokio::sync::watch::channel(shared::ipc::control::StatusInfo {
        session: shared::ipc::control::SessionState::Idle,
        session_kind: None,
        mode_path: None,
        warning: None,
        last_runtime_error: None,
        last_exit: None,
        schedule: shared::ipc::control::ScheduleStatus {
            enabled: config.schedule.enabled,
            next_session: None,
        },
    });
    // Development records are ephemeral and session-scoped. A bounded broadcast channel keeps a
    // slow or disconnected `lw mode dev` client from back-pressuring the engine.
    let (dev_log_tx, _) = tokio::sync::broadcast::channel(1024);

    {
        let control_tx = control_tx.clone();
        let dev_log_rx = dev_log_tx.subscribe();
        tokio::spawn(async move {
            if let Err(err) = crate::ipc_server::run(control_tx, status_rx, dev_log_rx).await {
                tracing::error!("ipc server exited: {err}");
            }
        });
    }
    {
        let control_tx = control_tx.clone();
        tokio::spawn(async move {
            if let Err(err) = crate::engine_link::run(control_tx).await {
                tracing::error!("engine link server exited: {err}");
            }
        });
    }

    Control::new(control_tx, config.schedule, status_tx, dev_log_tx)
        .run(control_rx)
        .await;
}
