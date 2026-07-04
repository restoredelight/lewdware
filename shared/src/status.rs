use std::path::PathBuf;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

/// The lifecycle state the engine last reported about itself, written to `status.json` next to
/// `config.json`. Every fatal startup path used to just log and let the Lua thread die silently
/// while the process kept running as an inert tray icon; this gives the config app (or anyone
/// polling the file) something to show the user instead of a launch that appears to do nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineState {
    Starting,
    Running,
    FailedToStart(String),
    Exited,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineStatus {
    pub pid: u32,
    pub state: EngineState,
    /// Non-fatal issue noticed at startup (e.g. a mode built for an older API version). The
    /// engine still runs; this is just surfaced for visibility.
    pub warning: Option<String>,
    /// The most recent Lua error raised while running (an event handler or timer callback).
    /// A single one of these doesn't stop the engine, so it's tracked separately from `state`.
    pub last_runtime_error: Option<String>,
}

impl EngineStatus {
    /// A fresh status for a process that has just started. Callers should write this once at
    /// startup (not merge it), so a previous run's warning/error can't leak into a new one.
    pub fn starting() -> Self {
        Self {
            pid: std::process::id(),
            state: EngineState::Starting,
            warning: None,
            last_runtime_error: None,
        }
    }
}

fn status_path() -> Result<PathBuf> {
    let mut path = dirs::config_dir()
        .ok_or_else(|| anyhow!("Could not find a valid config dir for this OS"))?;

    path.push("lewdware");
    std::fs::create_dir_all(&path)?;

    path.push("status.json");

    Ok(path)
}

pub fn write_status(status: &EngineStatus) -> Result<()> {
    let path = status_path()?;
    let temp_path = path.with_added_extension("tmp");

    std::fs::write(&temp_path, serde_json::to_string(status)?)?;
    std::fs::rename(temp_path, path)?;

    Ok(())
}

pub fn read_status() -> Result<EngineStatus> {
    let path = status_path()?;
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

/// Reads the current status (falling back to a fresh one if it can't be read) and overwrites
/// just `state`, preserving `pid`/`warning`/`last_runtime_error`.
pub fn set_state(state: EngineState) -> Result<()> {
    let mut status = read_status().unwrap_or_else(|_| EngineStatus::starting());
    status.pid = std::process::id();
    status.state = state;
    write_status(&status)
}

/// Reads the current status (falling back to a fresh one if it can't be read) and overwrites
/// just `warning`, preserving `pid`/`state`/`last_runtime_error`.
pub fn set_warning(warning: String) -> Result<()> {
    let mut status = read_status().unwrap_or_else(|_| EngineStatus::starting());
    status.pid = std::process::id();
    status.warning = Some(warning);
    write_status(&status)
}

/// Reads the current status (falling back to a fresh one if it can't be read) and overwrites
/// just `last_runtime_error`, preserving `pid`/`state`/`warning`.
pub fn set_last_runtime_error(error: String) -> Result<()> {
    let mut status = read_status().unwrap_or_else(|_| EngineStatus::starting());
    status.pid = std::process::id();
    status.last_runtime_error = Some(error);
    write_status(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_status_json_roundtrip() {
        let original = EngineStatus {
            pid: 1234,
            state: EngineState::FailedToStart("No pack configured".to_string()),
            warning: Some("stale mode version".to_string()),
            last_runtime_error: None,
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: EngineStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(original, decoded);
    }
}
