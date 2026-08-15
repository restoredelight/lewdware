use std::{
    future::Future,
    io,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    thread::available_parallelism,
};

use anyhow::{anyhow, Context};
use futures::{stream, StreamExt};
use infer::MatcherType;
use serde::Serialize;
use shared::encode::{encode_file, hash_file, HardwareEncoder};
use tempfile::TempDir;
use tokio::sync::{oneshot, watch, RwLock, Semaphore};
use uuid::Uuid;
use walkdir::WalkDir;

use tauri::Emitter;

use shared::behaviour::{Behaviour, MediaSlot};

use crate::pack::{AddOutcome, MediaFile};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct UploadErrorPayload {
    path: String,
    file_name: String,
    error: String,
}

impl UploadErrorPayload {
    pub(crate) fn new(path: &Path, error: impl Into<String>) -> Self {
        let file_name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        Self {
            path: path.to_string_lossy().into_owned(),
            file_name,
            error: error.into(),
        }
    }
}

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

pub(crate) fn encode_semaphore() -> &'static Semaphore {
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

pub fn explore_folder(path: &Path) -> Vec<PathBuf> {
    WalkDir::new(path)
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
    // Close, discard and identity-changing Save As operations take the write side. Holding this
    // for the complete batch gives those operations a reliable way to cancel and drain imports,
    // including their pending history entry, rather than merely waiting for the final DB commit.
    let _batch_guard = upload_lock.read().await;
    let total = paths.len();
    let _ = app.emit("upload:start", serde_json::json!({ "total": total }));

    let (dir, pack_id) = {
        let lock = pack_state.lock().await;
        match lock.as_ref() {
            Some(pack) => (pack.dir().to_path_buf(), pack.id()),
            None => {
                let _ = app.emit("upload:done", ());
                return;
            }
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
                UploadErrorPayload::new(
                    &dir,
                    format!("Could not create upload staging directory: {error}"),
                ),
            );
            let _ = app.emit("upload:done", ());
            return;
        }
    };
    let history_id = {
        let lock = pack_state.lock().await;
        match lock.as_ref().filter(|pack| pack.id() == pack_id) {
            Some(pack) => match pack.begin_media_import().await {
                Ok(id) => id,
                Err(error) => {
                    let _ = app.emit(
                        "upload:error",
                        UploadErrorPayload::new(&dir, format!("Could not begin import: {error}")),
                    );
                    let _ = app.emit("upload:done", ());
                    return;
                }
            },
            None => {
                let _ = app.emit("upload:done", ());
                return;
            }
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
                history_id,
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
                        UploadErrorPayload::new(&path, err.to_string()),
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

    {
        let lock = pack_state.lock().await;
        if let Some(pack) = lock.as_ref().filter(|pack| pack.id() == pack_id) {
            if let Err(error) = pack.finish_media_import(history_id, None).await {
                let _ = app.emit(
                    "upload:error",
                    UploadErrorPayload::new(
                        &dir,
                        format!("Could not finalize import history: {error}"),
                    ),
                );
            }
        }
    }

    let _ = app.emit("upload:done", ());
}

/// Brings in one file and puts it in `slot`, as a single action.
///
/// Same pipeline as `process_files`, minus the batch: one staging directory, no cancellation (a
/// single encode is short enough that "cancel" means closing the dialog). The `upload:added`
/// event still fires, so the media grid picks the file up exactly as it would from any other add.
/// A file the pack already has comes back as `AddOutcome::Duplicate` rather than an error -- for
/// a slot that's a perfectly good answer.
///
/// The slot write happens *inside* the import's own history entry rather than after it, so the
/// whole thing is one undo step: "set the wallpaper", not "import a file" and then "set the
/// wallpaper" (see `history::record_into_pending`).
pub async fn process_file_into_slot(
    pack_state: &crate::PackState,
    path: &Path,
    app: &tauri::AppHandle,
    encoder: HardwareEncoder,
    upload_lock: &Arc<RwLock<()>>,
    slot: MediaSlot,
    label: String,
) -> anyhow::Result<(AddOutcome, Behaviour, Option<u64>)> {
    let _batch_guard = upload_lock.read().await;
    let (dir, pack_id) = {
        let lock = pack_state.lock().await;
        let pack = lock.as_ref().context("No pack is open")?;
        (pack.dir().to_path_buf(), pack.id())
    };
    let staging = Arc::new(
        tempfile::Builder::new()
            .prefix("pack-editor-slot-")
            .tempdir_in(&dir)
            .context("Could not create upload staging directory")?,
    );
    let history_id = {
        let lock = pack_state.lock().await;
        let pack = lock
            .as_ref()
            .filter(|pack| pack.id() == pack_id)
            .context("No pack is open")?;
        pack.begin_media_import().await?
    };

    let outcome = process_one_file_outcome(
        pack_state,
        pack_id,
        path,
        &dir,
        &staging,
        encoder,
        upload_lock,
        history_id,
    )
    .await
    .map_err(|err| anyhow!("{err}"));

    // The import entry has to be finished either way -- an abandoned pending entry blocks undo
    // for everything after it -- so the slot write and the finish are both guarded rather than
    // short-circuited on failure.
    let filled = match &outcome {
        Ok(outcome) => {
            let lock = pack_state.lock().await;
            match lock.as_ref().filter(|pack| pack.id() == pack_id) {
                Some(pack) => {
                    let addition = crate::pack::MediaAddition::from(outcome);
                    pack.fill_media_slot_during_import(
                        slot,
                        addition.id,
                        history_id,
                        addition.new_to_pack,
                    )
                    .await
                    .map(Some)
                }
                None => Ok(None),
            }
        }
        Err(_) => Ok(None),
    };

    {
        let lock = pack_state.lock().await;
        if let Some(pack) = lock.as_ref().filter(|pack| pack.id() == pack_id) {
            pack.finish_media_import(history_id, Some(label)).await?;
        }
    }

    let outcome = outcome?;
    let (behaviour, displaced) = filled?.context("The pack was closed while importing")?;
    if let AddOutcome::Added(file) = &outcome {
        let _ = app.emit("upload:added", file);
    }
    Ok((outcome, behaviour, displaced))
}

/// Imports several files straight into the subliminal pool, as a single action.
///
/// The batch shares one history entry -- filling a pool is one thing the author did, not eight --
/// so the pool tagging is folded into the same pending entry the imports accumulate into (see
/// `history::record_into_pending`), exactly as `process_file_into_slot` does for one file.
///
/// Sequential rather than concurrent: a handful of images is not the bulk uploader's problem, and
/// keeping it in order means the returned outcomes line up with `paths` for reporting.
pub async fn process_files_into_subliminals(
    pack_state: &crate::PackState,
    paths: &[std::path::PathBuf],
    app: &tauri::AppHandle,
    encoder: HardwareEncoder,
    upload_lock: &Arc<RwLock<()>>,
) -> anyhow::Result<Vec<AddOutcome>> {
    let _batch_guard = upload_lock.read().await;
    let (dir, pack_id) = {
        let lock = pack_state.lock().await;
        let pack = lock.as_ref().context("No pack is open")?;
        (pack.dir().to_path_buf(), pack.id())
    };
    let staging = Arc::new(
        tempfile::Builder::new()
            .prefix("pack-editor-pool-")
            .tempdir_in(&dir)
            .context("Could not create upload staging directory")?,
    );
    let history_id = {
        let lock = pack_state.lock().await;
        let pack = lock
            .as_ref()
            .filter(|pack| pack.id() == pack_id)
            .context("No pack is open")?;
        pack.begin_media_import().await?
    };

    let mut outcomes = Vec::new();
    let mut failure = None;
    for path in paths {
        match process_one_file_outcome(
            pack_state,
            pack_id,
            path,
            &dir,
            &staging,
            encoder.clone(),
            upload_lock,
            history_id,
        )
        .await
        {
            Ok(outcome) => {
                if let AddOutcome::Added(file) = &outcome {
                    let _ = app.emit("upload:added", file);
                }
                outcomes.push(outcome);
            }
            // One unreadable file shouldn't cost the author the rest of the batch; it is reported
            // and the others still land.
            Err(err) => {
                let _ = app.emit(
                    "upload:error",
                    UploadErrorPayload::new(path, err.to_string()),
                );
                failure = Some(err);
            }
        }
    }

    let additions: Vec<crate::pack::MediaAddition> = outcomes.iter().map(Into::into).collect();
    let tagged = {
        let lock = pack_state.lock().await;
        match lock.as_ref().filter(|pack| pack.id() == pack_id) {
            Some(pack) => {
                pack.add_to_subliminals_during_import(additions, history_id)
                    .await
            }
            None => Ok(()),
        }
    };

    // The pending entry has to be closed either way -- an abandoned one blocks undo for
    // everything after it.
    let label = match outcomes.len() {
        1 => "Add subliminal".to_string(),
        n => format!("Add {n} subliminals"),
    };
    {
        let lock = pack_state.lock().await;
        if let Some(pack) = lock.as_ref().filter(|pack| pack.id() == pack_id) {
            pack.finish_media_import(history_id, Some(label)).await?;
        }
    }
    tagged?;

    match failure {
        Some(err) if outcomes.is_empty() => Err(anyhow!("{err}")),
        _ => Ok(outcomes),
    }
}

// The per-file arguments are one path; the rest is the loop-invariant context every file in a
// batch shares, which the caller resolves once before the stream starts.
#[allow(clippy::too_many_arguments)]
async fn process_one_file(
    pack_state: &crate::PackState,
    pack_id: Uuid,
    path: &Path,
    dir: &Path,
    staging: &Arc<TempDir>,
    encoder: HardwareEncoder,
    upload_lock: &RwLock<()>,
    history_id: i64,
) -> Result<Option<MediaFile>, ProcessErrorKind> {
    match process_one_file_outcome(
        pack_state,
        pack_id,
        path,
        dir,
        staging,
        encoder,
        upload_lock,
        history_id,
    )
    .await?
    {
        AddOutcome::Added(file) => Ok(Some(file)),
        // A bulk add treats a file the pack already has as a skip: the copy that's here is as
        // good, and a second one would only be clutter.
        AddOutcome::Duplicate(_) => Err(ProcessErrorKind::Skipped),
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_one_file_outcome(
    pack_state: &crate::PackState,
    pack_id: Uuid,
    path: &Path,
    dir: &Path,
    staging: &Arc<TempDir>,
    encoder: HardwareEncoder,
    _upload_lock: &RwLock<()>,
    history_id: i64,
) -> Result<AddOutcome, ProcessErrorKind> {
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
            if let Some(existing) = pack
                .file_with_hash(&hash)
                .await
                .map_err(ProcessErrorKind::Other)?
            {
                return Ok(AddOutcome::Duplicate(existing));
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
        .add_file_to_import(encoded, path, hash, Some(history_id))
        .await
        .map_err(ProcessErrorKind::PackError)?;
    match media {
        // The pre-check above already handles the common case; this only
        // fires when a race let a since-inserted duplicate through, and the
        // DB's own uniqueness constraint caught it.
        Some(outcome) => Ok(outcome),
        None => Err(ProcessErrorKind::Skipped),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::UploadErrorPayload;

    #[test]
    fn upload_errors_include_the_file_name_and_full_path() {
        let path = Path::new("media").join("imports").join("clip.mp4");
        let payload = UploadErrorPayload::new(&path, "failed");
        assert_eq!(payload.file_name, "clip.mp4");
        assert_eq!(payload.path, path.to_string_lossy());
    }
}
