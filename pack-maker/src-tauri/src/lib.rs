use std::{env, fs, path::PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use shared::encode::FileType;
use tauri::async_runtime;
use tauri::async_runtime::JoinHandle;
use tauri::{async_runtime::block_on, AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    sync::{oneshot, RwLock},
};

use tokio::sync::Mutex;

use crate::media_protocol::start_media_server;
use crate::pack::FileData;
use crate::pack::{EntryInfo, MediaInfo, MediaPack};
use crate::upload::{cancel_media_tasks, drop_files, upload_dir, upload_files};

mod media_protocol;
mod pack;
mod thumbnail;
mod upload;

#[derive(Serialize, Deserialize)]
struct PackInfo {
    files: Vec<MediaInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct CreatePackDetails {
    name: String,
}

#[tauri::command]
async fn open_pack(app_handle: AppHandle, state: State<'_>) -> Result<Option<PackInfo>, String> {
    let (tx, rx) = oneshot::channel();

    app_handle
        .dialog()
        .file()
        .set_title("Open a media pack")
        .add_filter("Media pack", &["md"])
        .pick_file(move |path| {
            if tx.send(path).is_err() {
                eprintln!("Receiver dropped");
            }
        });

    let path = rx.await.map_err(|err| err.to_string())?;

    println!("Got path");

    let path = match path {
        Some(FilePath::Path(x)) => x,
        Some(FilePath::Url(_)) => return Err("URLs not supported".to_string()),
        None => return Ok(None),
    };

    let pack = MediaPack::open(
        path,
        app_handle
            .path()
            .data_dir()
            .map_err(|err| err.to_string())?,
    )
    .await
    .map_err(|err| {
        eprintln!("Error opening media pack");
        println!("{:?}", err);
        err.to_string()
    })?;

    println!("Opened media pack");

    let files = pack.get_files().await.map_err(|err| err.to_string())?;

    println!("Got files");

    let mut pack_state = state.pack.write().await;
    *pack_state = Some(pack);

    println!("Set pack state; returnng pack info");

    Ok(Some(PackInfo { files }))
}

// #[tauri::command]
// async fn get_monitors(state: State<'_>) -> Result<(), String> {
//     return Ok(());
//
//     let mut rx = state.monitor_rx.lock().await;
//
//     let monitors = rx.recv().await.unwrap();
//
//     println!("{:?}", monitors);
//
//     Ok(())
// }

#[tauri::command]
async fn create_pack(
    app_handle: AppHandle,
    state: State<'_>,
    details: CreatePackDetails,
) -> Result<bool, String> {
    let (tx, rx) = oneshot::channel();

    app_handle
        .dialog()
        .file()
        .set_file_name(format!("{}.md", details.name))
        .add_filter("Media pack", &["md"])
        .save_file(move |path| {
            if tx.send(path).is_err() {
                eprintln!("Receiver dropped");
            }
        });

    let path = match rx.await.map_err(|err| err.to_string())? {
        Some(FilePath::Path(x)) => x,
        Some(FilePath::Url(_)) => return Err("URLs not supported".to_string()),
        None => return Ok(false),
    };

    let pack = MediaPack::new(
        path,
        details,
        app_handle
            .path()
            .data_dir()
            .map_err(|err| err.to_string())?,
    )
    .await
    .map_err(|err| err.to_string())?;

    let mut pack_state = state.pack.write().await;
    *pack_state = Some(pack);

    Ok(true)
}

#[tauri::command]
async fn get_pack_info(state: State<'_>) -> Result<Option<PackInfo>, String> {
    let mut pack = state.pack.write().await;

    if let Some(pack) = pack.as_mut() {
        let files = pack.get_files().await.map_err(|err| err.to_string())?;

        Ok(Some(PackInfo { files }))
    } else {
        Ok(None)
    }
}

// #[tauri::command]
// async fn get_thumbnail(
//     app_handle: AppHandle,
//     state: State<'_>,
//     id: u64,
//     is_image: bool,
//     large: bool,
// ) -> Result<String, String> {
//     // let _semaphore = state
//     //     .thumbnail_semaphore
//     //     .acquire()
//     //     .await
//     //     .map_err(|err| err.to_string())?;
//
//     println!("Getting thumbnail");
//
//     let data = {
//         let mut manager = state.pack.write().await;
//         let manager = manager.as_mut().unwrap();
//
//         manager.get_file(id).await.map_err(|err| err.to_string())?
//     };
//
//     let thumbnail = generate_thumbnail(app_handle, id, data, is_image, &state.temp_dir, large)
//         .await
//         .map_err(|err| err.to_string())?;
//
//     Ok(thumbnail.to_string_lossy().to_string())
// }

#[tauri::command]
async fn open_file(app_handle: AppHandle, state: State<'_>, id: u64) -> Result<(), String> {
    let path = get_file_path(state, id).await?;

    app_handle
        .opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
        .map_err(|err| err.to_string())?;

    Ok(())
}
//
// #[tauri::command]
// async fn get_file(state: State<'_>, id: u64) -> Result<String, String> {
//     let path = get_file_path(state, id).await?;
//
//     Ok(path.to_string_lossy().to_string())
// }

#[tauri::command]
async fn get_file_info(state: State<'_>, id: u64) -> Result<EntryInfo, String> {
    let manager = state.pack.read().await;
    let manager = manager.as_ref().ok_or("No open pack".to_string())?;

    manager
        .get_file_info(id)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn get_file_tags(state: State<'_>, id: u64) -> Result<Vec<String>, String> {
    let manager = state.pack.read().await;
    let manager = manager.as_ref().ok_or("No open pack".to_string())?;

    manager.get_tags(id).await.map_err(|err| err.to_string())
}

#[tauri::command]
async fn delete_file_view(state: State<'_>, id: u64) -> Result<(), String> {
    let path = state.temp_dir.join(format!("{}-view.avif", id));

    fs::remove_file(path).map_err(|err| err.to_string())
}

async fn get_file_path(state: State<'_>, id: u64) -> Result<PathBuf, String> {
    let (file_data, file_type) = {
        let mut manager = state.pack.write().await;
        let manager = manager.as_mut().ok_or("No open pack".to_string())?;

        manager.get_file(id).await.map_err(|err| err.to_string())?
    };

    let path = match file_data {
        FileData::Path(path) => path,
        FileData::Data(data) => {
            let extension = match file_type {
                FileType::Image => "avif",
                FileType::Video => "webm",
                FileType::Audio => "opus",
            };

            let path = state.temp_dir.join(format!("{}-view.{extension}", id));

            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)
                .await
                .map_err(|err| err.to_string())?;

            file.write_all(&data).await.map_err(|err| err.to_string())?;

            path
        }
    };

    Ok(path)
}

#[tauri::command]
async fn media_server_port(state: State<'_>) -> Result<u16, String> {
    let mut port = state.media_server_port.lock().await;

    if let Some(port) = *port {
        Ok(port)
    } else {
        let port_msg = state
            .port_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| "Receiver already taken")?
            .await
            .map_err(|err| err.to_string())?;

        *port = Some(port_msg);
        Ok(port_msg)
    }
}

pub struct AppState {
    pack: RwLock<Option<MediaPack>>,
    temp_dir: PathBuf,
    port_rx: Mutex<Option<oneshot::Receiver<u16>>>,
    media_server_port: Mutex<Option<u16>>,
    media_tasks: Mutex<Vec<JoinHandle<()>>>,
}

pub type State<'a> = tauri::State<'a, AppState>;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let temp_dir = env::temp_dir().join("lewdware-pack-editor");

    let (port_tx, port_rx) = oneshot::channel();

    fs::create_dir_all(&temp_dir).unwrap();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            pack: RwLock::new(None),
            temp_dir,
            port_rx: Mutex::new(Some(port_rx)),
            media_server_port: Mutex::new(None),
            media_tasks: Mutex::new(Vec::new()),
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            open_pack,
            create_pack,
            get_pack_info,
            upload_files,
            upload_dir,
            drop_files,
            cancel_media_tasks,
            open_file,
            get_file_tags,
            delete_file_view,
            media_server_port,
        ])
        .setup(|app| {
            async_runtime::spawn(start_media_server(app.handle().clone(), port_tx));

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Error building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state: State = app_handle.state();

                let mut pack = block_on(state.pack.write());

                if let Some(pack) = pack.as_mut() {
                    println!("Writing changes");
                    if let Err(err) = block_on(pack.write_changes()) {
                        eprintln!("{err}");
                    };
                }

                if let Err(e) = fs::remove_dir_all(&state.temp_dir) {
                    eprintln!("{}", e);
                }
            }
        });
}
