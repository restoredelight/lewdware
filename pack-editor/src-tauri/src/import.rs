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
//!
//! The wallpaper/splash slots are the exception, because they reference media by *name* and a
//! name isn't final until its file has arrived (a collision suffixes it; a duplicate or a failed
//! encode means it never arrives at all). Those are left out of that first write and filled in by
//! `fill_slots_for` the moment the file each one names lands -- per file, not once at the end, so
//! a slot waits only on its own media rather than on the whole (potentially very long) pipeline.
//! The pack still never carries a reference to a file it doesn't have: a name is only written
//! once that exact file is in the pack. Whatever is still unclaimed when the pipeline ends named
//! a file that never arrived, and stays empty.

use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{Arc, Mutex},
    thread::available_parallelism,
};

use anyhow::{anyhow, bail, Context};
use converter::{convert, ConversionOutput, ConvertedMedia, DirSource, PackSource, ZipSource};
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

    // Nothing recognizable was found. The converter has no way to distinguish "an Edgeware pack
    // that happens to be empty" from "not an Edgeware pack" -- both simply produce nothing -- so
    // this is where that judgement belongs, and it has to be a failure: succeeding leaves the
    // author looking at a new, empty, untitled pack with no hint that their file was never read.
    if output.media.is_empty() && output.behaviour.content.is_empty() {
        bail!(
            "{} doesn't contain an Edgeware pack -- no img/vid/aud media and no pack JSON was \
             found in it",
            source_path.display()
        );
    }

    Ok((source, output))
}

