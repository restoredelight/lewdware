mod editor_db;
mod encode;
mod history;
mod import;
mod media_server;
mod pack;
mod thumbnail;

use std::{
    io::Cursor,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use pack::{ArtistSummary, MediaFile, MediaPack, TagSummary};
use serde::{Deserialize, Serialize};
use shared::behaviour::v3::Behaviour;
use shared::mode;

// ─── Update check ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct UpdateManifest {
    version: String,
    download_page: String,
}

fn parse_version(v: &str) -> (u32, u32, u32) {
    let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[tauri::command]
async fn check_for_update() -> Result<Option<String>, String> {
    let current = env!("CARGO_PKG_VERSION");
    let resp = reqwest::get("https://lewdware.net/download/pack-editor-latest.json")
        .await
        .map_err(|e| e.to_string())?;
    let manifest: UpdateManifest = resp.json().await.map_err(|e| e.to_string())?;
    if parse_version(&manifest.version) > parse_version(current) {
        Ok(Some(manifest.download_page))
    } else {
        Ok(None)
    }
}
use shared::read_pack::{Metadata, RecommendedMode};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{watch, Mutex, RwLock};

use shared::encode::HardwareEncoder;

pub type PackState = Arc<Mutex<Option<Arc<MediaPack>>>>;

pub struct AppState {
    pub pack: PackState,
    pub media_port: OnceLock<u16>,
    pub hardware_encoder: OnceLock<HardwareEncoder>,
    pub upload_lock: Arc<RwLock<()>>,
    pub cancel_uploads: watch::Sender<bool>,
}

impl AppState {
    fn new() -> Self {
        Self {
            pack: Arc::new(Mutex::new(None)),
            media_port: OnceLock::new(),
            hardware_encoder: OnceLock::new(),
            upload_lock: Arc::new(RwLock::new(())),
            cancel_uploads: watch::channel(false).0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackInfo {
    pub name: String,
    pub has_unsaved_changes: bool,
    pub has_destination: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecentPack {
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub draft_id: Option<String>,
    pub last_opened: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct EmbeddedMode {
    pub id: u64,
    pub stable_id: String,
    pub name: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub option_count: usize,
    pub size: u64,
}

fn embedded_mode(id: u64, bytes: &[u8]) -> Result<EmbeddedMode, String> {
    let (header, metadata) =
        mode::read_mode_metadata(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let option_count = metadata.all_options().len();
    Ok(EmbeddedMode {
        id,
        stable_id: header.id.to_string(),
        name: metadata.name,
        author: metadata.author,
        version: metadata.version,
        option_count,
        size: bytes.len() as u64,
    })
}

fn editor_data_dir() -> Result<PathBuf, String> {
    dirs::data_dir()
        .map(|p| p.join("Lewdware Pack Editor"))
        .ok_or_else(|| "Couldn't find data dir".to_string())
}

fn recent_file() -> Result<PathBuf, String> {
    Ok(editor_data_dir()?.join("recent-packs.json"))
}

fn load_recents() -> Vec<RecentPack> {
    recent_file()
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_recents(recents: &[RecentPack]) -> Result<(), String> {
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

fn remember_pack(path: &std::path::Path, name: String) -> Result<(), String> {
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

fn remember_draft(id: uuid::Uuid, name: String) -> Result<(), String> {
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

fn forget_draft(id: uuid::Uuid) -> Result<(), String> {
    let id = id.to_string();
    let mut recents = load_recents();
    recents.retain(|r| r.draft_id.as_deref() != Some(&id));
    write_recents(&recents)
}

#[tauri::command]
fn get_recent_packs() -> Vec<RecentPack> {
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
fn remove_recent_pack(path: Option<String>, draft_id: Option<String>) -> Result<(), String> {
    let mut recents = load_recents();
    recents.retain(|r| r.path != path || r.draft_id != draft_id);
    if let Some(id) = draft_id {
        let _ = std::fs::remove_dir_all(editor_data_dir()?.join(id));
    }
    write_recents(&recents)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MetadataDto {
    pub name: String,
    pub creator: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    // Not yet editable from the pack editor UI (no Modes tab) -- carried through opaquely so
    // saving an unrelated metadata edit (e.g. renaming the pack) doesn't clobber a
    // recommended_mode set by, e.g., the converter. `#[serde(default)]` so the frontend's
    // pre-Modes-tab payloads (which don't send this key) still deserialize.
    #[serde(default)]
    pub recommended_mode: Option<RecommendedMode>,
}

impl From<Metadata> for MetadataDto {
    fn from(m: Metadata) -> Self {
        Self {
            name: m.name,
            creator: m.creator,
            description: m.description,
            version: m.version,
            recommended_mode: m.recommended_mode,
        }
    }
}

impl From<MetadataDto> for Metadata {
    fn from(d: MetadataDto) -> Self {
        Self {
            name: d.name,
            creator: d.creator,
            description: d.description,
            version: d.version,
            recommended_mode: d.recommended_mode,
        }
    }
}

// ── Pack lifecycle ───────────────────────────────────────────────────────────

#[tauri::command]
async fn new_pack(state: State<'_, AppState>) -> Result<PackInfo, String> {
    let name = "Untitled Pack";
    let data_dir = dirs::data_dir().ok_or("Couldn't find data dir")?;
    let pack = MediaPack::new_unsaved(&data_dir, name)
        .await
        .map_err(|e| e.to_string())?;
    let info = PackInfo {
        name: pack.name(),
        has_unsaved_changes: true,
        has_destination: false,
    };
    remember_draft(pack.id(), info.name.clone())?;
    *state.pack.lock().await = Some(Arc::new(pack));
    Ok(info)
}

#[tauri::command]
async fn open_pack_dialog(
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

    let data_dir = dirs::data_dir().ok_or("Couldn't find data dir")?;
    let pack = MediaPack::open(path.clone(), &data_dir)
        .await
        .map_err(|e| e.to_string())?;
    let has_unsaved_changes = !pack.is_saved().await;
    let info = PackInfo {
        name: pack.name(),
        has_unsaved_changes,
        has_destination: true,
    };
    remember_pack(&path, info.name.clone())?;
    *state.pack.lock().await = Some(Arc::new(pack));
    Ok(Some(info))
}

#[tauri::command]
async fn open_recent_pack(
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

/// A picked-path-plus-converted-output result the frontend needs to both open the (still
/// filling-in) pack and surface what the converter couldn't carry over.
#[derive(Serialize)]
struct ImportResult {
    info: PackInfo,
    warnings: Vec<converter::Warning>,
}

/// Replaces any filesystem-hostile character (Windows' reserved set is the strictest, so it's
/// used uniformly) with `_`, for turning a converted pack's freeform `name` into a suggested
/// `.lwpack` file name.
fn sanitize_file_name(name: &str) -> String {
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
async fn import_edgeware_pack_dialog(
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

    // Converting is I/O-only (JSON + directory listings, never media bytes) and runs
    // synchronously here -- before any pack is created or a destination is even picked -- so an
    // unreadable source (e.g. a corrupt zip) fails the command immediately instead of leaving a
    // half-created empty pack behind.
    let (source, output) = tokio::task::spawn_blocking(move || import::load(&source_path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    let shared::read_pack::Metadata { name, .. } = &output.metadata;
    let data_dir = dirs::data_dir().ok_or("Couldn't find data dir")?;
    let pack = MediaPack::new_unsaved(&data_dir, name)
        .await
        .map_err(|e| e.to_string())?;

    let converter::ConversionOutput {
        metadata,
        behaviour,
        media,
        icon: _,
        warnings,
    } = output;

    // Written synchronously, before the pack is even stored in app state or the media pipeline
    // spawned -- behaviour.json/metadata are keyed by tag, not by any specific media file's id, so
    // they have no dependency on encoding having finished. This is what lets the pack editor's
    // Content/Experience tabs show real converted data immediately, rather than only once a
    // (potentially large) media pipeline finishes streaming in.
    pack.set_pack_data(
        "behaviour",
        shared::behaviour::v3::Behaviour::from(behaviour)
            .to_json_bytes()
            .map_err(|e| e.to_string())?,
    )
    .await
    .map_err(|e| e.to_string())?;
    pack.set_metadata(&metadata)
        .await
        .map_err(|e| e.to_string())?;
    pack.save_metadata().await.map_err(|e| e.to_string())?;

    let info = PackInfo {
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
        app,
        encoder,
        upload_lock,
        cancel,
    ));

    Ok(Some(ImportResult { info, warnings }))
}

#[tauri::command]
async fn save_pack(state: State<'_, AppState>, app: AppHandle) -> Result<Option<PackInfo>, String> {
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
            name: pack.name(),
            has_unsaved_changes,
            has_destination: true,
        }));
    }
    Ok(None)
}

#[tauri::command]
async fn save_pack_as_dialog(
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
async fn discard_changes(state: State<'_, AppState>) -> Result<MetadataDto, String> {
    state.cancel_uploads.send_replace(true);
    let _uploads = state.upload_lock.write().await;
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        let metadata = pack.discard_changes().await.map_err(|e| e.to_string())?;
        Ok(metadata.into())
    } else {
        Err("No pack open".to_string())
    }
}

#[tauri::command]
async fn close_pack(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_uploads.send_replace(true);
    let _uploads = state.upload_lock.write().await;
    let pack = state.pack.lock().await.take();
    let draft_id = pack.as_ref().filter(|p| p.is_untitled()).map(|p| p.id());
    if let Some(pack) = &pack {
        pack.close().await;
    }
    drop(pack);
    if let Some(id) = draft_id {
        let _ = forget_draft(id);
    }
    Ok(())
}

#[tauri::command]
async fn confirm_close(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    close_pack(state).await?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn is_pack_saved(state: State<'_, AppState>) -> Result<bool, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => Ok(pack.is_saved().await),
        None => Ok(true),
    }
}

#[tauri::command]
async fn get_history_status(state: State<'_, AppState>) -> Result<history::Status, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => pack
            .history_status()
            .await
            .map_err(|error| error.to_string()),
        None => Ok(history::Status {
            can_undo: false,
            can_redo: false,
            undo_label: None,
            redo_label: None,
            at_saved_state: true,
        }),
    }
}

#[tauri::command]
async fn undo(state: State<'_, AppState>) -> Result<history::Status, String> {
    let lock = state.pack.lock().await;
    let pack = lock.as_ref().ok_or("No pack open")?;
    pack.undo().await.map_err(|error| error.to_string())
}

#[tauri::command]
async fn redo(state: State<'_, AppState>) -> Result<history::Status, String> {
    let lock = state.pack.lock().await;
    let pack = lock.as_ref().ok_or("No pack open")?;
    pack.redo().await.map_err(|error| error.to_string())
}

// ── Files ────────────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_files(state: State<'_, AppState>) -> Result<Vec<MediaFile>, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => pack.get_files().await.map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn remove_files(state: State<'_, AppState>, ids: Vec<u64>) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.remove_files(ids).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Embedded modes ──────────────────────────────────────────────────────────

#[tauri::command]
async fn get_modes(state: State<'_, AppState>) -> Result<Vec<EmbeddedMode>, String> {
    let lock = state.pack.lock().await;
    let Some(pack) = lock.as_ref() else {
        return Ok(vec![]);
    };
    pack.get_modes()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|(id, bytes)| embedded_mode(id, &bytes))
        .collect()
}

#[tauri::command]
async fn add_mode_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<EmbeddedMode>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Add custom mode")
            .add_filter("Lewdware Mode", &["lwmode"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path: PathBuf = picked.into_path().map_err(|error| error.to_string())?;
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| error.to_string())?;
    let candidate = embedded_mode(0, &bytes)?;

    let lock = state.pack.lock().await;
    let Some(pack) = lock.as_ref() else {
        return Err("No pack is open".into());
    };
    for (id, existing) in pack.get_modes().await.map_err(|error| error.to_string())? {
        let current = embedded_mode(id, &existing)?;
        if current.stable_id == candidate.stable_id {
            return Err(format!(
                "“{}” is already included in this pack",
                current.name
            ));
        }
    }
    let id = pack
        .add_mode(bytes)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Some(EmbeddedMode { id, ..candidate }))
}

#[tauri::command]
async fn remove_mode(state: State<'_, AppState>, id: u64) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.remove_mode(id)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn set_file_title(state: State<'_, AppState>, id: u64, name: String) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.set_title(id, name).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn set_file_source_url(
    state: State<'_, AppState>,
    id: u64,
    url: Option<String>,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.set_source_url(id, url)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Tags ─────────────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_all_tags(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => pack.get_all_tags().await.map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn get_file_tags(state: State<'_, AppState>, id: u64) -> Result<Vec<String>, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => pack.get_tags(id).await.map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn add_tag_to_file(state: State<'_, AppState>, id: u64, tag: String) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.add_tag(id, tag).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn remove_tag_from_file(
    state: State<'_, AppState>,
    id: u64,
    tag: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.remove_tag(id, tag).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn create_and_add_tag(
    state: State<'_, AppState>,
    id: u64,
    tag: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.create_and_add_tag(id, tag)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn get_tag_summaries(state: State<'_, AppState>) -> Result<Vec<TagSummary>, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => pack.get_tag_summaries().await.map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn rename_tag(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<Behaviour, String> {
    let lock = state.pack.lock().await;
    let pack = lock.as_ref().ok_or("No pack open")?;
    pack.rename_tag(from, to).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn merge_tag(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<Behaviour, String> {
    let lock = state.pack.lock().await;
    let pack = lock.as_ref().ok_or("No pack open")?;
    pack.merge_tag(from, to).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_tag(state: State<'_, AppState>, tag: String) -> Result<Behaviour, String> {
    let lock = state.pack.lock().await;
    let pack = lock.as_ref().ok_or("No pack open")?;
    pack.delete_tag(tag).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_tag_to_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    tag: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.add_tag_to_files(ids, tag)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn remove_tag_from_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    tag: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.remove_tag_from_files(ids, tag)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Artists ──────────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_all_artists(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => pack.get_all_artists().await.map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn get_file_artists(state: State<'_, AppState>, id: u64) -> Result<Vec<String>, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => pack.get_artists(id).await.map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn add_artist_to_file(
    state: State<'_, AppState>,
    id: u64,
    artist: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.add_artist(id, artist)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn remove_artist_from_file(
    state: State<'_, AppState>,
    id: u64,
    artist: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.remove_artist(id, artist)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn create_and_add_artist(
    state: State<'_, AppState>,
    id: u64,
    artist: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.create_and_add_artist(id, artist)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn get_artist_summaries(state: State<'_, AppState>) -> Result<Vec<ArtistSummary>, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => pack.get_artist_summaries().await.map_err(|e| e.to_string()),
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn rename_artist(state: State<'_, AppState>, from: String, to: String) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.rename_artist(from, to)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn merge_artist(state: State<'_, AppState>, from: String, to: String) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.merge_artist(from, to)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn delete_artist(state: State<'_, AppState>, artist: String) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.delete_artist(artist)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn add_artist_to_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    artist: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.add_artist_to_files(ids, artist)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn remove_artist_from_files(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    artist: String,
) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.remove_artist_from_files(ids, artist)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Metadata ─────────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_pack_metadata(state: State<'_, AppState>) -> Result<MetadataDto, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => Ok(pack.metadata().into()),
        None => Err("No pack open".to_string()),
    }
}

#[tauri::command]
async fn set_pack_metadata(state: State<'_, AppState>, dto: MetadataDto) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.set_metadata(&dto.into())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn save_pack_metadata(state: State<'_, AppState>) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.save_metadata().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn mark_pack_unsaved(state: State<'_, AppState>) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.mark_unsaved().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Behaviour (Content/Experience tabs) ──────────────────────────────────────

#[tauri::command]
async fn get_behaviour(state: State<'_, AppState>) -> Result<Behaviour, String> {
    let lock = state.pack.lock().await;
    match lock.as_ref() {
        Some(pack) => {
            let blob = pack
                .get_pack_data("behaviour")
                .await
                .map_err(|e| e.to_string())?;
            match blob {
                Some(bytes) => Behaviour::from_json_bytes(&bytes).map_err(|e| e.to_string()),
                None => Ok(Behaviour::new()),
            }
        }
        None => Err("No pack open".to_string()),
    }
}

#[tauri::command]
async fn set_behaviour(state: State<'_, AppState>, behaviour: Behaviour) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        let bytes = behaviour.to_json_bytes().map_err(|e| e.to_string())?;
        pack.set_pack_data("behaviour", bytes)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── Upload ───────────────────────────────────────────────────────────────────

#[tauri::command]
async fn add_files_dialog(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    use tauri_plugin_dialog::DialogExt;
    let app_c = app.clone();
    let files = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Select files")
            .blocking_pick_files()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(handles) = files else { return Ok(()) };
    let paths: Vec<PathBuf> = handles
        .into_iter()
        .filter_map(|h| h.into_path().ok())
        .collect();

    if paths.is_empty() {
        return Ok(());
    }

    let pack_state = state.pack.clone();
    let encoder = state
        .hardware_encoder
        .get()
        .cloned()
        .unwrap_or(HardwareEncoder::SoftwareFallback);
    let upload_lock = state.upload_lock.clone();
    state.cancel_uploads.send_replace(false);
    let cancel = state.cancel_uploads.subscribe();
    tauri::async_runtime::spawn(encode::process_files(
        pack_state,
        paths,
        app,
        encoder,
        upload_lock,
        cancel,
    ));
    Ok(())
}

#[tauri::command]
async fn add_folder_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
    recursive: bool,
) -> Result<(), String> {
    use tauri_plugin_dialog::DialogExt;
    let app_c = app.clone();
    let folder = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Select folder")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(folder) = folder else { return Ok(()) };
    let folder: PathBuf = folder.into_path().map_err(|e| e.to_string())?;

    let paths = tokio::task::spawn_blocking(move || encode::explore_folder(&folder, recursive))
        .await
        .map_err(|e| e.to_string())?;

    if paths.is_empty() {
        return Ok(());
    }

    let pack_state = state.pack.clone();
    let encoder = state
        .hardware_encoder
        .get()
        .cloned()
        .unwrap_or(HardwareEncoder::SoftwareFallback);
    let upload_lock = state.upload_lock.clone();
    state.cancel_uploads.send_replace(false);
    let cancel = state.cancel_uploads.subscribe();
    tauri::async_runtime::spawn(encode::process_files(
        pack_state,
        paths,
        app,
        encoder,
        upload_lock,
        cancel,
    ));
    Ok(())
}

#[tauri::command]
async fn add_paths(
    state: State<'_, AppState>,
    app: AppHandle,
    paths: Vec<PathBuf>,
) -> Result<(), String> {
    let paths = tokio::task::spawn_blocking(move || {
        let mut result = Vec::new();
        for path in paths {
            if path.is_dir() {
                result.extend(encode::explore_folder(&path, false));
            } else if !encode::is_junk_path(&path) && encode::is_media_path(&path).unwrap_or(false)
            {
                result.push(path);
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?;

    if paths.is_empty() {
        return Ok(());
    }

    let pack_state = state.pack.clone();
    let encoder = state
        .hardware_encoder
        .get()
        .cloned()
        .unwrap_or(HardwareEncoder::SoftwareFallback);
    let upload_lock = state.upload_lock.clone();
    state.cancel_uploads.send_replace(false);
    let cancel = state.cancel_uploads.subscribe();
    tauri::async_runtime::spawn(encode::process_files(
        pack_state,
        paths,
        app,
        encoder,
        upload_lock,
        cancel,
    ));
    Ok(())
}

#[tauri::command]
async fn cancel_upload(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_uploads.send_replace(true);
    Ok(())
}

// ── Media server port ────────────────────────────────────────────────────────

#[tauri::command]
fn get_media_port(state: State<'_, AppState>) -> u16 {
    *state.media_port.get().unwrap_or(&0)
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _log_guard = shared::logging::init("pack-editor");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
                window.set_icon(icon)?;
            }

            let state = app.state::<AppState>();

            if let Ok(resource_dir) = app.path().resource_dir() {
                let ffmpeg_name = if cfg!(target_os = "windows") {
                    "lewdware-ffmpeg.exe"
                } else {
                    "lewdware-ffmpeg"
                };
                let ffprobe_name = if cfg!(target_os = "windows") {
                    "lewdware-ffprobe.exe"
                } else {
                    "lewdware-ffprobe"
                };
                let ffmpeg = resource_dir.join("binaries").join(ffmpeg_name);
                let ffprobe = resource_dir.join("binaries").join(ffprobe_name);
                if ffmpeg.exists() && ffprobe.exists() {
                    shared::encode::init_binary_paths(ffmpeg, ffprobe);
                }
            }
            let _ = state
                .hardware_encoder
                .set(HardwareEncoder::detect_and_test());

            let pack = state.pack.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            tauri::async_runtime::spawn(async move {
                match media_server::start(pack).await {
                    Ok(port) => {
                        tx.send(port).ok();
                    }
                    Err(e) => tracing::error!("media server failed to start: {e}"),
                }
            });
            if let Ok(port) = rx.recv() {
                state.media_port.set(port).ok();
            }
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        win.emit("close-requested", ()).ok();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            new_pack,
            open_pack_dialog,
            open_recent_pack,
            get_recent_packs,
            remove_recent_pack,
            import_edgeware_pack_dialog,
            save_pack,
            save_pack_as_dialog,
            discard_changes,
            close_pack,
            confirm_close,
            is_pack_saved,
            get_history_status,
            undo,
            redo,
            get_files,
            remove_files,
            get_modes,
            add_mode_dialog,
            remove_mode,
            set_file_title,
            set_file_source_url,
            get_all_tags,
            get_file_tags,
            add_tag_to_file,
            remove_tag_from_file,
            create_and_add_tag,
            get_tag_summaries,
            rename_tag,
            merge_tag,
            delete_tag,
            add_tag_to_files,
            remove_tag_from_files,
            get_all_artists,
            get_file_artists,
            add_artist_to_file,
            remove_artist_from_file,
            create_and_add_artist,
            get_artist_summaries,
            rename_artist,
            merge_artist,
            delete_artist,
            add_artist_to_files,
            remove_artist_from_files,
            get_pack_metadata,
            set_pack_metadata,
            save_pack_metadata,
            mark_pack_unsaved,
            get_behaviour,
            set_behaviour,
            add_files_dialog,
            add_folder_dialog,
            add_paths,
            cancel_upload,
            get_media_port,
            check_for_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
