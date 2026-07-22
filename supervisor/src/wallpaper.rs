//! The supervisor's wallpaper safety net.
//!
//! The engine restores the wallpaper itself when an episode ends cleanly. This is the backstop for
//! when it doesn't -- a crash, a kill, a panic-key exit -- so the user is never left staring at
//! whatever the pack put on their desktop.
//!
//! Because the supervisor can itself be killed, the snapshot is written to disk rather than only
//! being held in memory: if we die between setting a wallpaper and restoring it, the file is the
//! only record of what the desktop used to look like. [`recover_stranded`] picks it up on the next
//! start.

use std::{fs, path::PathBuf};

use shared::wallpaper::Snapshot;

fn state_path() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("lewdware");
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("wallpaper-snapshot.json"))
}

/// Captures the current wallpaper and persists it.
///
/// A [`Snapshot::Unsupported`] result means we could not read the current wallpaper, and so must
/// not change it -- see [`Snapshot::is_restorable`].
pub fn snapshot() -> Snapshot {
    // Read fresh rather than threaded down from `Control`, which only keeps the schedule slice of
    // the config. This runs once per episode start, and picking the file up here means a restore
    // image chosen in the config app takes effect on the next episode with no reload plumbing.
    let fallback = shared::user_config::load_config()
        .map(|config| config.wallpaper)
        .unwrap_or_default();

    let snapshot = shared::wallpaper::snapshot(fallback.fallback_image());

    if !snapshot.is_restorable() {
        return snapshot;
    }

    if let Some(path) = state_path() {
        match serde_json::to_vec(&snapshot) {
            Ok(encoded) => {
                if let Err(err) = fs::write(&path, encoded) {
                    tracing::warn!("failed to persist wallpaper snapshot: {err}");
                }
            }
            Err(err) => tracing::warn!("failed to encode wallpaper snapshot: {err}"),
        }
    }

    snapshot
}

/// Puts the wallpaper back and drops the persisted copy.
pub fn restore(snapshot: &Snapshot) {
    if snapshot.is_restorable()
        && let Err(err) = shared::wallpaper::restore(snapshot)
    {
        // Deliberately not clearing the state file here: if the restore failed, the next start
        // should try again rather than forget the user's wallpaper entirely.
        tracing::warn!("failed to restore wallpaper: {err}");
        return;
    }

    clear();
}

/// Restores a wallpaper stranded by a previous supervisor that died mid-episode.
pub fn recover_stranded() {
    let Some(path) = state_path() else { return };

    let Ok(contents) = fs::read(&path) else {
        // No file is the normal case: the last run shut down cleanly.
        return;
    };

    match serde_json::from_slice::<Snapshot>(&contents) {
        Ok(snapshot) => {
            tracing::info!("restoring a wallpaper left behind by a previous run");
            restore(&snapshot);
        }
        Err(err) => {
            tracing::warn!("discarding an unreadable wallpaper snapshot: {err}");
            clear();
        }
    }
}

fn clear() {
    if let Some(path) = state_path()
        && let Err(err) = fs::remove_file(&path)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("failed to clear the persisted wallpaper snapshot: {err}");
    }
}
