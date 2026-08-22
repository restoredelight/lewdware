use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use interprocess::local_socket::Name;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

use super::{RecvHalf, SendHalf, Stream, prelude::*, read_line, socket_name, write_line};
use crate::logging::LogRecord;
use crate::utils::silence_command;

// ─── CLI / config-app protocol (one-shot request/response) ────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Status,
    /// Turns the connection into a long-lived status stream: the supervisor immediately writes
    /// one `Response::Status` line, then another on every state change, until the client
    /// disconnects. The push counterpart to `Status` polling.
    SubscribeStatus,
    /// Streams structured records for one `lw mode dev` owner. The client subscribes before its
    /// first restart so no startup records are lost.
    SubscribeDevLogs {
        stream_id: Uuid,
    },
    StartSession {
        mode_path: Option<PathBuf>,
        dev: bool,
        dev_stream_id: Option<Uuid>,
        replace: bool,
    },
    StopSession,
    StopImmediately,
    /// Ends a running schedule pause early.
    ///
    /// The pause lives on the schedule engine rather than in `config.json`, so it survives a
    /// config edit and even toggling scheduling off and on. That makes an explicit way to end it
    /// necessary rather than a convenience: without one, the only route is the tray, and on a
    /// desktop with no system tray (stock GNOME) there would be no route at all.
    ResumeSchedule,
    /// Re-reads `config.json` from disk and feeds the new `schedule` section to the schedule
    /// engine -- how a config-app edit reaches an already-resident supervisor (`design/
    /// scheduling.md`'s schedule vocabulary).
    ReloadConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    Status(StatusInfo),
    DevLogReady,
    DevLog { record: LogRecord },
    Ok,
    Busy { current: SessionSummary },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusInfo {
    pub session: SessionState,
    pub session_kind: Option<SessionKind>,
    pub mode_path: Option<PathBuf>,
    pub warning: Option<String>,
    pub last_runtime_error: Option<String>,
    pub last_exit: Option<ExitInfo>,
    pub schedule: ScheduleStatus,
}

/// Schedule-engine status for display -- not itself the source of truth (that's
/// `AppConfig::schedule` on disk), just a snapshot for the config app's Scheduling tab.
///
/// Carries no firing time for a rate rule, and could not if it wanted to: under the rate model a
/// firing does not exist until the tick it happens in. v1 shipped its pre-rolled instant over this
/// wire, so blurring it in the UI would have changed nothing -- anyone reading the socket learned
/// the answer. The schedule is public, the roll is secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleStatus {
    pub enabled: bool,
    /// The next `Trigger::At` firing. Naming the instant is that trigger's whole promise, so this
    /// is the one time the UI may display.
    ///
    /// UTC on the wire, not local -- the supervisor and config-app are separate OS processes that
    /// shouldn't need to agree on what "local" means (e.g. under different `TZ` environments); the
    /// frontend renders it timezone-aware for free via `Date`.
    pub next_exact_session: Option<DateTime<Utc>>,
    /// The earliest instant a rate rule could next fire: a range boundary the user typed in, never
    /// a draw. `None` while a range is already open, where the honest answer is not a time.
    pub next_opportunity: Option<DateTime<Utc>>,
    /// Firings left in the current budget period, and the total, summed across rate rules --
    /// "2 of 3 remaining today".
    pub budget_remaining: u32,
    pub budget_total: u32,
    /// Set while a post-session or explicit pause is suppressing firing.
    pub cooldown_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKind {
    Manual,
    Dev,
    /// Opened unattended by the schedule engine -- see `design/scheduling.md`: "a closing window
    /// ends only schedule-owned sessions." A manual session overlapping a window's boundary is
    /// never touched by the schedule engine, which is what this variant makes checkable.
    Scheduled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionState {
    Idle,
    Starting,
    Running {
        pid: u32,
    },
    RestartPending {
        attempt: u32,
        max_attempts: u32,
        retry_in_secs: u64,
        last_error: Option<String>,
    },
    GaveUp {
        attempts: u32,
        last_error: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub classification: ExitClassification,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitClassification {
    Graceful,
    Killed,
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub kind: SessionKind,
    pub mode_path: Option<PathBuf>,
    pub dev: bool,
}

/// The socket the supervisor's CLI/config-app-facing request/response server listens on.
pub fn control_socket_name() -> Result<Name<'static>> {
    socket_name("supervisor")
}

/// Sends one request to the resident supervisor and returns its response. Connects fresh
/// every call -- matches scheduling.md's "keep the protocol trivial" (no multiplexing, no
/// persistent client connection).
pub async fn request(req: &Request) -> Result<Response> {
    let name = control_socket_name()?;
    let conn = Stream::connect(name)
        .await
        .context("could not connect to the supervisor")?;

    write_line(&conn, &serde_json::to_string(req)?).await?;
    let line = read_line(&conn).await?;
    Ok(serde_json::from_str(line.trim_end())?)
}

/// A live status stream from the supervisor (`Request::SubscribeStatus`). Yields the current
/// status immediately, then again on every state change; errors once the supervisor goes
/// away, at which point the caller reconnects (or treats the supervisor as stopped).
pub struct StatusSubscription {
    reader: BufReader<RecvHalf>,
    // Keeps the write half open for the lifetime of the subscription.
    _send: SendHalf,
}

impl StatusSubscription {
    pub async fn next(&mut self) -> Result<StatusInfo> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("status subscription closed");
        }
        match serde_json::from_str(line.trim_end())? {
            Response::Status(info) => Ok(info),
            other => Err(anyhow!(
                "unexpected response on status subscription: {other:?}"
            )),
        }
    }
}