/// Streams `media` into the pack open in `pack_state`. Reuses the `upload:*` event names normal
/// uploads emit (`start`/`added`/`error`/`file-done`/`done`) -- from the frontend's perspective,
/// importing an Edgeware pack's media *is* a bulk upload; the only Edgeware-specific artifact (the
/// converter's warnings) is returned directly from the Tauri command instead, since conversion
/// itself already finished by the time this task starts (as did the behaviour.json/metadata write
/// -- see this module's own doc comment).
#[allow(clippy::too_many_arguments)]
pub async fn run_import(
    pack_state: crate::PackState,
    source: Arc<dyn PackSource + Send + Sync>,
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

    // The slot references held back from the behaviour write, keyed by the converted file name
    // each one is waiting on (see `fill_slots_for`). Claimed and removed as files land -- one
    // file can carry several slots (a stage wallpaper reusing the pack's primary), and whatever
    // is still here at the end named a file that never arrived.
    let pending = Arc::new(Mutex::new(group_media_references(media_references)));

    // Serializes the behaviour document's read-modify-write across the concurrent import tasks:
    // two referenced files landing at once would otherwise both read the document as it was
    // before either fill, and the second write would drop the first one's slot.
    let behaviour_write = Arc::new(tokio::sync::Mutex::new(()));

    let process_all = stream::iter(media).for_each_concurrent(limit, |item| {
        let pack_state = pack_state.clone();
        let app = app.clone();
        let dir = dir.clone();
        let encoder = encoder.clone();
        let upload_lock = upload_lock.clone();
        let source = source.clone();
        let staging = staging.clone();
        let pending = pending.clone();
        let behaviour_write = behaviour_write.clone();
        async move {
            let source_path = item.source_path.clone();
            let suggested_name = item.suggested_name.clone();
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
                    let slots = pending
                        .lock()
                        .unwrap()
                        .remove(&suggested_name)
                        .unwrap_or_default();
                    // Before the slot fill, so the front end has the file in hand by the time a
                    // slot points at it -- a slot naming media the media grid hasn't heard of
                    // renders as missing until the next refresh.
                    let _ = app.emit("upload:added", &media_file);
                    if !slots.is_empty() {
                        match fill_slots_for(
                            &pack_state,
                            pack_id,
                            &behaviour_write,
                            slots,
                            media_file.id,
                        )
                        .await
                        {
                            // The front end fetched this document at the start of the import (see
                            // `Start.svelte`), so its copy predates this write and nothing else
                            // would ever tell it -- the slot stayed empty in the editor until the
                            // pack was reopened.
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

    // Dropping the stream is the cancellation -- see encode::process_files.
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

/// The slot references `import_edgeware_pack_dialog` held back, indexed by the converted file
/// name each one waits on, so an arriving file can claim its slots with one lookup.
///
/// Several slots can name the same file -- Edgeware packs routinely reuse the pack's primary
/// wallpaper as a corruption level's, and `resolve_wallpaper` imports it once -- so this is a
/// name to *many* slots, not a name to one.
fn group_media_references(references: Vec<(MediaSlot, String)>) -> HashMap<String, Vec<MediaSlot>> {
    let mut grouped: HashMap<String, Vec<MediaSlot>> = HashMap::new();
    for (slot, name) in references {
        grouped.entry(name).or_default().push(slot);
    }
    grouped
}

/// Points the media slots (wallpaper/splash -- see `shared::behaviour::Content`) waiting on a
/// just-imported file at the id it actually landed under.
///
/// Called as each referenced file arrives rather than once at the end, so the Content and
/// Experience tabs fill in a slot as soon as its own media is really in the pack, instead of
/// showing an empty one until the last of the pack's (potentially thousands of) files has been
/// encoded. `prioritize_referenced_media` puts these files first in the import order for exactly
/// this reason.
///
/// A slot whose file never arrives -- a duplicate, a failed encode, a cancelled import -- is
/// simply never filled, which is the honest answer: `pack_has_wallpaper`/`pack_has_splash` then
/// report the feature as absent rather than offering a toggle with nothing behind it.
///
/// Returns the slots that were actually filled, so the caller can tell the front end about a
/// write it has no other way to learn about.
async fn fill_slots_for(
    pack_state: &crate::PackState,
    pack_id: Uuid,
    behaviour_write: &tokio::sync::Mutex<()>,
    slots: Vec<MediaSlot>,
    media_id: u64,
) -> anyhow::Result<Vec<FilledSlot>> {
    let _write_guard = behaviour_write.lock().await;
    let lock = pack_state.lock().await;
    let Some(pack) = lock.as_ref().filter(|pack| pack.id() == pack_id) else {
        return Ok(Vec::new());
    };
    // Fill, not set: a slot the author has already pointed somewhere keeps their choice. The
    // converted document named a file, and this is the moment that file arrives -- but the author
    // may have picked their own in the meantime, and the import must not overrule them.
    //
    // One targeted write per slot rather than a read-modify-write of the whole document: a
    // document read while the timeline is switched off has no stages in it, so writing it back
    // would delete them.
    let filled = pack
        .fill_empty_slots(slots, media_id, "Set media slot".to_string())
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
    use super::*;

    fn source_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, contents) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create parent");
            }
            std::fs::write(path, contents).expect("write fixture");
        }
        dir
    }

    /// The bug this guards: a source that holds no pack converted to nothing *successfully*, so
    /// the import went on to create a new, empty, untitled pack -- with nothing to tell the author
    /// their file had never been read. A clone of Edgeware with the pack nested several levels in
    /// looked exactly like this before `detect_root_prefix` learned to find it.
    #[test]
    fn a_source_holding_no_pack_fails_rather_than_importing_nothing() {
        let dir = source_dir(&[
            ("readme.txt", "not a pack"),
            ("src/main.py", "print('hi')"),
            ("assets/logo.png", "not media either, and not in img/"),
        ]);

        let error = match load(dir.path()) {
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

    /// The emptiness check must not tighten what counts as a pack: media alone, with no JSON of
    /// any kind, is a perfectly ordinary Edgeware pack.
    #[test]
    fn a_pack_of_bare_media_still_imports() {
        let dir = source_dir(&[("img/a.png", "image bytes"), ("aud/b.mp3", "audio bytes")]);

        let (_source, output) =
            load(dir.path()).unwrap_or_else(|e| panic!("bare media is a pack: {e}"));
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
        let dir = source_dir(&[("captions.json", r#"{"default": ["a caption"]}"#)]);

        let (_source, output) =
            load(dir.path()).unwrap_or_else(|e| panic!("text-only content is a pack: {e}"));
        assert!(output.media.is_empty());
        assert!(!output.behaviour.content.is_empty());
    }
}
