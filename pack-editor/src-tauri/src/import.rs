use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{Arc, Mutex},
    thread::available_parallelism,
};

use anyhow::{anyhow, bail, Context};
use converter::{convert, ConversionOutput, ConvertedMedia, ZipSource};
use futures::{stream, StreamExt};
use shared::behaviour::MediaSlot;
use shared::encode::{encode_file, hash_file, HardwareEncoder};
use tauri::Emitter;
use tempfile::TempDir;
use tokio::sync::{oneshot, watch, RwLock};
use uuid::Uuid;

use crate::encode::{wait_cancelled, DiscardOnDrop, UploadErrorPayload};
use crate::pack::{AddOutcome, MediaFile};

/// One slot `fill_slots_for` filled in, as the `import:slots-filled` payload carries it.
#[derive(Debug, Clone, serde::Serialize)]
struct FilledSlot {
    slot: MediaSlot,
    media_id: u64,
}

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

/// Opens an Edgeware `.zip` and runs the converter against it.
pub fn load(source_path: &Path) -> anyhow::Result<(ZipSource, ConversionOutput)> {
    let source = ZipSource::open(source_path)
        .with_context(|| format!("opening {} as a zip archive", source_path.display()))?;
    if !source.has_pack_marker() {
        bail!(
            "{} doesn't contain an Edgeware pack -- no recognized pack files or directories \
             were found in it",
            source_path.display()
        );
    }
    let output = convert(&source);

    Ok((source, output))
}

