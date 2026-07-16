use std::{
    future::Future,
    io,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    thread::available_parallelism,
};

use anyhow::anyhow;
use futures::{stream, StreamExt};
use infer::MatcherType;
use shared::encode::{encode_file, hash_file, HardwareEncoder};
use tempfile::TempDir;
use tokio::sync::{oneshot, watch, RwLock, Semaphore};
use uuid::Uuid;
use walkdir::WalkDir;

use tauri::Emitter;

use crate::pack::MediaFile;

#[derive(Debug)]
pub enum ProcessErrorKind {
    Skipped,
    Unrecognized,
    EncodeError(anyhow::Error),
    PackError(anyhow::Error),
    HashError(io::Error),
    Other(anyhow::Error),
}

impl std::fmt::Display for ProcessErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => write!(f, "Duplicate (skipped)"),
            Self::Unrecognized => write!(f, "Not a recognizable image/video/audio file"),
            Self::EncodeError(e) => write!(f, "Encode error: {e}"),
            Self::PackError(e) => write!(f, "Pack error: {e}"),
            Self::HashError(e) => write!(f, "Hash error: {e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

/// Resolves when the current import batch is cancelled; pends forever if it can't be.
pub(crate) async fn wait_cancelled(cancel: &mut watch::Receiver<bool>) {
    if cancel.wait_for(|cancelled| *cancelled).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Wraps a handle to work running outside this future (a rayon encode, a blocking extract).
/// If the wrapping future is dropped before the work lands — the import was cancelled — the
/// work itself can't be interrupted, so await it in a detached task and delete its output file.
pub(crate) struct DiscardOnDrop<F: Future + Send + Unpin + 'static>
where
    F::Output: Send,
{
    work: Option<F>,
    output: PathBuf,
    staging: Option<Arc<TempDir>>,
}

impl<F: Future + Send + Unpin + 'static> DiscardOnDrop<F>
where
    F::Output: Send,
{
    pub(crate) fn new(work: F, output: PathBuf, staging: Arc<TempDir>) -> Self {
        Self {
            work: Some(work),
            output,
            staging: Some(staging),
        }
    }

    pub(crate) async fn finish(mut self) -> F::Output {
        let result = self.work.as_mut().expect("finish called once").await;
        self.work = None;
        result
    }
}

impl<F: Future + Send + Unpin + 'static> Drop for DiscardOnDrop<F>
where
    F::Output: Send,
{
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            let output = std::mem::take(&mut self.output);
            let staging = self.staging.take();
            tauri::async_runtime::spawn(async move {
                let _ = work.await;
                let _ = tokio::fs::remove_file(&output).await;
                drop(staging);
            });
        }
    }
}

static ENCODE_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

fn encode_semaphore() -> &'static Semaphore {
    ENCODE_SEMAPHORE.get_or_init(|| {
        let permits = available_parallelism()
            .map(|x| (x.get() / 4).max(2))
            .unwrap_or(2);
        Semaphore::new(permits)
    })
}

/// OS-generated cruft that can end up sitting next to real media (AppleDouble sidecar files and
/// the `__MACOSX/` mirror directory a zip built on macOS carries, Windows thumbnail/folder-view
/// caches) but isn't itself media -- excluded here so it never reaches ffprobe and shows up as a
/// spurious error.
pub fn is_junk_path(path: &Path) -> bool {
    let is_junk_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("._") || n == ".DS_Store" || n == "Thumbs.db" || n == "desktop.ini")
        .unwrap_or(false);
    is_junk_name || path.components().any(|c| c.as_os_str() == "__MACOSX")
}

