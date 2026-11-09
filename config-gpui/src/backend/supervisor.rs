//! Talking to the resident supervisor, which owns the session lifecycle. Ported from
//! `config/src-tauri/src/commands/process.rs`.

use std::time::Duration;

use shared::ipc::control::{self, Request, Response, SessionState, StatusInfo};
use tokio::sync::mpsc;

/// What the UI shows about the engine, read off a `StatusInfo`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineStatus {
    pub running: bool,
    /// Why the last launch died before it was running, or why the supervisor stopped
    /// restarting it after repeated crashes.
    pub error: Option<String>,
    /// A non-fatal startup issue (e.g. a mode built for an older API). Only set while running.
    pub warning: Option<String>,
}

impl EngineStatus {
    pub fn from_info(info: &StatusInfo) -> Self {
        let error = match &info.session {
            SessionState::GaveUp { last_error, .. } => last_error
                .clone()
                .or_else(|| Some("Crashed repeatedly and was not restarted".to_string())),
            _ => info.last_exit.as_ref().and_then(|exit| exit.error.clone()),
        };
        Self {
            running: matches!(
                info.session,
                SessionState::Starting
                    | SessionState::Running { .. }
                    | SessionState::RestartPending { .. }
            ),
            error,
            warning: info.warning.clone(),
        }
    }
}

/// `None` when no supervisor is reachable -- it exits when idle, which means nothing is running.
pub async fn status() -> Option<StatusInfo> {
    match control::request(&Request::Status).await {
        Ok(Response::Status(info)) => Some(info),
        _ => None,
    }
}

pub async fn launch() -> anyhow::Result<()> {
    control::ensure_supervisor_running().await?;
    match control::request(&Request::StartSession {
        mode_path: None,
        dev: false,
        dev_stream_id: None,
        replace: false,
    })
    .await?
    {
        Response::Error { message } => Err(anyhow::anyhow!(message)),
        // `Busy` means a session is already running, which is what was asked for.
        Response::Ok | Response::Busy { .. } | Response::Status(_) => Ok(()),
        Response::DevLogReady | Response::DevLog { .. } => Err(anyhow::anyhow!(
            "unexpected development log response while launching"
        )),
    }
}

/// Best-effort: with no supervisor reachable there is nothing running to stop.
pub async fn stop() {
    let _ = control::request(&Request::StopSession).await;
}

/// Follows the supervisor's status stream for the life of the app, sending `None` whenever no
/// supervisor is reachable. Retries quietly every 2s: connecting to a local socket is cheap.
pub async fn follow(updates: mpsc::UnboundedSender<Option<StatusInfo>>) {
    loop {
        if let Ok(mut subscription) = control::subscribe().await {
            while let Ok(info) = subscription.next().await {
                if updates.send(Some(info)).is_err() {
                    return;
                }
            }
        }
        // The stream ended, which usually means the supervisor exited. One predating `Subscribe`
        // drops the stream but still answers one-shot requests, so confirm before saying stopped;
        // against such a supervisor this becomes 2s polling rather than a wrong answer.
        if updates.send(status().await).is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
