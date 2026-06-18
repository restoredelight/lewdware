mod encode;
mod media_server;
mod pack;
mod thumbnail;

use std::{path::PathBuf, sync::Arc};

use pack::{MediaFile, MediaPack};
use serde::{Deserialize, Serialize};

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
use shared::read_pack::Metadata;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

use crate::encode::HardwareEncoder;

pub type PackState = Arc<Mutex<Option<MediaPack>>>;

pub struct AppState {
    pub pack: PackState,
    pub media_port: std::sync::OnceLock<u16>,
    pub hardware_encoder: HardwareEncoder,
}

impl AppState {
    fn new(hardware_encoder: HardwareEncoder) -> Self {
        Self {
            pack: Arc::new(Mutex::new(None)),
            media_port: std::sync::OnceLock::new(),
            hardware_encoder,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackInfo {
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MetadataDto {
    pub name: String,
    pub creator: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
}

impl From<Metadata> for MetadataDto {
    fn from(m: Metadata) -> Self {
        Self {
            name: m.name,
            creator: m.creator,
            description: m.description,
            version: m.version,
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
            ..Default::default()
        }
    }
}

// ── Pack lifecycle ───────────────────────────────────────────────────────────

#[tauri::command]
async fn new_pack_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<Option<PackInfo>, String> {
    use tauri_plugin_dialog::DialogExt;
    let app_c = app.clone();
    let file = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Create new pack")
            .add_filter("Lewdware Pack", &["md"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(path) = file else { return Ok(None) };
    let path: PathBuf = path.into_path().map_err(|e| e.to_string())?;

    // Prompt for a name based on the file stem
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "New Pack".to_string());

    let pack = MediaPack::new(path, &name)
        .await
        .map_err(|e| e.to_string())?;
    let info = PackInfo { name: pack.name() };
    *state.pack.lock().await = Some(pack);
    Ok(Some(info))
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
            .add_filter("Lewdware Pack", &["md"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(path) = file else { return Ok(None) };
    let path: PathBuf = path.into_path().map_err(|e| e.to_string())?;

    let pack = MediaPack::open(path).await.map_err(|e| e.to_string())?;
    let info = PackInfo { name: pack.name() };
    *state.pack.lock().await = Some(pack);
    Ok(Some(info))
}

#[tauri::command]
async fn save_pack(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        let app_cb = app.clone();
        pack.save(move |saved, t| {
            let _ = app_cb.emit(
                "save:progress",
                serde_json::json!({ "saved": saved, "total": t }),
            );
        })
        .await
        .map_err(|e| e.to_string())?;
        let _ = app.emit("save:done", ());
    }
    Ok(())
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
            .add_filter("Lewdware Pack", &["md"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(path) = file else { return Ok(None) };
    let path: PathBuf = path.into_path().map_err(|e| e.to_string())?;

    let mut lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
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
            };
            *lock = Some(new_pack);
            let _ = app.emit("save:done", ());
            return Ok(Some(info));
        }
    }
    Ok(None)
}

#[tauri::command]
async fn discard_changes(state: State<'_, AppState>) -> Result<MetadataDto, String> {
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
    *state.pack.lock().await = None;
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

#[tauri::command]
async fn set_file_title(state: State<'_, AppState>, id: u64, name: String) -> Result<(), String> {
    let lock = state.pack.lock().await;
    if let Some(pack) = lock.as_ref() {
        pack.set_title(id, name).await.map_err(|e| e.to_string())?;
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

// ── Upload ───────────────────────────────────────────────────────────────────

#[tauri::command]
async fn add_files_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
    skip_duplicates: bool,
) -> Result<(), String> {
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
    tauri::async_runtime::spawn(encode::process_files(
        pack_state,
        paths,
        skip_duplicates,
        app,
        state.hardware_encoder.clone(),
    ));
    Ok(())
}

#[tauri::command]
async fn add_folder_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
    recursive: bool,
    skip_duplicates: bool,
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
    tauri::async_runtime::spawn(encode::process_files(
        pack_state,
        paths,
        skip_duplicates,
        app,
        state.hardware_encoder.clone(),
    ));
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
    let hardware_encoder = HardwareEncoder::detect_and_test();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new(hardware_encoder))
        .setup(|app| {
            let state = app.state::<AppState>();
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            new_pack_dialog,
            open_pack_dialog,
            save_pack,
            save_pack_as_dialog,
            discard_changes,
            close_pack,
            is_pack_saved,
            get_files,
            remove_files,
            set_file_title,
            get_all_tags,
            get_file_tags,
            add_tag_to_file,
            remove_tag_from_file,
            create_and_add_tag,
            get_pack_metadata,
            set_pack_metadata,
            save_pack_metadata,
            mark_pack_unsaved,
            add_files_dialog,
            add_folder_dialog,
            get_media_port,
            check_for_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