/// Opens a status subscription to the resident supervisor. Fails if none is reachable —
/// callers decide whether that means "stopped" or "spawn one".
pub async fn subscribe() -> Result<StatusSubscription> {
    let name = control_socket_name()?;
    let conn = Stream::connect(name)
        .await
        .context("could not connect to the supervisor")?;
    write_line(&conn, &serde_json::to_string(&Request::SubscribeStatus)?).await?;
    let (recv, send) = conn.split();
    Ok(StatusSubscription {
        reader: BufReader::new(recv),
        _send: send,
    })
}

pub struct DevLogSubscription {
    reader: BufReader<RecvHalf>,
    _send: SendHalf,
}

impl DevLogSubscription {
    pub async fn next(&mut self) -> Result<crate::logging::LogRecord> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("development log subscription closed");
        }
        match serde_json::from_str(line.trim_end())? {
            Response::DevLog { record } => Ok(record),
            other => Err(anyhow!(
                "unexpected response on development log subscription: {other:?}"
            )),
        }
    }
}

pub async fn subscribe_dev_logs(stream_id: Uuid) -> Result<DevLogSubscription> {
    let name = control_socket_name()?;
    let conn = Stream::connect(name)
        .await
        .context("could not connect to the supervisor")?;
    write_line(
        &conn,
        &serde_json::to_string(&Request::SubscribeDevLogs { stream_id })?,
    )
    .await?;
    let line = read_line(&conn).await?;
    if !matches!(
        serde_json::from_str::<Response>(line.trim_end())?,
        Response::DevLogReady
    ) {
        anyhow::bail!("supervisor did not acknowledge development log subscription");
    }
    let (recv, send) = conn.split();
    Ok(DevLogSubscription {
        reader: BufReader::new(recv),
        _send: send,
    })
}

/// Ensures a supervisor is reachable, spawning one on demand (per scheduling.md: "the config
/// app always talks to the supervisor, starting it on demand if absent") if `request` can't
/// reach one yet.
pub async fn ensure_supervisor_running() -> Result<()> {
    if request(&Request::Status).await.is_ok() {
        return Ok(());
    }

    let mut cmd = crate::binaries::find_supervisor_binary()
        .ok_or_else(|| anyhow!("could not find the lewdware-supervisor binary"))?;
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    silence_command(&mut cmd);
    cmd.spawn().context("could not spawn the supervisor")?;

    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if request(&Request::Status).await.is_ok() {
            return Ok(());
        }
    }

    Err(anyhow!("supervisor did not become reachable"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(value: T) {
        let json = serde_json::to_string(&value).unwrap();
        let decoded: T = serde_json::from_str(&json).unwrap();
        assert_eq!(value, decoded);
    }

    #[test]
    fn request_variants_roundtrip() {
        roundtrip(Request::Status);
        roundtrip(Request::SubscribeStatus);
        roundtrip(Request::StartSession {
            mode_path: Some(PathBuf::from("/tmp/mode.lwmode")),
            dev: true,
            dev_stream_id: Some(Uuid::nil()),
            replace: true,
        });
        roundtrip(Request::StartSession {
            mode_path: None,
            dev: false,
            dev_stream_id: None,
            replace: false,
        });
        roundtrip(Request::StopSession);
        roundtrip(Request::StopImmediately);
        roundtrip(Request::ReloadConfig);
    }

    #[test]
    fn response_variants_roundtrip() {
        roundtrip(Response::Ok);
        roundtrip(Response::DevLogReady);
        roundtrip(Response::Error {
            message: "boom".to_string(),
        });
        roundtrip(Response::Busy {
            current: SessionSummary {
                kind: SessionKind::Dev,
                mode_path: Some(PathBuf::from("/tmp/mode.lwmode")),
                dev: true,
            },
        });
        roundtrip(Response::Status(StatusInfo {
            session: SessionState::RestartPending {
                attempt: 1,
                max_attempts: 3,
                retry_in_secs: 2,
                last_error: Some("crashed".to_string()),
            },
            session_kind: Some(SessionKind::Scheduled),
            mode_path: None,
            warning: Some("stale mode version".to_string()),
            last_runtime_error: Some("nil index".to_string()),
            last_exit: Some(ExitInfo {
                code: Some(101),
                classification: ExitClassification::Crashed,
                error: Some("crashed".to_string()),
            }),
            schedule: ScheduleStatus {
                enabled: true,
                next_exact_session: Some(Utc::now()),
                next_opportunity: None,
                budget_remaining: 2,
                budget_total: 3,
                cooldown_until: None,
            },
        }));
    }

    #[test]
    fn schedule_status_variants_roundtrip() {
        roundtrip(ScheduleStatus {
            enabled: false,
            next_exact_session: None,
            next_opportunity: None,
            budget_remaining: 0,
            budget_total: 0,
            cooldown_until: None,
        });
        // An `At` rule pending, a rate rule's range still shut, and a cooldown running: the three
        // things the UI may say, all at once.
        roundtrip(ScheduleStatus {
            enabled: true,
            next_exact_session: Some(Utc::now()),
            next_opportunity: Some(Utc::now()),
            budget_remaining: 1,
            budget_total: 3,
            cooldown_until: Some(Utc::now()),
        });
    }

    #[test]
    fn session_kind_scheduled_roundtrips() {
        roundtrip(SessionKind::Scheduled);
    }

    #[test]
    fn session_state_variants_roundtrip() {
        roundtrip(SessionState::Idle);
        roundtrip(SessionState::Starting);
        roundtrip(SessionState::Running { pid: 1234 });
        roundtrip(SessionState::GaveUp {
            attempts: 3,
            last_error: None,
        });
    }
}
