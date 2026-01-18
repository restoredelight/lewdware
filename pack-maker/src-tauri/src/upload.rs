use std::{
    fs::File,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail};
use bounded_join_set::JoinSet;
use futures::{stream, StreamExt};
use infer::MatcherType;
use mime_guess::mime;
use serde::{Deserialize, Serialize};
use shared::{
    encode::{encode_file, FileInfo},
    read_config::MediaCategory,
};
use tauri::{
    async_runtime::{self, spawn_blocking},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_shell::ShellExt;
use tokio::sync::oneshot;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{
    pack::{EntryInfo, MediaInfo, PackedEntry},
    State,
};

#[derive(Serialize, Deserialize, Clone)]
struct Error {
    path: String,
    message: String,
}

#[tauri::command]
pub async fn upload_files(app_handle: AppHandle) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();

    app_handle.dialog().file().pick_files(move |paths| {
        if tx.send(paths).is_err() {
            eprintln!("Receiver dropped");
        }
    });

    let paths: Vec<_> = match rx.await.map_err(|err| err.to_string())? {
        Some(paths) => paths
            .into_iter()
            .filter_map(|path| match path {
                FilePath::Url(_) => None,
                FilePath::Path(path_buf) => Some(path_buf),
            })
            .collect(),
        None => return Ok(()),
    };

    spawn_upload_files_task(app_handle, paths).await
}

#[tauri::command]
pub async fn upload_dir(app_handle: AppHandle) -> Result<(), String> {
    let (tx, rx) = oneshot::channel();

    app_handle.dialog().file().pick_folder(move |path| {
        if tx.send(path).is_err() {
            eprintln!("Receiver dropped");
        }
    });

    let path = rx.await.map_err(|err| err.to_string())?;

    let path = match path {
        Some(FilePath::Path(x)) => x,
        Some(FilePath::Url(_)) => return Err("URLs not supported".to_string()),
        None => return Ok(()),
    };

    let mut i = 0;

    let paths: Vec<_> = WalkDir::new(&path)
        .into_iter()
        .filter_map(|x| x.ok())
        .filter(|e| {
            let path = e.path();

            if !path.is_file() {
                return false;
            }

            println!("{i}");
            i += 1;

            match is_media(path) {
                Ok(x) => x,
                Err(err) => {
                    eprintln!("{err}");
                    false
                }
            }
        })
        .map(|x| x.path().to_path_buf())
        .collect();

    spawn_upload_files_task(app_handle, paths).await
}

#[tauri::command]
pub async fn drop_files(app_handle: AppHandle, paths: Vec<String>) -> Result<(), String> {
    let paths: Vec<_> = paths.iter().map(|x| PathBuf::from(x)).collect();

    spawn_upload_files_task(app_handle, paths).await
}

#[tauri::command]
pub async fn cancel_media_tasks(state: State<'_>) -> Result<(), ()> {
    let mut tasks = state.media_tasks.lock().await;

    for task in tasks.drain(..) {
        task.abort()
    }

    Ok(())
}

async fn spawn_upload_files_task(app_handle: AppHandle, paths: Vec<PathBuf>) -> Result<(), String> {
    app_handle
        .emit("files_found", paths.len())
        .map_err(|err| err.to_string())?;

    let app_handle_clone = app_handle.clone();

    let task = async_runtime::spawn(async move {
        if let Err(err) = upload_files_inner(app_handle_clone, paths).await {
            eprintln!("{err}");
        }
    });

    let state: State = app_handle.state();
    let mut tasks = state.media_tasks.lock().await;
    tasks.push(task);

    Ok(())
}

async fn upload_files_inner(app_handle: AppHandle, paths: Vec<PathBuf>) -> Result<(), String> {
    let state: State = app_handle.state();

    let (dir, id) = {
        let pack = state.pack.read().await;
        let pack = pack.as_ref().ok_or_else(|| "No pack")?;
        (pack.dir().to_path_buf(), pack.id().clone())
    };

    stream::iter(paths)
        .for_each_concurrent(Some(10), |path| {
            let state: State<'_> = app_handle.state();
            let app_handle = app_handle.clone();
            let dir = dir.clone();

            async move {
                let app_handle_clone = app_handle.clone();
                let result = process_file(app_handle_clone, dir, path.clone()).await;

                match handle_result(&state, &path, result).await {
                    Ok(Some(entry)) => {
                        println!("File handles successfully");
                        app_handle.emit("new_file", entry).unwrap();
                    }
                    Ok(None) => {
                        app_handle.emit("file_ignored", ()).unwrap();
                        eprintln!("Nothing");
                    }
                    Err(err) => {
                        eprintln!("{err}");
                        app_handle
                            .emit(
                                "file_failed",
                                Error {
                                    path: path
                                        .file_name()
                                        .unwrap_or(path.as_os_str())
                                        .to_string_lossy()
                                        .to_string(),
                                    message: err.to_string(),
                                },
                            )
                            .unwrap();
                    }
                }
            }
        })
        .await;

    Ok(())
}

