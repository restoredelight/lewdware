use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ─── CLI / config-app protocol (one-shot request/response) ────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Request {
    Status,
    StartSession {
        mode_path: Option<PathBuf>,
        dev: bool,
    },
    RestartSession {
        mode_path: PathBuf,
        dev: bool,
    },
    StopSession,
    Panic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Response {
    Status(StatusInfo),
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionKind {
    Manual,
    Dev,
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

// ─── Engine <-> supervisor protocol (long-lived duplex, one per session) ───────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EngineToSupervisor {
    /// Always the first message on a fresh connection. `token` is the spawning episode's
    /// sequence number, as a string -- lets the supervisor match this connection to the session
    /// it belongs to without a separate lookup table.
    Hello { token: String },
    /// The Lua runtime finished initialising and the mode has started running (distinct from
    /// the OS process merely being alive).
    Started,
    Warning { message: String },
    RuntimeError { message: String },
    FailedToStart { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SupervisorToEngine {
    Stop,
}

#[cfg(feature = "ipc")]
mod client {
    use std::time::Duration;

    use anyhow::{Context, Result, anyhow};
    use interprocess::local_socket::{
        GenericFilePath, GenericNamespaced, ListenerOptions, Name, ToFsName, ToNsName,
        tokio::prelude::*,
    };
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::{EngineToSupervisor, Request, Response};

    // Re-exported so downstream crates (the supervisor) don't need their own direct
    // `interprocess` dependency just to name these types.
    pub use interprocess::local_socket::tokio::{Listener, RecvHalf, SendHalf, Stream};

    /// The extension traits (`.accept()`, `.split()`, `AsyncRead`/`AsyncWrite` on the types
    /// above) live behind anonymous trait imports in `interprocess`'s own prelude -- re-exported
    /// under an explicit name here so callers bring them into scope via
    /// `use shared::ipc::prelude::*;` instead of needing a direct `interprocess` dependency.
    pub mod prelude {
        pub use interprocess::local_socket::tokio::prelude::*;
    }

    fn socket_name(id: &str) -> Result<Name<'static>> {
        if GenericNamespaced::is_supported() {
            Ok(format!("lewdware-{id}.sock").to_ns_name::<GenericNamespaced>()?)
        } else {
            let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
            Ok(dir
                .join(format!("lewdware-{id}.sock"))
                .to_fs_name::<GenericFilePath>()?)
        }
    }

    /// The socket the supervisor's CLI/config-app-facing request/response server listens on.
    pub fn control_socket_name() -> Result<Name<'static>> {
        socket_name("supervisor")
    }

    /// The socket the supervisor's engine-facing long-lived-connection server listens on.
    pub fn engine_link_socket_name() -> Result<Name<'static>> {
        socket_name("supervisor-engine")
    }

    /// `try_overwrite(true)`: a supervisor that was hard-killed without a chance to clean up its
    /// own socket file would otherwise permanently block every future daemon from binding again.
    pub fn bind_listener(name: Name<'static>) -> Result<Listener> {
        Ok(ListenerOptions::new()
            .name(name)
            .try_overwrite(true)
            .create_tokio()?)
    }

    async fn read_line(stream: &Stream) -> Result<String> {
        let mut recver = BufReader::new(stream);
        let mut line = String::new();
        let n = recver.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("connection closed");
        }
        Ok(line)
    }

    async fn write_line(stream: &Stream, line: &str) -> Result<()> {
        let mut sender = stream;
        sender.write_all(line.as_bytes()).await?;
        sender.write_all(b"\n").await?;
        Ok(())
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

    /// Ensures a supervisor is reachable, spawning one on demand (per scheduling.md: "the config
    /// app always talks to the supervisor, starting it on demand if absent") if `request` can't
    /// reach one yet.
    pub async fn ensure_supervisor_running() -> Result<()> {
        if request(&Request::Status).await.is_ok() {
            return Ok(());
        }

        let mut cmd = crate::child::find_supervisor_binary()
            .ok_or_else(|| anyhow!("could not find the lewdware-supervisor binary"))?;
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        cmd.spawn().context("could not spawn the supervisor")?;

        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if request(&Request::Status).await.is_ok() {
                return Ok(());
            }
        }

        Err(anyhow!("supervisor did not become reachable"))
    }

    /// An engine's live connection to its supervising session. `recv`/`send` are split so one
    /// task can watch for an incoming `SupervisorToEngine::Stop` while other callers push
    /// `EngineToSupervisor` reports independently.
    pub struct EngineLink {
        pub recv: RecvHalf,
        pub send: SendHalf,
    }

    /// Connects to the supervisor's engine-link socket and identifies this session via `token`
    /// (the spawning episode's sequence number). The engine is always the connecting side; the
    /// supervisor is always the one listening.
    pub async fn connect_engine_link(token: &str) -> Result<EngineLink> {
        let name = engine_link_socket_name()?;
        let conn = Stream::connect(name)
            .await
            .context("could not connect to the supervisor's engine link")?;

        write_line(
            &conn,
            &serde_json::to_string(&EngineToSupervisor::Hello {
                token: token.to_string(),
            })?,
        )
        .await?;

        let (recv, send) = conn.split();
        Ok(EngineLink { recv, send })
    }
}

#[cfg(feature = "ipc")]
pub use client::*;

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
        roundtrip(Request::StartSession {
            mode_path: Some(PathBuf::from("/tmp/mode.lwmode")),
            dev: true,
        });
        roundtrip(Request::StartSession { mode_path: None, dev: false });
        roundtrip(Request::RestartSession {
            mode_path: PathBuf::from("/tmp/mode.lwmode"),
            dev: true,
        });
        roundtrip(Request::StopSession);
        roundtrip(Request::Panic);
    }

    #[test]
    fn response_variants_roundtrip() {
        roundtrip(Response::Ok);
        roundtrip(Response::Error { message: "boom".to_string() });
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
            session_kind: Some(SessionKind::Manual),
            mode_path: None,
            warning: Some("stale mode version".to_string()),
            last_runtime_error: Some("nil index".to_string()),
            last_exit: Some(ExitInfo {
                code: Some(101),
                classification: ExitClassification::Crashed,
                error: Some("crashed".to_string()),
            }),
        }));
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

    #[test]
    fn engine_to_supervisor_variants_roundtrip() {
        roundtrip(EngineToSupervisor::Hello { token: "7".to_string() });
        roundtrip(EngineToSupervisor::Started);
        roundtrip(EngineToSupervisor::Warning {
            message: "stale mode version".to_string(),
        });
        roundtrip(EngineToSupervisor::RuntimeError {
            message: "nil index".to_string(),
        });
        roundtrip(EngineToSupervisor::FailedToStart {
            message: "no pack configured".to_string(),
        });
    }

    #[test]
    fn supervisor_to_engine_variants_roundtrip() {
        roundtrip(SupervisorToEngine::Stop);
    }
}