pub fn explore_folder(path: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut walkdir = WalkDir::new(path);
    if !recursive {
        walkdir = walkdir.max_depth(1);
    }
    walkdir
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file()
                && !is_junk_path(e.path())
                && is_media_path(e.path()).unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

pub fn is_media_path(path: &Path) -> anyhow::Result<bool> {
    let guess = mime_guess::from_path(path);
    if guess.iter().any(|m| {
        let t = m.type_();
        t == mime_guess::mime::IMAGE || t == mime_guess::mime::VIDEO || t == mime_guess::mime::AUDIO
    }) {
        return Ok(true);
    }
    if guess.first().is_some() {
        return Ok(false);
    }
    if let Some(g) = infer::get_from_path(path)? {
        let t = g.matcher_type();
        if t == MatcherType::Image || t == MatcherType::Audio || t == MatcherType::Video {
            return Ok(true);
        }
    }
    Ok(false)
}

// Called from Tauri commands via AppHandle
pub async fn process_files(
    pack_state: crate::PackState,
    paths: Vec<PathBuf>,
    app: tauri::AppHandle,
    encoder: HardwareEncoder,
    upload_lock: Arc<RwLock<()>>,
    mut cancel: watch::Receiver<bool>,
) {
    let total = paths.len();
    let _ = app.emit("upload:start", serde_json::json!({ "total": total }));

    let (dir, pack_id) = {
        let lock = pack_state.lock().await;
        match lock.as_ref() {
            Some(pack) => (pack.dir().to_path_buf(), pack.id()),
            None => return,
        }
    };
    let staging = match tempfile::Builder::new()
        .prefix("pack-editor-upload-")
        .tempdir_in(&dir)
    {
        Ok(staging) => Arc::new(staging),
        Err(error) => {
            let _ = app.emit(
                "upload:error",
                serde_json::json!({ "path": dir, "error": format!("Could not create upload staging directory: {error}") }),
            );
            let _ = app.emit("upload:done", ());
            return;
        }
    };

    let limit = available_parallelism().map(|x| x.get()).ok();

    let process_all = stream::iter(paths).for_each_concurrent(limit, |path| {
        let pack_state = pack_state.clone();
        let app = app.clone();
        let dir = dir.clone();
        let encoder = encoder.clone();
        let upload_lock = upload_lock.clone();
        let staging = staging.clone();
        async move {
            match process_one_file(
                &pack_state,
                pack_id,
                &path,
                &dir,
                &staging,
                encoder,
                &upload_lock,
            )
            .await
            {
                Ok(Some(media_file)) => {
                    let _ = app.emit("upload:added", &media_file);
                }
                Ok(None) => {}
                Err(ProcessErrorKind::Skipped) => {
                    let _ = app.emit("upload:skipped", path.to_string_lossy().as_ref());
                }
                Err(err) => {
                    let _ = app.emit(
                        "upload:error",
                        serde_json::json!({
                            "path": path.to_string_lossy(),
                            "error": err.to_string()
                        }),
                    );
                }
            }
            let _ = app.emit("upload:file-done", ());
        }
    });
    tokio::pin!(process_all);

    // Dropping the stream is the cancellation: queued paths are never visited, in-flight
    // files stop at their next await, and encodes already running on the rayon pool finish
    // detached with their output discarded (see DiscardOnDrop).
    tokio::select! {
        _ = &mut process_all => {}
        _ = wait_cancelled(&mut cancel) => {}
    }

    let _ = app.emit("upload:done", ());
}

async fn process_one_file(
    pack_state: &crate::PackState,
    pack_id: Uuid,
    path: &Path,
    dir: &Path,
    staging: &Arc<TempDir>,
    encoder: HardwareEncoder,
    upload_lock: &RwLock<()>,
) -> Result<Option<MediaFile>, ProcessErrorKind> {
    let path_owned = path.to_path_buf();
    let hash = tokio::task::spawn_blocking(move || hash_file(&path_owned))
        .await
        .map_err(|e| ProcessErrorKind::Other(e.into()))?
        .map_err(ProcessErrorKind::HashError)?;

    // Duplicates are always rejected (add_file enforces this with a DB-level
    // constraint, so this can't be turned off) - checking here just avoids
    // wasting an encode on a file we already know will be rejected.
    {
        let lock = pack_state.lock().await;
        if let Some(pack) = lock.as_ref() {
            if pack
                .check_hash(&hash)
                .await
                .map_err(ProcessErrorKind::Other)?
            {
                return Err(ProcessErrorKind::Skipped);
            }
        }
    }

    let _permit = encode_semaphore()
        .acquire()
        .await
        .map_err(|e| ProcessErrorKind::Other(anyhow!("{e}")))?;

    let id = Uuid::new_v4();
    let output_path = staging.path().join(id.to_string());
    let path_owned = path.to_path_buf();

    let (tx, rx) = oneshot::channel();
    let encode_output = output_path.clone();
    rayon::spawn(move || {
        let _ = tx.send(encode_file(&path_owned, &encode_output, encoder));
    });

    let encoded = DiscardOnDrop::new(rx, output_path, staging.clone())
        .finish()
        .await
        .map_err(|e| ProcessErrorKind::Other(e.into()))?
        .map_err(ProcessErrorKind::EncodeError)?;

    let mut encoded = match encoded {
        Some(e) => e,
        None => return Err(ProcessErrorKind::Unrecognized),
    };

    // Saving only needs to exclude the atomic commit into the managed media directory and the
    // corresponding database update. Encoding itself happens in pack-local staging storage.
    let _read_guard = upload_lock.read().await;
    let mut lock = pack_state.lock().await;
    let pack = lock
        .as_mut()
        .filter(|pack| pack.id() == pack_id)
        .ok_or_else(|| ProcessErrorKind::Other(anyhow!("The pack was closed while encoding")))?;
    let final_path = dir.join("media").join(
        encoded
            .path
            .file_name()
            .ok_or_else(|| ProcessErrorKind::Other(anyhow!("Encoded file has no name")))?,
    );
    tokio::fs::rename(&encoded.path, &final_path)
        .await
        .map_err(|e| ProcessErrorKind::Other(e.into()))?;
    encoded.path = final_path;

    let media = pack
        .add_file(encoded, path, hash)
        .await
        .map_err(ProcessErrorKind::PackError)?;
    match media {
        // The pre-check above already handles the common case; this only
        // fires when a race let a since-inserted duplicate through, and the
        // DB's own uniqueness constraint caught it - treat it the same as
        // the pre-check's skip.
        Some(media) => Ok(Some(media)),
        None => Err(ProcessErrorKind::Skipped),
    }
}
