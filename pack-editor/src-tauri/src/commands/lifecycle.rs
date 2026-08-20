//! Opening, importing, saving, closing and discarding a pack, and undo/redo over its edits.

use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use shared::encode::HardwareEncoder;
use tauri::{AppHandle, Emitter, State};

use super::recents::{forget_draft, remember_draft, remember_pack};
use super::MetadataDto;
use crate::pack::MediaPack;
use crate::{history, import, AppState};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackInfo {
    pub id: String,
    pub name: String,
    pub has_unsaved_changes: bool,
    pub has_destination: bool,
}

/// A picked-path-plus-converted-output result the frontend needs to both open the (still
/// filling-in) pack and surface what the converter couldn't carry over.
#[derive(Serialize)]
pub struct ImportResult {
    pub info: PackInfo,
    pub warnings: Vec<converter::Warning>,
}

/// Replaces any filesystem-hostile character (Windows' reserved set is the strictest, so it's
/// used uniformly) with `_`, for turning a converted pack's freeform `name` into a suggested
/// `.lwpack` file name.
pub fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if r#"<>:"/\|?*"#.contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Imported Pack".to_string()
    } else {
        trimmed.to_string()
    }
}

#[tauri::command]
pub async fn new_pack(state: State<'_, AppState>) -> Result<PackInfo, String> {
    let name = "Untitled Pack";
    let data_dir = dirs::data_dir().ok_or("Couldn't find data dir")?;
    let pack = MediaPack::new_unsaved(&data_dir, name)
        .await
        .map_err(|e| e.to_string())?;
    let info = PackInfo {
        id: pack.id().to_string(),
        name: pack.name(),
        has_unsaved_changes: true,
        has_destination: false,
    };
    remember_draft(pack.id(), info.name.clone())?;
    *state.pack.lock().await = Some(Arc::new(pack));
    Ok(info)
}

