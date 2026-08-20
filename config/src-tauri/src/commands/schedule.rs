use serde::Serialize;

use crate::modes::save_to_disk;
use crate::state::State;

#[derive(Serialize, Clone)]
pub struct ScheduleStatusDto {
    pub enabled: bool,
    /// RFC3339, or `null` if nothing's scheduled -- the frontend renders it via `Date`.
    pub next_session: Option<String>,
}

pub fn schedule_status_from(info: &shared::ipc::control::StatusInfo) -> ScheduleStatusDto {
    ScheduleStatusDto {
        enabled: info.schedule.enabled,
        next_session: info.schedule.next_session.map(|t| t.to_rfc3339()),
    }
}

#[tauri::command]
pub async fn get_schedule_status() -> ScheduleStatusDto {
    match shared::ipc::control::request(&shared::ipc::control::Request::Status).await {
        Ok(shared::ipc::control::Response::Status(info)) => schedule_status_from(&info),
        _ => ScheduleStatusDto {
            enabled: false,
            next_session: None,
        },
    }
}

/// The single "enable scheduling" toggle: registers/deregisters the supervisor for OS
/// autostart-at-login (never a separate option -- the supervisor should just always be running
/// when scheduling is on), only persisting `schedule.enabled` once that OS call succeeds (it must
/// never claim a registration state that isn't actually true), then best-effort activates the
/// change in an already-running supervisor -- ensuring one is up (and reloading it) if enabling,
/// or just reloading a reachable one (never spawning one solely to tell it to stop) if disabling.
#[tauri::command]
pub async fn set_schedule_enabled(state: State<'_>, enabled: bool) -> Result<(), String> {
    let result = if enabled {
        crate::autostart::enable()
    } else {
        crate::autostart::disable()
    };
    result.map_err(|e| e.to_string())?;

    {
        let mut config = state.config.lock().unwrap();
        config.schedule.enabled = enabled;
        let uploaded = state.uploaded.lock().unwrap();
        save_to_disk(&config, &uploaded).map_err(|e| e.to_string())?;
    }

    if enabled {
        let _ = shared::ipc::control::ensure_supervisor_running().await;
    }
    let _ = shared::ipc::control::request(&shared::ipc::control::Request::ReloadConfig).await;

    Ok(())
}

/// Pings an already-saved schedule *content* change (windows/quiet-hours/grace-notification, not
/// `enabled` itself -- see `set_schedule_enabled`) through to a resident supervisor, if the
/// schedule is enabled (starting one on demand otherwise would just spawn a supervisor for
/// nothing). Called in addition to (never instead of) the ordinary `save_config`, which already
/// persisted the change; a failure here is best-effort and shouldn't undo that save.
#[tauri::command]
pub async fn reload_supervisor_schedule(state: State<'_>) -> Result<(), String> {
    let enabled = state.config.lock().unwrap().schedule.enabled;
    if !enabled {
        return Ok(());
    }

    shared::ipc::control::ensure_supervisor_running()
        .await
        .map_err(|e| e.to_string())?;

    match shared::ipc::control::request(&shared::ipc::control::Request::ReloadConfig)
        .await
        .map_err(|e| e.to_string())?
    {
        shared::ipc::control::Response::Error { message } => Err(message),
        _ => Ok(()),
    }
}
