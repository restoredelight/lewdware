use std::sync::Arc;
use std::{env, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use shared::{
    encode::{encode_audio, encode_image, encode_video, is_animated, Metadata},
    read_config::MediaCategory,
    utils::{classify_ext, FileType},
};
use tao::monitor::MonitorHandle;
use tauri::{
    async_runtime::block_on,
    http::{header::CONTENT_TYPE, Response, StatusCode},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_shell::ShellExt;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    sync::{mpsc, oneshot, RwLock},
    task::JoinSet,
};

use tokio::sync::Mutex;
use walkdir::WalkDir;

use crate::{
    monitor_plugin::MonitorPluginBuilder,
    pack::{Entry, EntryInfo, MediaInfo, MediaPack, PackedEntry},
    thumbnail::generate_thumbnail,
};

mod monitor_plugin;
mod pack;
mod thumbnail;

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
            tx.send(path).unwrap();
        });

    let path = rx.await.map_err(|err| err.to_string())?;

    println!("Got path");

    let path = match path {
        Some(FilePath::Path(x)) => x,
        Some(FilePath::Url(_)) => return Err("URLs not supported".to_string()),
        None => return Ok(None),
    };

    let pack = MediaPack::open(path).await.map_err(|err| {
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

#[tauri::command]
async fn get_monitors(state: State<'_>) -> Result<(), String> {
    let mut rx = state.monitor_rx.lock().await;

    let monitors = rx.recv().await.unwrap();

    println!("{:?}", monitors);

    Ok(())
}

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
            tx.send(path).unwrap();
        });

    let path = match rx.await.map_err(|err| err.to_string())? {
        Some(FilePath::Path(x)) => x,
        Some(FilePath::Url(_)) => return Err("URLs not supported".to_string()),
        None => return Ok(false),
    };

    let pack = MediaPack::new(path, details)
        .await
        .map_err(|err| err.to_string())?;

    let mut pack_state = state.pack.write().await;
    *pack_state = Some(pack);

    Ok(true)
}

#[tauri::command]
async fn upload_files(app_handle: AppHandle, state: State<'_>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();

    app_handle.dialog().file().pick_files(move |paths| {
        tx.send(paths).unwrap();
    });

    let paths = match rx.await.map_err(|err| err.to_string())? {
        Some(paths) => paths
            .into_iter()
            .filter_map(|path| match path {
                FilePath::Url(_) => None,
                FilePath::Path(path_buf) => Some(path_buf),
            })
            .collect(),
        None => return Ok(()),
    };

    upload_files_inner(app_handle, state, paths).await
}

#[tauri::command]
async fn upload_dir(app_handle: AppHandle, state: State<'_>) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();

    app_handle.dialog().file().pick_folder(move |path| {
        tx.send(path).unwrap();
    });

    let path = rx.await.map_err(|err| err.to_string())?;

    let path = match path {
        Some(FilePath::Path(x)) => x,
        Some(FilePath::Url(_)) => return Err("URLs not supported".to_string()),
        None => return Ok(()),
    };

    let paths: Vec<_> = WalkDir::new(&path)
        .into_iter()
        .filter_map(|x| x.ok())
        .filter(|e| e.path().is_file() && classify_ext(e.path()) != FileType::Other)
        .map(|x| x.path().to_path_buf())
        .collect();

    app_handle.emit("files_found", paths.len()).unwrap();

    upload_files_inner(app_handle, state, paths).await
}

async fn upload_files_inner(
    app_handle: AppHandle,
    state: State<'_>,
    paths: Vec<PathBuf>,
) -> Result<(), String> {
    let mut set = JoinSet::new();

    for path in paths {
        let app_handle = app_handle.clone();
        set.spawn(async move { (path.clone(), process_file(app_handle, path).await) });
    }

    while let Some(res) = set.join_next().await {
        app_handle.emit("file_processed", ()).unwrap();

        match res {
            Ok((path, res)) => match handle_result(&state, res).await {
                Ok(Some(entry)) => {
                    app_handle.emit("new_file", entry).unwrap();
                }
                Ok(None) => {}
                Err(_) => {
                    app_handle.emit("file_failed", path).unwrap();
                }
            },
            Err(err) => {
                eprintln!("{}", err);
            }
        }
    }

    Ok(())
}