#[tauri::command]
pub async fn open_pack_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<PackInfo>, String> {
    use tauri_plugin_dialog::DialogExt;
    let app_c = app.clone();
    let file = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Open pack")
            .add_filter("Lewdware Pack", &["lwpack"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(path) = file else { return Ok(None) };
    let path: PathBuf = path.into_path().map_err(|e| e.to_string())?;
    app.emit("picker:pack-selected", ())
        .map_err(|e| e.to_string())?;

    let data_dir = dirs::data_dir().ok_or("Couldn't find data dir")?;
    let pack = MediaPack::open(path.clone(), &data_dir)
        .await
        .map_err(|e| e.to_string())?;
    let has_unsaved_changes = !pack.is_saved().await;
    let info = PackInfo {
        id: pack.id().to_string(),
        name: pack.name(),
        has_unsaved_changes,
        has_destination: true,
    };
    remember_pack(&path, info.name.clone())?;
    *state.pack.lock().await = Some(Arc::new(pack));
    Ok(Some(info))
}

#[tauri::command]
pub async fn open_recent_pack(
    state: State<'_, AppState>,
    path: Option<String>,
    draft_id: Option<String>,
) -> Result<PackInfo, String> {
    let data_dir = dirs::data_dir().ok_or("Couldn't find data dir")?;
    let pack = if let Some(path) = path.as_deref() {
        MediaPack::open(PathBuf::from(path), &data_dir)
            .await
            .map_err(|e| e.to_string())?
    } else {
        let id = uuid::Uuid::parse_str(draft_id.as_deref().ok_or("Missing draft id")?)
            .map_err(|e| e.to_string())?;
        MediaPack::recover_unsaved(&data_dir, id)
            .await
            .map_err(|e| e.to_string())?
    };
    let info = PackInfo {
        id: pack.id().to_string(),
        name: pack.name(),
        has_unsaved_changes: !pack.is_saved().await,
        has_destination: pack.path().is_some(),
    };
    if let Some(path) = path {
        remember_pack(std::path::Path::new(&path), info.name.clone())?;
    } else {
        remember_draft(pack.id(), info.name.clone())?;
    }
    *state.pack.lock().await = Some(Arc::new(pack));
    Ok(info)
}

#[tauri::command]
pub async fn import_edgeware_pack_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<ImportResult>, String> {
    use tauri_plugin_dialog::DialogExt;

    let app_c = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Select an Edgeware pack (.zip)")
            .add_filter("Zip Archive", &["zip"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(picked) = picked else {
        return Ok(None);
    };
    let source_path: PathBuf = picked.into_path().map_err(|e| e.to_string())?;
    app.emit("picker:edgeware-selected", ())
        .map_err(|e| e.to_string())?;

    // Converting is I/O-only (JSON + directory listings, never media bytes) and runs
    // synchronously here -- before any pack is created or a destination is even picked -- so an
    // unreadable source (e.g. a corrupt zip) fails the command immediately instead of leaving a
    // half-created empty pack behind.
    let (source, output) = tokio::task::spawn_blocking(move || import::load(&source_path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    let shared::pack::Metadata { name, .. } = &output.metadata;
    let data_dir = dirs::data_dir().ok_or("Couldn't find data dir")?;
    let pack = MediaPack::new_unsaved(&data_dir, name)
        .await
        .map_err(|e| e.to_string())?;

    let converter::ConversionOutput {
        metadata,
        behaviour,
        media,
        media_references,
        icon: _,
        warnings,
    } = output;

    // The wallpaper/splash slots are the one part the converter can't fill: they reference media
    // by id, and no file has one until it has been imported (a name collision suffixes it, a
    // duplicate or a failed encode means it never arrives at all). The converter reports which
    // file each slot is waiting on instead, and `run_import` fills them in as those files really
    // land -- so the pack never carries a reference to something it doesn't have.

    // Everything else is written synchronously, before the pack is even stored in app state or
    // the media pipeline spawned -- it's keyed by tag, not by any specific media file's id, so it
    // has no dependency on encoding having finished. This is what lets the pack editor's
    // Content/Experience tabs show real converted data immediately, rather than only once a
    // (potentially large) media pipeline finishes streaming in.
    pack.replace_behaviour(behaviour, "Import Edgeware pack".to_string())
        .await
        .map_err(|e| e.to_string())?;
    pack.set_metadata(&metadata)
        .await
        .map_err(|e| e.to_string())?;
    pack.save_metadata().await.map_err(|e| e.to_string())?;

    let info = PackInfo {
        id: pack.id().to_string(),
        name: pack.name(),
        has_unsaved_changes: !pack.is_saved().await,
        has_destination: false,
    };
    remember_draft(pack.id(), info.name.clone())?;
    *state.pack.lock().await = Some(Arc::new(pack));

    let pack_state = state.pack.clone();
    let encoder = state
        .hardware_encoder
        .get()
        .cloned()
        .unwrap_or(HardwareEncoder::SoftwareFallback);
    let upload_lock = state.upload_lock.clone();
    state.cancel_uploads.send_replace(false);
    let cancel = state.cancel_uploads.subscribe();

    tauri::async_runtime::spawn(import::run_import(
        pack_state,
        Arc::from(source),
        media,
        media_references,
        app,
        encoder,
        upload_lock,
        cancel,
    ));

    Ok(Some(ImportResult { info, warnings }))
}

#[tauri::command]
pub async fn save_pack(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<PackInfo>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (needs_destination, suggested_name) = {
        let lock = state.pack.lock().await;
        let pack = lock.as_ref().ok_or("No pack open")?;
        (pack.path().is_none(), sanitize_file_name(&pack.name()))
    };

    if needs_destination {
        let app_c = app.clone();
        let file = tokio::task::spawn_blocking(move || {
            app_c
                .dialog()
                .file()
                .set_title("Save pack")
                .add_filter("Lewdware Pack", &["lwpack"])
                .set_file_name(format!("{suggested_name}.lwpack"))
                .blocking_save_file()
        })
        .await
        .map_err(|e| e.to_string())?;
        let Some(file) = file else { return Ok(None) };
        let path: PathBuf = file.into_path().map_err(|e| e.to_string())?;
        app.emit("picker:save-destination-selected", ())
            .map_err(|e| e.to_string())?;
        let _uploads = state.upload_lock.write().await;
        let pack = state
            .pack
            .lock()
            .await
            .as_ref()
            .cloned()
            .ok_or("No pack open")?;
        let draft_id = pack.id();
        let app_cb = app.clone();
        if let Some(new_pack) = pack
            .save_as(&path, move |saved, total| {
                let _ = app_cb.emit(
                    "save:progress",
                    serde_json::json!({ "saved": saved, "total": total }),
                );
            })
            .await
            .map_err(|e| e.to_string())?
        {
            let info = PackInfo {
                id: new_pack.id().to_string(),
                name: new_pack.name(),
                has_unsaved_changes: false,
                has_destination: true,
            };
            forget_draft(draft_id)?;
            remember_pack(&path, info.name.clone())?;
            let mut lock = state.pack.lock().await;
            if lock
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(current, &pack))
            {
                return Err("The pack was closed while saving".into());
            }
            *lock = Some(Arc::new(new_pack));
            let _ = app.emit(
                "save:done",
                serde_json::json!({ "has_unsaved_changes": false }),
            );
            return Ok(Some(info));
        }
        return Ok(None);
    }

    let pack = state.pack.lock().await.as_ref().cloned();
    if let Some(pack) = pack {
        let progress_app = app.clone();
        let fallback_app = app.clone();
        pack.save_with_in_place_notification(
            move |saved, t| {
                let _ = progress_app.emit(
                    "save:progress",
                    serde_json::json!({ "saved": saved, "total": t }),
                );
            },
            move || {
                let _ = fallback_app.emit("save:in-place", ());
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        if let Some(path) = pack.path() {
            remember_pack(path, pack.name())?;
        }
        let has_unsaved_changes = !pack.is_saved().await;
        let _ = app.emit(
            "save:done",
            serde_json::json!({ "has_unsaved_changes": has_unsaved_changes }),
        );
        return Ok(Some(PackInfo {
            id: pack.id().to_string(),
            name: pack.name(),
            has_unsaved_changes,
            has_destination: true,
        }));
    }
    Ok(None)
}

#[tauri::command]
pub async fn save_pack_as_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<PackInfo>, String> {
    use tauri_plugin_dialog::DialogExt;
    let app_c = app.clone();
    let file = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Save pack as")
            .add_filter("Lewdware Pack", &["lwpack"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(path) = file else { return Ok(None) };
    let path: PathBuf = path.into_path().map_err(|e| e.to_string())?;
    app.emit("picker:save-as-destination-selected", ())
        .map_err(|e| e.to_string())?;

    let _uploads = state.upload_lock.write().await;
    let pack = state.pack.lock().await.as_ref().cloned();
    if let Some(pack) = pack {
        let was_untitled = pack.path().is_none();
        let draft_id = was_untitled.then(|| pack.id());
        let app_cb = app.clone();
        let new_pack = pack
            .save_as(&path, move |saved, t| {
                let _ = app_cb.emit(
                    "save:progress",
                    serde_json::json!({ "saved": saved, "total": t }),
                );
            })
            .await
            .map_err(|e| e.to_string())?;

        if let Some(new_pack) = new_pack {
            let info = PackInfo {
                id: new_pack.id().to_string(),
                name: new_pack.name(),
                has_unsaved_changes: false,
                has_destination: true,
            };
            if let Some(id) = draft_id {
                forget_draft(id)?;
            }
            remember_pack(&path, info.name.clone())?;
            let mut lock = state.pack.lock().await;
            if lock
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(current, &pack))
            {
                return Err("The pack was closed while saving".into());
            }
            *lock = Some(Arc::new(new_pack));
            drop(lock);
            if !was_untitled {
                pack.close().await;
            }
            let _ = app.emit(
                "save:done",
                serde_json::json!({ "has_unsaved_changes": false }),
            );
            return Ok(Some(info));
        }
    }
    Ok(None)
}

#[tauri::command]
pub async fn discard_changes(state: State<'_, AppState>) -> Result<MetadataDto, String> {
    state.cancel_uploads.send_replace(true);
    let _uploads = state.upload_lock.write().await;
    state
        .with_pack(async |pack| Ok(pack.discard_changes().await?.into()))
        .await
}

#[tauri::command]
pub async fn close_pack(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_uploads.send_replace(true);
    let _uploads = state.upload_lock.write().await;
    let pack = state.pack.lock().await.take();
    let draft_id = pack.as_ref().filter(|p| p.is_untitled()).map(|p| p.id());
    let forget_draft_after_close = if let Some(pack) = &pack {
        pack.is_saved().await
    } else {
        false
    };
    if let Some(pack) = &pack {
        pack.close().await;
    }
    drop(pack);
    if forget_draft_after_close {
        if let Some(id) = draft_id {
            let _ = forget_draft(id);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn discard_pack(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_uploads.send_replace(true);
    let _uploads = state.upload_lock.write().await;
    let pack = state.pack.lock().await.take();
    let draft_id = pack
        .as_ref()
        .filter(|pack| pack.is_untitled())
        .map(|pack| pack.id());
    if let Some(pack) = &pack {
        pack.close_and_discard().await;
    }
    drop(pack);
    if let Some(id) = draft_id {
        let _ = forget_draft(id);
    }
    Ok(())
}

#[tauri::command]
pub async fn confirm_close(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    close_pack(state).await?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub async fn is_pack_saved(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .with_pack_or(true, async |pack| Ok(pack.is_saved().await))
        .await
}

#[tauri::command]
pub async fn get_history_status(
    state: State<'_, AppState>,
) -> Result<history::Status, String> {
    let none_open = history::Status {
        can_undo: false,
        can_redo: false,
        undo_label: None,
        redo_label: None,
        at_saved_state: true,
    };
    state
        .with_pack_or(none_open, async |pack| pack.history_status().await)
        .await
}

#[tauri::command]
pub async fn undo(state: State<'_, AppState>) -> Result<history::Status, String> {
    state.with_pack(async |pack| pack.undo().await).await
}

#[tauri::command]
pub async fn redo(state: State<'_, AppState>) -> Result<history::Status, String> {
    state.with_pack(async |pack| pack.redo().await).await
}
