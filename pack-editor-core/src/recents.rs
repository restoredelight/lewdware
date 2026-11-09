//! The recently-opened list: where it is stored, and keeping it in step as packs are opened,
//! saved and discarded. Shared by both editors, which read and write the same file.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use shared::pack::Metadata;

use crate::pack::MediaPack;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RecentPack {
    pub name: String,
    /// A saved pack's file.
    #[serde(default)]
    pub path: Option<String>,
    /// A never-saved draft's id; its working copy is `editor_data_dir()/<id>`.
    #[serde(default)]
    pub draft_id: Option<String>,
    /// Seconds since the Unix epoch.
    pub last_opened: u64,
}

const LIMIT: usize = 10;

pub fn editor_data_dir() -> Result<PathBuf> {
    dirs::data_dir()
        .map(|p| p.join("Lewdware Pack Editor"))
        .ok_or_else(|| anyhow!("Couldn't find data dir"))
}

fn recent_file() -> Result<PathBuf> {
    Ok(editor_data_dir()?.join("recent-packs.json"))
}

fn load() -> Vec<RecentPack> {
    recent_file()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write(recents: &[RecentPack]) -> Result<()> {
    let path = recent_file()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(recents)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn remember_pack(path: &Path, name: String) -> Result<()> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = canonical.to_string_lossy().to_string();
    let mut recents = load();
    recents.retain(|r| r.path.as_deref() != Some(&value));
    recents.insert(
        0,
        RecentPack {
            name,
            path: Some(value),
            draft_id: None,
            last_opened: now(),
        },
    );
    recents.truncate(LIMIT);
    write(&recents)
}

pub fn remember_draft(id: uuid::Uuid, name: String) -> Result<()> {
    let id = id.to_string();
    let mut recents = load();
    recents.retain(|r| r.draft_id.as_deref() != Some(&id));
    recents.insert(
        0,
        RecentPack {
            name,
            path: None,
            draft_id: Some(id),
            last_opened: now(),
        },
    );
    recents.truncate(LIMIT);
    write(&recents)
}

pub fn forget_draft(id: uuid::Uuid) -> Result<()> {
    let id = id.to_string();
    let mut recents = load();
    recents.retain(|r| r.draft_id.as_deref() != Some(&id));
    write(&recents)
}

/// The list, minus entries whose pack or draft no longer exists (which are also dropped from the
/// file). A draft shows the name its working copy has now, not the one it was remembered under.
pub fn list() -> Vec<RecentPack> {
    let base = editor_data_dir().ok();
    let recents: Vec<_> = load()
        .into_iter()
        .filter_map(|mut r| {
            let saved_exists = r.path.as_deref().is_some_and(|p| Path::new(p).is_file());
            let draft_dir = r
                .draft_id
                .as_deref()
                .and_then(|id| base.as_ref().map(|b| b.join(id)));
            let draft_exists = draft_dir
                .as_ref()
                .is_some_and(|d| d.join("UNSAVED").is_file());
            if draft_exists {
                if let Some(name) = draft_dir
                    .and_then(|d| rusqlite::Connection::open(d.join("index.db")).ok())
                    .and_then(|connection| Metadata::read(&connection).ok())
                    .map(|m| m.name)
                {
                    r.name = name;
                }
            }
            (saved_exists || draft_exists).then_some(r)
        })
        .collect();
    let _ = write(&recents);
    recents
}

/// Drops an entry. A draft's working copy is deleted with it, so the caller must make sure that
/// draft isn't the one open.
pub fn remove(path: Option<&str>, draft_id: Option<uuid::Uuid>) -> Result<()> {
    if let Some(id) = draft_id {
        let data_dir = dirs::data_dir().ok_or_else(|| anyhow!("Couldn't find data dir"))?;
        MediaPack::remove_recoverable_draft(&data_dir, id)?;
    }
    let mut recents = load();
    let draft_id = draft_id.map(|id| id.to_string());
    recents.retain(|r| r.path.as_deref() != path || r.draft_id != draft_id);
    write(&recents)
}
