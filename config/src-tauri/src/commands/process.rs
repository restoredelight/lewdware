use serde::Serialize;
use tauri::Emitter;

use crate::commands::schedule::{schedule_status_from, ScheduleStatusDto};

// ─── Process management ───────────────────────────────────────────────────────
//
// The config app no longer owns the engine `Child` directly -- it talks to the resident
// supervisor over IPC (starting it on demand), which is now the sole owner of session
// lifecycle, wallpaper safety, and the panic key. See `design/scheduling.md`.

#[tauri::command]
pub async fn launch_lewdware() -> Result<(), String> {
    shared::ipc::ensure_supervisor_running()
        .await
        .map_err(|e| e.to_string())?;

    match shared::ipc::request(&shared::ipc::Request::StartSession {
        mode_path: None,
        dev: false,
    })
    .await
    .map_err(|e| e.to_string())?
    {
        shared::ipc::Response::Error { message } => Err(message),
        // `Busy` means a session is already running -- matches the old no-op-if-already-running
        // idempotency.
        shared::ipc::Response::Ok
        | shared::ipc::Response::Busy { .. }
        | shared::ipc::Response::Status(_) => Ok(()),
        shared::ipc::Response::DevLogReady | shared::ipc::Response::DevLog { .. } => {
            Err("unexpected development log response while launching".to_string())
        }
    }
}

#[tauri::command]
pub async fn stop_lewdware() -> Result<(), String> {
    // Best-effort: if no supervisor is reachable, there's nothing running to stop either --
    // matches the old code's no-op-safe handling of a possibly-`None` tracked `Child`.
    let _ = shared::ipc::request(&shared::ipc::Request::StopSession).await;
    Ok(())
}

#[derive(Serialize, Clone)]
pub struct EngineStatusDto {
    pub running: bool,
    /// Why the engine failed to start, if the last launch we made died before reaching a
    /// running state (e.g. a bad pack, a stale mode reference, a mode-script error), or why it
    /// stopped restarting after repeated crashes.
    pub error: Option<String>,
    /// A non-fatal issue noticed at startup (e.g. a mode built for an older API version). Only
    /// set while `running` is true.
    pub warning: Option<String>,
}

impl EngineStatusDto {
    pub fn stopped() -> Self {
        Self {
            running: false,
            error: None,
            warning: None,
        }
    }
}

pub fn engine_status_from(info: &shared::ipc::StatusInfo) -> EngineStatusDto {
    let error = match &info.session {
        shared::ipc::SessionState::GaveUp { last_error, .. } => last_error
            .clone()
            .or_else(|| Some("Crashed repeatedly and was not restarted".to_string())),
        _ => info.last_exit.as_ref().and_then(|exit| exit.error.clone()),
    };

    EngineStatusDto {
        running: matches!(
            info.session,
            shared::ipc::SessionState::Starting
                | shared::ipc::SessionState::Running { .. }
                | shared::ipc::SessionState::RestartPending { .. }
        ),
        error,
        warning: info.warning.clone(),
    }
}

#[tauri::command]
pub async fn lewdware_running() -> EngineStatusDto {
    match shared::ipc::request(&shared::ipc::Request::Status).await {
        Ok(shared::ipc::Response::Status(info)) => engine_status_from(&info),
        _ => EngineStatusDto::stopped(),
    }
}

/// Pushed to the webview as the `supervisor:status` event whenever the supervisor's state
/// changes (and once with a "stopped" payload when it goes away).
#[derive(Serialize, Clone)]
pub struct SupervisorStatusDto {
    pub engine: EngineStatusDto,
    pub schedule: ScheduleStatusDto,
}

/// Follows the supervisor's status stream for the lifetime of the app, forwarding every update
/// to the webview. When no supervisor is reachable (it self-terminates when idle), retries
/// quietly — the connect attempt against a local socket is cheap.
pub async fn forward_supervisor_status(app: tauri::AppHandle) {
    loop {
        if let Ok(mut subscription) = shared::ipc::subscribe().await {
            while let Ok(info) = subscription.next().await {
                let _ = app.emit(
                    "supervisor:status",
                    SupervisorStatusDto {
                        engine: engine_status_from(&info),
                        schedule: schedule_status_from(&info),
                    },
                );
            }
        }
        // No stream (or it just ended). That usually means the supervisor exited — but a
        // supervisor predating `Subscribe` drops the stream while still answering one-shot
        // requests, so confirm with `Status` before reporting stopped. Against such a
        // supervisor this loop degrades into accurate 2s polling instead of lying.
        let payload = match shared::ipc::request(&shared::ipc::Request::Status).await {
            Ok(shared::ipc::Response::Status(info)) => SupervisorStatusDto {
                engine: engine_status_from(&info),
                schedule: schedule_status_from(&info),
            },
            _ => SupervisorStatusDto {
                engine: EngineStatusDto::stopped(),
                schedule: ScheduleStatusDto {
                    enabled: false,
                    next_session: None,
                },
            },
        };
        let _ = app.emit("supervisor:status", payload);
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
