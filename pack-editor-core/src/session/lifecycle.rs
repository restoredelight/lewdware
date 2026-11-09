//! `Session` methods for pack metadata and modes, plus what the shell needs around a session to
//! bring media in: [`Imports`] (running and cancelling import pipelines against the open pack),
//! [`ImportProgress`] (folding their events into a readout) and [`create_from_edgeware`].
//!
//! Each `Session` method is one line over [`Session::write`](super::Session) and a function in
//! `crate::pack::ops::lifecycle`.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use converter::{ConvertedMedia, ZipSource};
use shared::behaviour::MediaSlot;
use shared::encode::HardwareEncoder;
use shared::pack::Metadata;
use tokio::sync::{watch, Mutex, OwnedRwLockWriteGuard, RwLock};

use super::{Applied, Session, WriteKind};
use crate::changes::Scope;
use crate::encode::{self, UploadErrorPayload};
use crate::events::{Events, ImportEvent};
use crate::import;
use crate::pack::ops::lifecycle::{self as ops, EmbeddedMode, MetadataField};
use crate::pack::MediaPack;
use crate::PackState;

impl Session {
    /// The pack's name, creator, description, version and recommended mode.
    pub fn metadata(&self) -> Metadata {
        self.pack.metadata()
    }

    /// Sets one metadata field: one undo entry, none if the value is unchanged.
    pub fn set_metadata(&mut self, field: MetadataField) -> Result<Applied<bool>> {
        self.write_metadata(|c| ops::set_metadata(c, field))
    }

    /// The modes bundled with the pack. Reads every mode file, so views keep the result until
    /// the `Modes` scope changes.
    pub fn modes(&self) -> Result<Vec<EmbeddedMode>> {
        self.read(ops::modes)
    }

    pub fn add_mode(&mut self, bytes: Vec<u8>) -> Result<Applied<EmbeddedMode>> {
        self.write(WriteKind::Edit, |c| ops::add_mode(c, bytes))
    }

    /// Removes a mode, and the recommendation with it if it pointed there.
    pub fn remove_mode(&mut self, id: u64) -> Result<Applied<()>> {
        self.write_metadata(|c| ops::remove_mode(c, id))
    }

    /// A write that may change metadata, which `MediaPack` also caches.
    fn write_metadata<T>(
        &mut self,
        op: impl FnOnce(&rusqlite::Connection) -> Result<T>,
    ) -> Result<Applied<T>> {
        let applied = self.write(WriteKind::Edit, op)?;
        if applied.changes.scopes.contains(Scope::Metadata) {
            ops::sync_metadata(&self.pack, &self.connection)?;
        }
        Ok(applied)
    }
}

// ── Imports ─────────────────────────────────────────────────────────────────────────────────

/// The import pipelines for one open pack: starting them, cancelling them, and waiting for them
/// to stop writing before the pack is closed, discarded or saved somewhere new.
///
/// The pipelines (`encode::process_files`, `import::run_import`) write on pooled connections from
/// tokio workers and report through [`Events`]; feed those to [`Session::import_event`] and
/// [`ImportProgress::apply`] on the UI thread.
pub struct Imports {
    pack: PackState,
    /// Each running batch holds the read side; [`Self::stop`] and [`Self::exclusive`] take the
    /// write side, which waits for every batch (and its history entry) to finish.
    lock: Arc<RwLock<()>>,
    cancel: watch::Sender<bool>,
}

impl Imports {
    pub fn new(pack: Arc<MediaPack>) -> Self {
        Self {
            pack: Arc::new(Mutex::new(Some(pack))),
            lock: Arc::new(RwLock::new(())),
            cancel: watch::channel(false).0,
        }
    }

    /// Imports `paths` (files, already resolved with [`media_paths`]) as one batch and one undo
    /// entry. Run the future on the tokio runtime; it ends with `ImportEvent::Done`.
    pub fn files(
        &self,
        paths: Vec<PathBuf>,
        events: Events,
        encoder: HardwareEncoder,
    ) -> impl Future<Output = ()> + Send + 'static {
        let cancel = self.fresh_cancel();
        encode::process_files(
            self.pack.clone(),
            paths,
            events,
            encoder,
            self.lock.clone(),
            cancel,
        )
    }

    /// Streams an Edgeware pack's media into the draft [`create_from_edgeware`] made, filling the
    /// wallpaper and splash slots as their files land.
    pub fn edgeware(
        &self,
        media: EdgewareMedia,
        events: Events,
        encoder: HardwareEncoder,
    ) -> impl Future<Output = ()> + Send + 'static {
        let cancel = self.fresh_cancel();
        import::run_import(
            self.pack.clone(),
            media.source,
            media.media,
            media.references,
            events,
            encoder,
            self.lock.clone(),
            cancel,
        )
    }

    /// Asks every running batch to stop. Files already in finish; the rest are never started.
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }

    /// Cancels, and resolves once nothing is importing any more.
    pub fn stop(&self) -> impl Future<Output = ()> + Send + 'static {
        self.cancel();
        let lock = self.lock.clone();
        async move {
            drop(lock.write().await);
        }
    }

    /// Cancels, waits, and detaches the pack: nothing can import into it afterwards. For closing.
    pub fn close(&self) -> impl Future<Output = ()> + Send + 'static {
        let stop = self.stop();
        let pack = self.pack.clone();
        async move {
            stop.await;
            pack.lock().await.take();
        }
    }

    /// Waits for running batches to finish (without cancelling them) and holds new ones off
    /// until the guard drops. For Save As, which moves the pack's working copy.
    pub fn exclusive(&self) -> impl Future<Output = OwnedRwLockWriteGuard<()>> + Send + 'static {
        self.lock.clone().write_owned()
    }

    /// A new batch shouldn't inherit an earlier batch's cancellation.
    fn fresh_cancel(&self) -> watch::Receiver<bool> {
        self.cancel.send_replace(false);
        self.cancel.subscribe()
    }
}

