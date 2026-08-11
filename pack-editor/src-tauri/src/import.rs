//! "Import Edgeware pack" -- runs the `converter` crate against a source pack (directory or
//! `.zip`) and streams the result into a freshly-created `MediaPack`, re-encoding media via the
//! same ffmpeg sidecars/hardware encoder normal uploads use. See
//! `behaviour-design/edgeware-compat.md` (the pack editor is the sole product conversion front
//! end) and `design/release-plan.md`'s M6 bullet.
//!
//! Mirrors `encode.rs`'s `process_files`/`process_one_file` shape closely -- same
//! extract-hash-encode-insert pipeline (hashing the extracted source, not the encoded output --
//! see `import_one_media`'s own comment), just pulling input bytes from a `PackSource` instead of
//! an already-local path. behaviour.json/metadata are written synchronously in the
//! `import_edgeware_pack_dialog` command itself (`lib.rs`), before this module's media pipeline
//! ever starts: they're keyed by tag, not by any specific media file's id, so they have no actual
//! dependency on encoding having finished -- waiting for the whole (potentially large) media
//! pipeline before the Content/Experience tabs had anything to show was an artificial delay, not a
//! real one.

use std::{io, path::Path, sync::Arc, thread::available_parallelism};

use anyhow::{anyhow, Context};
use converter::{convert, ConversionOutput, ConvertedMedia, DirSource, PackSource, ZipSource};
use futures::{stream, StreamExt};
use shared::encode::{encode_file, hash_file, HardwareEncoder};
use tauri::Emitter;
use tempfile::TempDir;
use tokio::sync::{oneshot, watch, RwLock};
use uuid::Uuid;

use crate::encode::{wait_cancelled, DiscardOnDrop, UploadErrorPayload};
use crate::pack::MediaFile;

#[derive(Debug)]
enum ImportErrorKind {
    Skipped,
    Unrecognized,
    ExtractError(io::Error),
    EncodeError(anyhow::Error),
    HashError(io::Error),
    PackError(anyhow::Error),
    Other(anyhow::Error),
}