/// Streams `media` into the pack open in `pack_state`.
#[allow(clippy::too_many_arguments)]
pub async fn run_import(
    pack_state: crate::PackState,
    source: Arc<ZipSource>,
    media: Vec<ConvertedMedia>,
    media_references: Vec<(MediaSlot, String)>,
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
    let total = media.len();
    let _ = app.emit("upload:start", serde_json::json!({ "total": total }));

    let limit = available_parallelism().map(|x| x.get()).ok();

    let pending = Arc::new(Mutex::new(group_media_references(media_references)));

    let process_all = stream::iter(media).for_each_concurrent(limit, |item| {
        let pack_state = pack_state.clone();
        let app = app.clone();
        let dir = dir.clone();
        let encoder = encoder.clone();
        let source = source.clone();
        let staging = staging.clone();
        let pending = pending.clone();

        async move {
            let source_path = item.source_path.clone();
            let suggested_name = item.suggested_name.clone();
            match import_one_media(&pack_state, pack_id, source, item, &dir, &staging, encoder)
                .await
            {
                Ok(Some(media_file)) => {
                    let slots = pending
                        .lock()
                        .unwrap()
                        .remove(&suggested_name)
                        .unwrap_or_default();

                    let _ = app.emit("upload:added", &media_file);

                    if !slots.is_empty() {
                        match fill_slots_for(&pack_state, pack_id, slots, media_file.id).await {
                            Ok(filled) if !filled.is_empty() => {
                                let _ = app.emit("import:slots-filled", &filled);
                            }
                            Ok(_) => {}
                            Err(error) => {
                                let _ = app.emit(
                                    "upload:error",
                                    UploadErrorPayload::new(
                                        &dir,
                                        format!(
                                            "Could not fill in the pack's media slots: {error}"
                                        ),
                                    ),
                                );
                            }
                        }
                    }
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

    tokio::select! {
        _ = &mut process_all => {}
        _ = wait_cancelled(&mut cancel) => {}
    }

    let _ = app.emit("upload:done", ());
}

fn group_media_references(references: Vec<(MediaSlot, String)>) -> HashMap<String, Vec<MediaSlot>> {
    let mut grouped: HashMap<String, Vec<MediaSlot>> = HashMap::new();
    for (slot, name) in references {
        grouped.entry(name).or_default().push(slot);
    }
    grouped
}

/// Resolves media slots (wallpaper, splash, etc) waiting for an imported file.
async fn fill_slots_for(
    pack_state: &crate::PackState,
    pack_id: Uuid,
    slots: Vec<MediaSlot>,
    media_id: u64,
) -> anyhow::Result<Vec<FilledSlot>> {
    let lock = pack_state.lock().await;
    let Some(pack) = lock.as_ref().filter(|pack| pack.id() == pack_id) else {
        return Ok(Vec::new());
    };

    let filled = pack
        .fill_empty_slots_from_conversion(slots, media_id)
        .await?;

    Ok(filled
        .into_iter()
        .map(|slot| FilledSlot { slot, media_id })
        .collect())
}

// As in `encode::process_one_file`: one media item, plus the batch-wide context resolved once
// by the caller.
#[allow(clippy::too_many_arguments)]
async fn import_one_media(
    pack_state: &crate::PackState,
    pack_id: Uuid,
    source: Arc<ZipSource>,
    media: ConvertedMedia,
    dir: &Path,
    staging: &Arc<TempDir>,
    encoder: HardwareEncoder,
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
        .add_file_to_import(encoded, Path::new(&media.suggested_name), hash, None)
        .await
        .map_err(ImportErrorKind::PackError)?;
    // A bulk import skips a file the pack already has -- the copy that's here is as good, and a
    // second one would only be clutter. (A slot fill answers `Duplicate` differently; see
    // `AddOutcome`.)
    let mut media_file = match added.and_then(AddOutcome::into_added) {
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use shared::behaviour::Content;

    use super::*;

    fn source_zip(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = std::fs::File::create(dir.path().join("pack.zip")).expect("create zip");
        let mut archive = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for &(name, contents) in files {
            archive.start_file(name, options).expect("start zip file");
            archive
                .write_all(contents.as_bytes())
                .expect("write zip file");
        }
        archive.finish().expect("finish zip");
        dir
    }

    fn load_test_zip(dir: &tempfile::TempDir) -> anyhow::Result<(ZipSource, ConversionOutput)> {
        load(&dir.path().join("pack.zip"))
    }

    /// The bug this guards: a source that holds no pack converted to nothing *successfully*, so
    /// the import went on to create a new, empty, untitled pack -- with nothing to tell the author
    /// their file had never been read. A clone of Edgeware with the pack nested several levels in
    /// looked exactly like this before `detect_root_prefix` learned to find it.
    #[test]
    fn a_source_holding_no_pack_fails_rather_than_importing_nothing() {
        let dir = source_zip(&[
            ("readme.txt", "not a pack"),
            ("src/main.py", "print('hi')"),
            ("assets/logo.png", "not media either, and not in img/"),
        ]);

        let error = match load_test_zip(&dir) {
            Err(error) => error,
            Ok(_) => panic!("a source with no pack must not import"),
        };
        assert!(
            error
                .to_string()
                .contains("doesn't contain an Edgeware pack"),
            "unhelpful error: {error}"
        );
    }

    /// Marker validation must not tighten what counts as a pack: media alone, with no JSON of
    /// any kind, is a perfectly ordinary Edgeware pack.
    #[test]
    fn a_pack_of_bare_media_still_imports() {
        let dir = source_zip(&[("img/a.png", "image bytes"), ("aud/b.mp3", "audio bytes")]);

        let (_source, output) =
            load_test_zip(&dir).unwrap_or_else(|e| panic!("bare media is a pack: {e}"));
        assert_eq!(output.media.len(), 2);
    }

    /// A file arriving has to claim *every* slot waiting on it, not just the first: Edgeware packs
    /// routinely point a corruption level at the pack's primary wallpaper, and that file is
    /// imported once. Grouping by name is what lets one arrival fill them all in a single write.
    #[test]
    fn slot_references_group_by_the_file_they_wait_on() {
        let grouped = group_media_references(vec![
            (MediaSlot::Wallpaper, "wallpaper.png".to_string()),
            (MediaSlot::Splash, "loading_splash.png".to_string()),
            (
                MediaSlot::StageWallpaper {
                    stage: "stage-2".to_string(),
                },
                "wallpaper.png".to_string(),
            ),
        ]);

        assert_eq!(grouped.len(), 2);
        assert_eq!(
            grouped["wallpaper.png"],
            vec![
                MediaSlot::Wallpaper,
                MediaSlot::StageWallpaper {
                    stage: "stage-2".to_string()
                }
            ]
        );
        assert_eq!(grouped["loading_splash.png"], vec![MediaSlot::Splash]);
    }

    /// And neither must it reject a pack whose content is entirely text -- captions with no media
    /// convert to a real behaviour document, even though nothing will be imported alongside it.
    #[test]
    fn a_pack_of_only_text_content_still_imports() {
        let dir = source_zip(&[("captions.json", r#"{"default": ["a caption"]}"#)]);

        let (_source, output) =
            load_test_zip(&dir).unwrap_or_else(|e| panic!("text-only content is a pack: {e}"));
        assert!(output.media.is_empty());
        assert!(!output.behaviour.content.captions.is_empty());
    }

    /// `info.json` is an Edgeware pack path even when there is nothing for the content converter
    /// to emit. Conversion output is therefore not a reliable format probe.
    #[test]
    fn a_metadata_only_pack_still_imports() {
        let dir = source_zip(&[("info.json", r#"{"name": "Empty on purpose"}"#)]);

        let (_source, output) =
            load_test_zip(&dir).unwrap_or_else(|e| panic!("metadata-only pack is valid: {e}"));
        assert!(output.media.is_empty());
        assert_eq!(output.behaviour.content, Content::default());
        assert_eq!(output.metadata.name, "Empty on purpose");
    }

    /// Edgeware's own importer treats the standard media directories as pack markers even when
    /// they are empty.
    #[test]
    fn an_empty_media_directory_is_still_a_pack_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = std::fs::File::create(dir.path().join("pack.zip")).expect("create zip");
        let mut archive = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        archive
            .add_directory("img/", options)
            .expect("add img directory");
        archive.finish().expect("finish zip");

        let (_source, output) =
            load_test_zip(&dir).unwrap_or_else(|e| panic!("empty img directory is a marker: {e}"));
        assert!(output.media.is_empty());
        assert_eq!(output.behaviour.content, Content::default());
    }
}
