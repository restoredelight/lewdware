//! What the wallpaper is put back to when a session ends. Ported from
//! `config/src-tauri/src/commands/wallpaper.rs`.

use std::path::{Path, PathBuf};

const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];

/// Whether this desktop can report its wallpaper, so the original can be restored. Runs a real
/// snapshot (a `dbus-send` on KDE, a `gsettings` read on GNOME), so call it once per page view.
pub fn can_restore_original() -> bool {
    shared::wallpaper::snapshot(None).is_restorable()
}

/// The bundled near-black placeholder, written to disk. Offered as the starting point so the
/// restore image is never unset once the user opts in.
pub fn default_restore_image() -> anyhow::Result<PathBuf> {
    shared::wallpaper::default_restore_image_path()
}

/// Asks for an image. `None` when the dialog is dismissed.
pub async fn pick_image() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Choose the wallpaper to restore")
        .add_filter("Image", &IMAGE_EXTENSIONS)
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}

/// Copies `picked` into app data and returns the copy. Referencing it where it sits would fail
/// exactly when it is needed if the user later moved or deleted it, stranding the wallpaper the
/// setting exists to protect.
pub fn adopt_image(picked: &Path) -> anyhow::Result<PathBuf> {
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

    // Otherwise a change of extension would leave the previous copy behind.
    for stale in IMAGE_EXTENSIONS {
        let stale = dir.join(format!("restore-wallpaper.{stale}"));
        if stale != destination {
            let _ = std::fs::remove_file(stale);
        }
    }

    std::fs::copy(picked, &destination)?;
    Ok(destination)
}
