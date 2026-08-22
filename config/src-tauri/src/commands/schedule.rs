use serde::Serialize;

use crate::modes::save_to_disk;
use crate::state::State;

/// What the Scheduling tab may show. Deliberately no firing time for a rate rule -- there is not
/// one to show: under the rate model a firing does not exist until the tick it happens in. The
/// schedule is public, the roll is secret.
#[derive(Serialize, Clone)]
pub struct ScheduleStatusDto {
    pub enabled: bool,
    /// RFC3339, or `null`. Only ever an `At` rule's instant, whose whole promise is the time.
    pub next_exact_session: Option<String>,
    /// RFC3339, or `null`. The earliest a rate rule *could* fire -- a range boundary the user
    /// typed in. `null` while a range is already open.
    pub next_opportunity: Option<String>,
    pub budget_remaining: u32,
    pub budget_total: u32,
    /// RFC3339, or `null`. Set while a post-session or explicit pause is suppressing firing.
    pub cooldown_until: Option<String>,
}

pub fn schedule_status_from(info: &shared::ipc::control::StatusInfo) -> ScheduleStatusDto {
    ScheduleStatusDto {
        enabled: info.schedule.enabled,
        next_exact_session: info.schedule.next_exact_session.map(|t| t.to_rfc3339()),
        next_opportunity: info.schedule.next_opportunity.map(|t| t.to_rfc3339()),
        budget_remaining: info.schedule.budget_remaining,
        budget_total: info.schedule.budget_total,
        cooldown_until: info.schedule.cooldown_until.map(|t| t.to_rfc3339()),
    }
}

#[tauri::command]
pub async fn get_schedule_status() -> ScheduleStatusDto {
    match shared::ipc::control::request(&shared::ipc::control::Request::Status).await {
        Ok(shared::ipc::control::Response::Status(info)) => schedule_status_from(&info),
        _ => ScheduleStatusDto {
            enabled: false,
            next_exact_session: None,
            next_opportunity: None,
            budget_remaining: 0,
            budget_total: 0,
            cooldown_until: None,
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
    let previous = state.config.lock().unwrap().schedule.enabled;
    let result = if enabled {
        crate::autostart::enable()
    } else {
        crate::autostart::disable()
    };
    result.map_err(|e| e.to_string())?;

    let save_result = {
        let mut config = state.config.lock().unwrap();
        config.schedule.enabled = enabled;
        let uploaded = state.uploaded.lock().unwrap();
        let result = save_to_disk(&config, &uploaded);
        if result.is_err() {
            config.schedule.enabled = previous;
        }
        result
    };

    if let Err(save_error) = save_result {
        // The OS registration and config bit are one setting. Put the registration back if the
        // durable half failed, otherwise the UI and next login would disagree about whether
        // scheduling is enabled.
        let rollback = if previous {
            crate::autostart::enable()
        } else {
            crate::autostart::disable()
        };
        return Err(match rollback {
            Ok(()) => save_error.to_string(),
            Err(rollback_error) => format!(
                "{save_error}; also failed to restore autostart after the save failed: {rollback_error}"
            ),
        });
    }

    if enabled {
        let _ = shared::ipc::control::ensure_supervisor_running().await;
    }
    let _ = shared::ipc::control::request(&shared::ipc::control::Request::ReloadConfig).await;

    Ok(())
}

/// Ends a running schedule pause early. Best-effort by
/// design: no resident supervisor means nothing is paused, so there is nothing to end -- and
/// starting one just to tell it that would be absurd.
#[tauri::command]
pub async fn resume_schedule() -> Result<(), String> {
    match shared::ipc::control::request(&shared::ipc::control::Request::ResumeSchedule)
        .await
        .map_err(|e| e.to_string())?
    {
        shared::ipc::control::Response::Error { message } => Err(message),
        _ => Ok(()),
    }
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