fn spawn_file_task(
    app_handle: &AppHandle,
    set: &mut JoinSet<FileResult>,
    dir: &PathBuf,
    path: PathBuf,
) {
    let app_handle = app_handle.clone();
    let dir = dir.to_path_buf();

    set.spawn(async move {
        let result = process_file(app_handle, dir, path.clone()).await;
        FileResult {
            input_path: path,
            result,
        }
    });
}

fn is_media(path: &Path) -> anyhow::Result<bool> {
    let guess = mime_guess::from_path(path);

    if guess.iter().any(|mime| {
        let mime_type = mime.type_();

        mime_type == mime::IMAGE || mime_type == mime::VIDEO || mime_type == mime::AUDIO
    }) {
        return Ok(true);
    }

    if guess.first().is_some() {
        return Ok(false);
    }

    let better_guess = infer::get_from_path(path)?;
    if let Some(guess) = better_guess {
        let file_type = guess.matcher_type();

        if file_type == MatcherType::Image
            || file_type == MatcherType::Audio
            || file_type == MatcherType::Video
        {
            return Ok(true);
        }
    }

    Ok(false)
}

struct FileResult {
    input_path: PathBuf,
    result: anyhow::Result<Option<ProcessedFile>>,
}

struct ProcessedFile {
    output_path: PathBuf,
    file_info: FileInfo,
    hash: blake3::Hash,
}

async fn handle_result(
    state: &State<'_>,
    input_path: &Path,
    res: anyhow::Result<Option<ProcessedFile>>,
) -> anyhow::Result<Option<MediaInfo>> {
    match res? {
        Some(result) => {
            let mut manager = state.pack.write().await;
            let manager = manager.as_mut().ok_or_else(|| anyhow!("No open pack"))?;

            let file_name = input_path
                .file_name()
                .map(|x| x.to_string_lossy().to_string())
                .unwrap_or("".to_string());

            let info = EntryInfo {
                file_name: file_name.clone(),
                category: MediaCategory::Default,
                file_info: result.file_info.clone(),
            };

            let id = manager
                .add_file(PackedEntry {
                    path: result.output_path,
                    info,
                    tags: vec![],
                    hash: result.hash,
                })
                .await?;

            let info = MediaInfo {
                id: id,
                file_name,
                file_info: result.file_info,
                category: MediaCategory::Default,
            };

            Ok(Some(info))
        }
        None => Ok(None),
    }
}

pub fn hash_file(path: &Path) -> anyhow::Result<blake3::Hash> {
    let file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();

    hasher.update_reader(file)?;

    Ok(hasher.finalize())
}

async fn process_file(
    app_handle: AppHandle,
    dir: PathBuf,
    path: PathBuf,
) -> anyhow::Result<Option<ProcessedFile>> {
    let hash = {
        let path_clone = path.clone();

        let hash = spawn_blocking(move || hash_file(&path_clone)).await??;

        let state: State<'_> = app_handle.state();

        let pack = state.pack.read().await;
        let pack = pack.as_ref().ok_or_else(|| anyhow!("No pack"))?;

        if pack.check_hash(&hash).await? {
            bail!("Duplicate file (skipped)");
        };

        hash
    };

    let (tx, rx) = oneshot::channel();

    let app_handle = app_handle.clone();

    let fun = move || -> anyhow::Result<_> {
        app_handle.emit(
            "processing_started",
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .to_string(),
        )?;
        let shell = app_handle.shell();

        let id = Uuid::new_v4();

        let output_path = dir.join("media").join(id.to_string());

        let result = encode_file(
            || {
                shell
                    .sidecar("ffmpeg")
                    .expect("Missing ffmpeg sidecar")
                    .into()
            },
            || {
                shell
                    .sidecar("ffprobe")
                    .expect("Missing ffprobe sidecar")
                    .into()
            },
            &path,
            &output_path,
        )?;

        Ok(result.map(|(file_info, output_path)| ProcessedFile {
            output_path,
            file_info,
            hash,
        }))
    };

    rayon::spawn(move || {
        if tx.is_closed() {
            return;
        }

        let _ = tx.send(fun());
    });

    rx.await?
}
