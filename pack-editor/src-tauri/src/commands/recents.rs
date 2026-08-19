//! The recently-opened list: where it is stored, and keeping it in step as packs are
//! opened, saved and discarded.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use shared::read_pack::Metadata;
use tauri::State;

use crate::pack::MediaPack;
use crate::AppState;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecentPack {
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub draft_id: Option<String>,
    pub last_opened: u64,
}

pub(crate) fn editor_data_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|p| p.join("Lewdware Pack Editor"))
        .ok_or_else(|| "Couldn't find data dir".to_string())
}

pub(crate) fn recent_file() -> Result<PathBuf, String> {
    Ok(editor_data_dir()?.join("recent-packs.json"))
}

pub(crate) fn load_recents() -> Vec<RecentPack> {
    recent_file()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(crate) fn write_recents(recents: &[RecentPack]) -> Result<(), String> {
    let path = recent_file()?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(
        &temp,
        serde_json::to_vec_pretty(recents).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

pub(crate) fn remember_pack(path: &std::path::Path, name: String) -> Result<(), String> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = canonical.to_string_lossy().to_string();
    let mut recents = load_recents();
    recents.retain(|r| r.path.as_deref() != Some(&value));
    recents.insert(
        0,
        RecentPack {
            name,
            path: Some(value),
            draft_id: None,
            last_opened: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
    );
    recents.truncate(10);
    write_recents(&recents)
}

pub(crate) fn remember_draft(id: uuid::Uuid, name: String) -> Result<(), String> {
    let id = id.to_string();
    let mut recents = load_recents();
    recents.retain(|r| r.draft_id.as_deref() != Some(&id));
    recents.insert(
        0,
        RecentPack {
            name,
            path: None,
            draft_id: Some(id),
            last_opened: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
    );
    recents.truncate(10);
    write_recents(&recents)
}

pub(crate) fn forget_draft(id: uuid::Uuid) -> Result<(), String> {
    let id = id.to_string();
    let mut recents = load_recents();
    recents.retain(|r| r.draft_id.as_deref() != Some(&id));
    write_recents(&recents)
}

#[tauri::command]
pub(crate) fn get_recent_packs() -> Vec<RecentPack> {
    let base = editor_data_dir().ok();
    let recents: Vec<_> = load_recents()
        .into_iter()
        .filter_map(|mut r| {
            let saved_exists = r
                .path
                .as_deref()
                .is_some_and(|p| std::path::Path::new(p).is_file());
            let draft_dir = r
                .draft_id
                .as_deref()
                .and_then(|id| base.as_ref().map(|b| b.join(id)));
            let draft_exists = draft_dir
                .as_ref()
                .is_some_and(|d| d.join("UNSAVED").is_file());
            if draft_exists {
                if let Some(name) = draft_dir
                    .and_then(|d| std::fs::read(d.join("Metadata")).ok())
                    .and_then(|b| Metadata::from_buf(&b).ok())
                    .map(|m| m.name)
                {
                    r.name = name;
                }
            }
            (saved_exists || draft_exists).then_some(r)
        })
        .collect();
    let _ = write_recents(&recents);
    recents
}

#[tauri::command]
pub(crate) async fn remove_recent_pack(
    state: State<'_, AppState>,
    path: Option<String>,
    draft_id: Option<String>,
) -> Result<(), String> {
    let draft_id = draft_id
        .map(|id| uuid::Uuid::parse_str(&id).map_err(|error| error.to_string()))
        .transpose()?;
    if let Some(id) = draft_id {
        let lock = state.pack.lock().await;
        if lock.as_ref().is_some_and(|pack| pack.id() == id) {
            return Err("Cannot remove the draft while it is open".into());
        }
        drop(lock);
        let data_dir = dirs::data_dir().ok_or("Couldn't find data dir")?;
        MediaPack::remove_recoverable_draft(&data_dir, id).map_err(|error| error.to_string())?;
    }
    let mut recents = load_recents();
    let draft_id = draft_id.map(|id| id.to_string());
    recents.retain(|r| r.path != path || r.draft_id != draft_id);
    write_recents(&recents)
}
