use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, OnceLock,
    },
    thread::available_parallelism,
};

use anyhow::anyhow;
use futures::{stream, StreamExt};
use infer::MatcherType;
use shared::encode::{encode_file, hash_file, HardwareEncoder};
use tokio::sync::{oneshot, RwLock, Semaphore};
use uuid::Uuid;
use walkdir::WalkDir;

use tauri::Emitter;

use crate::pack::MediaFile;

#[derive(Debug)]
pub enum ProcessErrorKind {
    Skipped,
    EncodeError(anyhow::Error),
    PackError(anyhow::Error),
    HashError(io::Error),
    Other(anyhow::Error),
}

impl std::fmt::Display for ProcessErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => write!(f, "Duplicate (skipped)"),
            Self::EncodeError(e) => write!(f, "Encode error: {e}"),
            Self::PackError(e) => write!(f, "Pack error: {e}"),
            Self::HashError(e) => write!(f, "Hash error: {e}"),
            Self::Other(e) => write!(f, "{e}"),
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

pub fn explore_folder(path: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut walkdir = WalkDir::new(path);
    if !recursive {
        walkdir = walkdir.max_depth(1);
    }
    walkdir
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file() && is_media_path(e.path()).unwrap_or(false))
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
    cancel: Arc<AtomicBool>,
) {
    let total = paths.len();
    let _ = app.emit("upload:start", serde_json::json!({ "total": total }));

    let dir = {
        let lock = pack_state.lock().await;
        match lock.as_ref() {
            Some(pack) => pack.dir().to_path_buf(),
            None => return,
        }
    };

    let limit = available_parallelism().map(|x| x.get()).ok();

    stream::iter(paths)
        .for_each_concurrent(limit, |path| {
            let pack_state = pack_state.clone();
            let app = app.clone();
            let dir = dir.clone();
            let encoder = encoder.clone();
            let upload_lock = upload_lock.clone();
            let cancel = cancel.clone();
            async move {
                if cancel.load(Ordering::Relaxed) {
                    let _ = app.emit("upload:file-done", ());
                    return;
                }
                // Hold read lock for duration of file processing so save can acquire
                // the write lock and run exclusively between file uploads.
                let _read_guard = upload_lock.read().await;
                match process_one_file(&pack_state, &path, &dir, encoder).await {
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
        })
        .await;

    let _ = app.emit("upload:done", ());
}

async fn process_one_file(
    pack_state: &crate::PackState,
    path: &Path,
    dir: &Path,
    encoder: HardwareEncoder,
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
    let output_path = dir.join("media").join(id.to_string());
    let path_owned = path.to_path_buf();

    let (tx, rx) = oneshot::channel();
    rayon::spawn(move || {
        let _ = tx.send(encode_file(&path_owned, &output_path, encoder));
    });

    let encoded = rx
        .await
        .map_err(|e| ProcessErrorKind::Other(e.into()))?
        .map_err(ProcessErrorKind::EncodeError)?;

    let encoded = match encoded {
        Some(e) => e,
        None => return Ok(None),
    };

    let mut lock = pack_state.lock().await;
    if let Some(pack) = lock.as_mut() {
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
    } else {
        Err(ProcessErrorKind::Other(anyhow!("Pack was closed")))
    }
}