async fn handle_result(
    state: &State<'_>,
    res: anyhow::Result<Option<(Vec<u8>, Metadata, FileType, PathBuf)>>,
) -> anyhow::Result<Option<Entry>> {
    match res? {
        Some((file, metadata, file_type, path)) => {
            let mut manager = state.pack.write().await;
            let manager = manager.as_mut().unwrap();

            let info = EntryInfo {
                path: path.to_string_lossy().to_string(),
                category: MediaCategory::Default,
                width: metadata.width,
                height: metadata.height,
                duration: metadata.duration.map(|x| x as i64),
            };

            let id = manager
                .add_file(PackedEntry {
                    data: file,
                    media_type: file_type,
                    info: info.clone(),
                    tags: vec![],
                })
                .await?;

            Ok(Some(Entry { id, info }))
        }
        None => Ok(None),
    }
}

async fn process_file(
    app_handle: AppHandle,
    path: PathBuf,
) -> anyhow::Result<Option<(Vec<u8>, Metadata, FileType, PathBuf)>> {
    let (tx, rx) = oneshot::channel();

    let app_handle = app_handle.clone();

    let fun = move || -> anyhow::Result<_> {
        let shell = app_handle.shell();
        let mut file_type = classify_ext(&path);

        let res = match file_type {
            FileType::Image => {
                if is_animated(shell.sidecar("ffprobe")?.into(), &path)? {
                    file_type = FileType::Video;
                    Some(encode_video(
                        shell.sidecar("ffmpeg")?.into(),
                        shell.sidecar("ffprobe")?.into(),
                        &path,
                        false,
                    )?)
                } else {
                    Some(encode_image(
                        shell.sidecar("ffmpeg")?.into(),
                        shell.sidecar("ffprobe")?.into(),
                        &path,
                    )?)
                }
            }
            FileType::Video => Some(encode_video(
                shell.sidecar("ffmpeg")?.into(),
                shell.sidecar("ffprobe")?.into(),
                &path,
                true,
            )?),
            FileType::Audio => Some((
                encode_audio(shell.sidecar("ffmpeg")?.into(), &path)?,
                Metadata::default(),
            )),
            FileType::Other => None,
        };

        Ok(res.map(|x| (x.0, x.1, file_type, path)))
    };

    rayon::spawn(move || {
        let _ = tx.send(fun());
    });

    rx.await?
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
    let manager = manager.as_ref().unwrap();

    manager
        .get_file_info(id)
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn get_file_tags(state: State<'_>, id: u64) -> Result<Vec<String>, String> {
    let manager = state.pack.read().await;
    let manager = manager.as_ref().unwrap();

    manager.get_tags(id).await.map_err(|err| err.to_string())
}

#[tauri::command]
async fn delete_file_view(state: State<'_>, id: u64) -> Result<(), String> {
    let path = state.temp_dir.join(format!("{}-view.avif", id));

    fs::remove_file(path).map_err(|err| err.to_string())
}

async fn get_file_path(state: State<'_>, id: u64) -> Result<PathBuf, String> {
    let (data, file_type) = {
        let mut manager = state.pack.write().await;
        let manager = manager.as_mut().unwrap();

        manager.get_file(id).await.map_err(|err| err.to_string())?
    };

    let extension = match file_type {
        FileType::Image => "avif",
        FileType::Video => "mp4",
        FileType::Audio => "opus",
        FileType::Other => return Err("Invalid file type".to_string()),
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

    Ok(path)
}

pub struct AppState {
    pack: RwLock<Option<MediaPack>>,
    temp_dir: PathBuf,
    monitor_rx: Arc<Mutex<mpsc::Receiver<Vec<MonitorHandle>>>>,
}

pub type State<'a> = tauri::State<'a, AppState>;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let temp_dir = env::temp_dir().join("lewdware-pack-editor");

    let (tx, rx) = mpsc::channel(10);

    fs::create_dir_all(&temp_dir).unwrap();

    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("image", |ctx, request, responder| {
            let path = request.uri().path()[1..].to_string();

            let app_handle = ctx.app_handle().clone();

            tauri::async_runtime::spawn(async move {
                let mut parts = path.split("/");

                let state: State<'_> = app_handle.state();

                match parts.next() {
                    Some("thumbnail") => {
                        let id: u64 = parts.next().unwrap().parse().unwrap();

                        let (data, file_type) = {
                            let mut manager = state.pack.write().await;
                            let manager = manager.as_mut().unwrap();

                            manager
                                .get_file(id)
                                .await
                                .map_err(|err| err.to_string())
                                .unwrap()
                        };

                        let thumbnail = generate_thumbnail(
                            app_handle,
                            data,
                            file_type == FileType::Image,
                            false,
                        )
                        .await
                        .unwrap();

                        let response = Response::builder()
                            .header(CONTENT_TYPE, "image/png")
                            .body(thumbnail)
                            .unwrap();

                        responder.respond(response);
                    }
                    Some("big-thumbnail") => {
                        let id: u64 = parts.next().unwrap().parse().unwrap();

                        let (data, file_type) = {
                            let mut manager = state.pack.write().await;
                            let manager = manager.as_mut().unwrap();

                            manager
                                .get_file(id)
                                .await
                                .map_err(|err| err.to_string())
                                .unwrap()
                        };

                        let thumbnail = generate_thumbnail(
                            app_handle,
                            data,
                            file_type == FileType::Image,
                            true,
                        )
                        .await
                        .unwrap();

                        let response = Response::builder()
                            .header(CONTENT_TYPE, "image/png")
                            .body(thumbnail)
                            .unwrap();

                        responder.respond(response);
                    }
                    Some("image") => {
                        let id: u64 = parts.next().unwrap().parse().unwrap();

                        let (data, file_type) = {
                            let mut manager = state.pack.write().await;
                            let manager = manager.as_mut().unwrap();

                            manager
                                .get_file(id)
                                .await
                                .map_err(|err| err.to_string())
                                .unwrap()
                        };

                        let content_type = match file_type {
                            FileType::Image => "image/avif",
                            FileType::Video => "video/mp4",
                            FileType::Audio => "audio/opus",
                            FileType::Other => return,
                        };

                        let response = Response::builder()
                            .header(CONTENT_TYPE, content_type)
                            .body(data)
                            .unwrap();

                        responder.respond(response);
                    }
                    _ => {
                        let response = Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .header(CONTENT_TYPE, "text/plain")
                            .body("Bad request".as_bytes().to_vec())
                            .unwrap();

                        responder.respond(response);
                    }
                }
            });
        })
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            pack: RwLock::new(None),
            temp_dir,
            monitor_rx: Arc::new(Mutex::new(rx)),
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            open_pack,
            create_pack,
            upload_files,
            upload_dir,
            open_file,
            get_file_tags,
            delete_file_view,
            get_monitors,
        ])
        .setup(|app| {
            app.wry_plugin(MonitorPluginBuilder::new(tx));

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Error building tauri application")
        .run(|app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let state: State = app_handle.state();

                let mut pack = block_on(state.pack.write());

                if let Some(pack) = pack.as_mut() {
                    block_on(pack.write_changes(false)).unwrap();
                }

                if let Err(e) = fs::remove_dir_all(&state.temp_dir) {
                    eprintln!("{}", e);
                }
            }
        });
}