impl std::fmt::Display for ImportErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Skipped => write!(f, "Duplicate (skipped)"),
            Self::Unrecognized => write!(f, "Not a recognizable image/video/audio file"),
            Self::ExtractError(e) => write!(f, "Couldn't read from the source pack: {e}"),
            Self::EncodeError(e) => write!(f, "Encode error: {e}"),
            Self::HashError(e) => write!(f, "Hash error: {e}"),
            Self::PackError(e) => write!(f, "Pack error: {e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

/// Opens `source_path` as a `PackSource` (a directory or a `.zip` archive, going by whether the
/// path is a directory) and runs the converter against it. Blocking (archive open, JSON/dir
/// reads) -- callers should run this inside `spawn_blocking`. Kept synchronous and separate from
/// the media pipeline below so a bad source (e.g. an unreadable zip) fails the Tauri command
/// immediately, before any pack has been created or dialog shown for a destination.
pub fn load(
    source_path: &Path,
) -> anyhow::Result<(Box<dyn PackSource + Send + Sync>, ConversionOutput)> {
    let source: Box<dyn PackSource + Send + Sync> = if source_path.is_dir() {
        Box::new(DirSource::new(source_path))
    } else {
        Box::new(
            ZipSource::open(source_path)
                .with_context(|| format!("opening {} as a zip archive", source_path.display()))?,
        )
    };
    let output = convert(source.as_ref());
    Ok((source, output))
}

/// Streams `media` into the pack open in `pack_state`. Reuses the `upload:*` event names normal
/// uploads emit (`start`/`added`/`error`/`file-done`/`done`) -- from the frontend's perspective,
/// importing an Edgeware pack's media *is* a bulk upload; the only Edgeware-specific artifact (the
/// converter's warnings) is returned directly from the Tauri command instead, since conversion
/// itself already finished by the time this task starts (as did the behaviour.json/metadata write
/// -- see this module's own doc comment).
pub async fn run_import(
    pack_state: crate::PackState,
    source: Arc<dyn PackSource + Send + Sync>,
    media: Vec<ConvertedMedia>,
    app: tauri::AppHandle,
    encoder: HardwareEncoder,
    upload_lock: Arc<RwLock<()>>,
    mut cancel: watch::Receiver<bool>,
) {
    // See encode::process_files: lifecycle operations drain this whole batch, not just one
    // file's final commit, before replacing or deleting the pack working directory.
    let _batch_guard = upload_lock.read().await;
    let (dir, pack_id) = {
        let lock = pack_state.lock().await;
        match lock.as_ref() {
            Some(pack) => (pack.dir().to_path_buf(), pack.id()),
            None => return,
        }
    };
    let staging = match tempfile::Builder::new()
        .prefix("pack-editor-import-")
        .tempdir_in(&dir)
    {
        Ok(staging) => Arc::new(staging),
        Err(error) => {
            let _ = app.emit(
                "upload:error",
                UploadErrorPayload::new(
                    &dir,
                    format!("Could not create import staging directory: {error}"),
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
            None => return,
        }
    };

    let total = media.len();
    let _ = app.emit("upload:start", serde_json::json!({ "total": total }));

    let limit = available_parallelism().map(|x| x.get()).ok();

    let process_all = stream::iter(media).for_each_concurrent(limit, |item| {
        let pack_state = pack_state.clone();
        let app = app.clone();
        let dir = dir.clone();
        let encoder = encoder.clone();
        let upload_lock = upload_lock.clone();
        let source = source.clone();
        let staging = staging.clone();
        async move {
            let source_path = item.source_path.clone();
            match import_one_media(
                &pack_state,
                pack_id,
                source,
                item,
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
                Err(ImportErrorKind::Skipped) => {
                    let _ = app.emit("upload:skipped", &source_path);
                }
                Err(err) => {
                    let _ = app.emit(
                        "upload:error",
                        UploadErrorPayload::new(Path::new(&source_path), err.to_string()),
                    );
                }
            }
            let _ = app.emit("upload:file-done", ());
        }
    });
    tokio::pin!(process_all);

    // Dropping the stream is the cancellation -- see encode::process_files.
    tokio::select! {
        _ = &mut process_all => {}
        _ = wait_cancelled(&mut cancel) => {}
    }

    {
        let lock = pack_state.lock().await;
        if let Some(pack) = lock.as_ref().filter(|pack| pack.id() == pack_id) {
            if let Err(error) = pack.finish_media_import(history_id).await {
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

// As in `encode::process_one_file`: one media item, plus the batch-wide context resolved once
// by the caller.
#[allow(clippy::too_many_arguments)]
async fn import_one_media(
    pack_state: &crate::PackState,
    pack_id: Uuid,
    source: Arc<dyn PackSource + Send + Sync>,
    media: ConvertedMedia,
    dir: &Path,
    staging: &Arc<TempDir>,
    encoder: HardwareEncoder,
    _upload_lock: &RwLock<()>,
    history_id: i64,
) -> Result<Option<MediaFile>, ImportErrorKind> {
    let extract_path = staging.path().join(format!("source-{}", Uuid::new_v4()));
    let extract_dest = extract_path.clone();
    let extract_source_path = media.source_path.clone();
    let extract = tokio::task::spawn_blocking(move || {
        source.extract_file(&extract_source_path, &extract_dest)
    });
    DiscardOnDrop::new(extract, extract_path.clone(), staging.clone())
        .finish()
        .await
        .map_err(|e| ImportErrorKind::Other(e.into()))?
        .map_err(ImportErrorKind::ExtractError)?;

    // Hash the extracted source, not the encoded output: hashes are purely a dedup key, and
    // ffmpeg's output isn't guaranteed byte-for-byte deterministic (across runs, machines,
    // encoder settings), so hashing post-encode risks the same source producing different
    // hashes on different imports. Hashing here -- as encode::process_one_file also does --
    // additionally means a known duplicate can skip the extract's own encode+hash-of-output
    // work entirely instead of paying for it and then discarding the result.
    let hash_path = extract_path.clone();
    let hash = tokio::task::spawn_blocking(move || hash_file(&hash_path))
        .await
        .map_err(|e| ImportErrorKind::Other(e.into()))?
        .map_err(ImportErrorKind::HashError)?;

    // Duplicates are always rejected (add_file_to_import enforces this with a DB-level
    // constraint) -- checking here just avoids wasting an encode on a file we already know
    // will be rejected. See encode::process_one_file's identical check.
    {
        let lock = pack_state.lock().await;
        if let Some(pack) = lock.as_ref() {
            if pack
                .check_hash(&hash)
                .await
                .map_err(ImportErrorKind::Other)?
            {
                let _ = std::fs::remove_file(&extract_path);
                return Err(ImportErrorKind::Skipped);
            }
        }
    }

    // Matches encode::process_one_file's own gating: without it, this path's outer
    // for_each_concurrent(available_parallelism()) is the only limit on how many `encode_file`
    // calls run at once, which -- combined with avifenc's own default of using every core per
    // invocation -- oversubscribes far worse than the upload path's tighter semaphore does.
    let _permit = crate::encode::encode_semaphore()
        .acquire()
        .await
        .map_err(|e| ImportErrorKind::Other(anyhow!("{e}")))?;

    let output_path = staging.path().join(Uuid::new_v4().to_string());
    let encode_input = extract_path.clone();

    let (tx, rx) = oneshot::channel();
    let encode_output = output_path.clone();
    rayon::spawn(move || {
        let result = encode_file(&encode_input, &encode_output, encoder);
        let _ = std::fs::remove_file(&encode_input);
        let _ = tx.send(result);
    });

    let encoded = DiscardOnDrop::new(rx, output_path, staging.clone())
        .finish()
        .await
        .map_err(|e| ImportErrorKind::Other(e.into()))?
        .map_err(ImportErrorKind::EncodeError)?;
    let mut encoded = match encoded {
        Some(e) => e,
        None => return Err(ImportErrorKind::Unrecognized),
    };

    let mut lock = pack_state.lock().await;
    let pack = lock
        .as_mut()
        .filter(|pack| pack.id() == pack_id)
        .ok_or_else(|| ImportErrorKind::Other(anyhow!("The pack was closed while encoding")))?;
    let final_path = dir.join("media").join(
        encoded
            .path
            .file_name()
            .ok_or_else(|| ImportErrorKind::Other(anyhow!("Encoded file has no name")))?,
    );
    tokio::fs::rename(&encoded.path, &final_path)
        .await
        .map_err(|e| ImportErrorKind::Other(e.into()))?;
    encoded.path = final_path;
    let added = pack
        .add_file_to_import(
            encoded,
            Path::new(&media.suggested_name),
            hash,
            Some(history_id),
        )
        .await
        .map_err(ImportErrorKind::PackError)?;
    let mut media_file = match added {
        Some(m) => m,
        None => return Err(ImportErrorKind::Skipped),
    };

    if !media.tags.is_empty() {
        pack.add_tags(media_file.id, media.tags.clone())
            .await
            .map_err(ImportErrorKind::PackError)?;
        media_file.tags = media.tags;
    }

    Ok(Some(media_file))
}
