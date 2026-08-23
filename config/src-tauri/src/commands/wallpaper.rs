use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Whether this desktop can have its wallpaper put back after a pack changes it.
///
/// Read once when the Permissions page mounts. It runs a real snapshot (a `dbus-send` on KDE, a
/// `gsettings` read on GNOME), which is why it isn't polled -- the answer only changes if the user
/// switches desktop session, at which point the page is being re-opened anyway.
#[tauri::command]
pub async fn wallpaper_support() -> Result<WallpaperSupportDto, String> {
    let snapshot = tokio::task::spawn_blocking(|| shared::wallpaper::snapshot(None))
        .await
        .map_err(|e| e.to_string())?;

    Ok(WallpaperSupportDto {
        can_restore_original: snapshot.is_restorable(),
    })
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WallpaperSupportDto {
    /// `false` means the user has to nominate an image to restore to, or wallpaper changes stay
    /// off. See `shared::wallpaper::Snapshot::is_restorable`.
    pub can_restore_original: bool,
}

/// The restore image, as a `data:` URL for `<img src>`.
///
/// Inlined rather than served over Tauri's asset protocol so there is no protocol scope or CSP to
/// configure -- this is one small image on one settings page, shown once.
#[tauri::command]
pub async fn wallpaper_restore_preview(path: String) -> Result<Option<String>, String> {
    let preview = tokio::task::spawn_blocking(move || -> Option<String> {
        use base64::Engine;

        let bytes = std::fs::read(&path).ok()?;
        // The file is whatever the user picked, so guess the type from its extension rather than
        // assuming PNG.
        let mime = match std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            Some("bmp") => "image/bmp",
            _ => "image/png",
        };

        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Some(format!("data:{mime};base64,{encoded}"))
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(preview)
}

/// Prompts for an image and adopts it as the restore target.
///
/// The file is copied into app data rather than referenced where it sits: a restore image that the
/// user later deletes or moves would fail exactly when it is needed, stranding the wallpaper the
/// setting exists to protect.
#[tauri::command]
pub async fn pick_restore_image(app_handle: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app_handle
        .dialog()
        .file()
        .add_filter("Image", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());

    let Some(picked) = picked else {
        return Ok(None);
    };
    app_handle
        .emit("picker:restore-image-selected", ())
        .map_err(|e| e.to_string())?;

    let adopted = tokio::task::spawn_blocking(move || -> anyhow::Result<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("could not locate the local data directory"))?
            .join("lewdware");
        std::fs::create_dir_all(&dir)?;

        let extension = picked
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let destination = dir.join(format!("restore-wallpaper.{extension}"));

        // Copying onto the previous choice would leave a stale file behind whenever the extension
        // changes, and the config would still point at the old one.
        for stale in ["png", "jpg", "jpeg", "webp", "bmp", "gif"] {
            let stale = dir.join(format!("restore-wallpaper.{stale}"));
            if stale != destination {
                let _ = std::fs::remove_file(stale);
            }
        }

        std::fs::copy(&picked, &destination)?;
        Ok(destination)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    Ok(Some(adopted.to_string_lossy().into_owned()))
}

/// The bundled near-black placeholder, materialised on disk.
///
/// Offered as the starting point so the restore image is never left unset once the user opts in.
#[tauri::command]
pub async fn default_restore_image() -> Result<String, String> {
    tokio::task::spawn_blocking(shared::wallpaper::default_restore_image_path)
        .await
        .map_err(|e| e.to_string())?
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}