/// What dropped or picked paths hold that can be imported: folders are walked, and anything
/// that isn't an image, video or audio file (or is OS cruft such as `.DS_Store`) is left out.
/// Touches the disk, so call it off the UI thread.
pub fn media_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for path in paths {
        if path.is_dir() {
            found.extend(encode::explore_folder(&path));
        } else if !encode::is_junk_path(&path) && encode::is_media_path(&path).unwrap_or(false) {
            found.push(path);
        }
    }
    found
}

/// Import progress across every batch running at once, as the import window shows it.
#[derive(Clone, Debug, Default)]
pub struct ImportProgress {
    /// Batches started and not yet done.
    pub batches: usize,
    /// Files in the batches since the last time nothing was importing.
    pub total: usize,
    /// Of those, how many have finished, however they ended.
    pub done: usize,
    /// Files passed over because the pack already has them.
    pub skipped: usize,
    /// Files that failed, kept until dismissed.
    pub errors: Vec<UploadErrorPayload>,
}

impl ImportProgress {
    pub fn apply(&mut self, event: &ImportEvent) {
        match event {
            ImportEvent::Start { total } => {
                if self.batches == 0 {
                    self.total = *total;
                    self.done = 0;
                    self.skipped = 0;
                } else {
                    self.total += total;
                }
                self.batches += 1;
            }
            ImportEvent::FileDone => self.done += 1,
            ImportEvent::Skipped(_) => self.skipped += 1,
            ImportEvent::Error(error) => self.errors.push(error.clone()),
            ImportEvent::Done => self.batches = self.batches.saturating_sub(1),
            ImportEvent::Added(_) | ImportEvent::SlotsFilled(_) => {}
        }
    }

    pub fn running(&self) -> bool {
        self.batches > 0
    }

    /// From 0 to 1.
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f32 / self.total as f32).min(1.0)
    }

    /// Whether a finished import still has something to say.
    pub fn has_report(&self) -> bool {
        self.skipped > 0 || !self.errors.is_empty()
    }
}

// ── Edgeware ────────────────────────────────────────────────────────────────────────────────

pub use converter::{Warning as ConversionWarning, WarningKind};

/// A short, sentence-case name for a kind of conversion warning, for grouping them in a list.
pub fn warning_kind_label(kind: WarningKind) -> &'static str {
    match kind {
        WarningKind::MalformedSource => "Unreadable file",
        WarningKind::UnsupportedFeatureDropped => "Feature not supported",
        WarningKind::ScriptSkipped => "Script not converted",
        WarningKind::DiscordSkipped => "Discord status not supported",
        WarningKind::CorruptionNotConverted => "Corruption not converted",
        WarningKind::ConfigNotConverted => "Settings not converted",
        WarningKind::UnreadableMediaFile => "Missing media file",
    }
}

/// A draft made from an Edgeware pack: its behaviour and metadata are already written, and its
/// media is waiting to be imported with [`Imports::edgeware`].
pub struct EdgewareDraft {
    pub pack: MediaPack,
    /// What the converter couldn't carry over.
    pub warnings: Vec<converter::Warning>,
    pub media: EdgewareMedia,
}

/// An Edgeware pack's media, still in its zip.
pub struct EdgewareMedia {
    source: Arc<ZipSource>,
    media: Vec<ConvertedMedia>,
    references: Vec<(MediaSlot, String)>,
}

impl EdgewareMedia {
    pub fn len(&self) -> usize {
        self.media.len()
    }

    pub fn is_empty(&self) -> bool {
        self.media.is_empty()
    }
}

/// Converts the Edgeware `.zip` at `source` into a new draft pack. Reading and converting happen
/// before the draft exists, so an unreadable zip leaves nothing behind.
pub async fn create_from_edgeware(source: &Path, data_dir: &Path) -> Result<EdgewareDraft> {
    let source = source.to_path_buf();
    let (zip, output) = tokio::task::spawn_blocking(move || import::load(&source))
        .await
        .context("Converting the pack stopped unexpectedly")??;
    let converter::ConversionOutput {
        metadata,
        behaviour,
        media,
        media_references,
        icon: _,
        warnings,
    } = output;
    let pack = MediaPack::new_unsaved(data_dir, &metadata.name).await?;
    // Not an undo entry: this is where the draft starts.
    pack.initialize_from_conversion(behaviour, metadata).await?;
    Ok(EdgewareDraft {
        pack,
        warnings,
        media: EdgewareMedia {
            source: Arc::new(zip),
            media,
            references: media_references,
        },
    })
}
