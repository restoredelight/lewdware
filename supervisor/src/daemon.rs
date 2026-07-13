use std::fs::File;

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use crate::control::{Control, ControlMessage};

pub fn run() -> Result<()> {
    let _log_guard = shared::logging::init("lewdware-supervisor");

    let lock_path = dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lewdware-supervisor.lock");
    let lock_file = File::create(&lock_path).context("failed to create lock file")?;
    if lock_file.try_lock().is_err() {
        tracing::info!("another supervisor instance is already running");
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    return run_linux();
    #[cfg(not(target_os = "linux"))]
    return run_non_linux();
}

#[cfg(target_os = "linux")]
fn run_linux() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let (control_tx, control_rx) = mpsc::channel::<ControlMessage>(64);
        crate::tray::create_tray_icon(control_tx.clone())?;
        run_services(control_tx, control_rx).await;
        Ok(())
    })
}

/// Tray-icon needs a real platform event loop pumped (macOS requires this on the main thread);
/// `tao` is the lightweight, GPU-free event-loop crate tray-icon is designed to pair with -- not
/// `winit`, which the "tiny by charter" supervisor doesn't need. So on these platforms the main
/// thread is dedicated to pumping it, and everything else (IPC servers, session supervision, the
/// panic-key thread) runs on a tokio runtime hosted on a background thread.
#[cfg(not(target_os = "linux"))]
fn run_non_linux() -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    let (control_tx, control_rx) = mpsc::channel::<ControlMessage>(64);

    let tray_tx = control_tx.clone();
    std::thread::spawn(move || {
        rt.block_on(run_services(control_tx, control_rx));
    });

    crate::tray::create_tray_icon(tray_tx)?;

    let event_loop = tao::event_loop::EventLoop::new();
    event_loop.run(move |_event, _target, control_flow| {
        *control_flow = tao::event_loop::ControlFlow::Wait;
    });
}

async fn run_services(control_tx: mpsc::Sender<ControlMessage>, control_rx: mpsc::Receiver<ControlMessage>) {
    let panic_button = shared::user_config::load_config()
        .map(|config| config.panic_button)
        .unwrap_or_else(|err| {
            tracing::warn!("failed to load user config, falling back to the default panic button: {err}");
            shared::user_config::AppConfig::default().panic_button
        });
    crate::panic_key::spawn_panic_thread(control_tx.clone(), panic_button);

    {
        let control_tx = control_tx.clone();
        tokio::spawn(async move {
            if let Err(err) = crate::ipc_server::run(control_tx).await {
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

    Control::new(control_tx).run(control_rx).await;
}
