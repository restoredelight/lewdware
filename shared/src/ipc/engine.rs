use anyhow::{Context, Result};
use interprocess::local_socket::Name;
use serde::{Deserialize, Serialize};

use crate::logging::LogRecord;
use super::{RecvHalf, SendHalf, Stream, prelude::*, socket_name, write_line};

// ─── Engine <-> supervisor protocol (long-lived duplex, one per session) ───────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EngineToSupervisor {
    /// Always the first message on a fresh connection. `token` is the spawning episode's
    /// sequence number, as a string -- lets the supervisor match this connection to the session
    /// it belongs to without a separate lookup table.
    Hello {
        token: String,
    },
    /// The Lua runtime finished initialising and the mode has started running (distinct from
    /// the OS process merely being alive).
    Started,
    Warning {
        message: String,
    },
    RuntimeError {
        message: String,
    },
    FailedToStart {
        message: String,
    },
    Log {
        record: LogRecord,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SupervisorToEngine {
    Stop,
}

/// The socket the supervisor's engine-facing long-lived-connection server listens on.
pub fn engine_link_socket_name() -> Result<Name<'static>> {
    socket_name("supervisor-engine")
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

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use super::*;

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(value: T) {
        let json = serde_json::to_string(&value).unwrap();
        let decoded: T = serde_json::from_str(&json).unwrap();
        assert_eq!(value, decoded);
    }

    #[test]
    fn engine_to_supervisor_variants_roundtrip() {
        roundtrip(EngineToSupervisor::Hello {
            token: "7".to_string(),
        });
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
        roundtrip(EngineToSupervisor::Log {
            record: crate::logging::LogRecord {
                schema_version: 1,
                timestamp: Utc::now(),
                level: crate::logging::LogLevel::Info,
                program: "engine".to_string(),
                target: "lewdware::lua".to_string(),
                message: "started".to_string(),
                file: None,
                line: None,
                session_id: Some("7".to_string()),
                fields: Default::default(),
            },
        });
    }

    #[test]
    fn supervisor_to_engine_variants_roundtrip() {
        roundtrip(SupervisorToEngine::Stop);
    }
}
