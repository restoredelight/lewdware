//! One open pack, as a gpui entity: [`Session`] plus what the window needs around it.
//!
//! Views hold an `Entity<Editor>`, read from `editor.session()` in `render`, and subscribe to
//! [`EditorEvent::Changed`] to learn when something they show has changed. Writes go through
//! [`Editor::apply`], which runs them, reports failures, and emits the change.
//!
//! Imports also live here: [`Editor::import_paths`] and [`Editor::import_edgeware`] run the
//! pipelines on the tokio runtime (holding off idle suspend), fold their events into the session
//! and [`ImportProgress`], and emit what changed like any other write.
//!
//! Opening another pack makes a new `Editor`. Nothing from the old one can land in the new one.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use futures::StreamExt;
use gpui_kit::{Context, EventEmitter, SharedString};
use pack_editor_core::changes::Changes;
use pack_editor_core::events::{Events, ImportEvent};
use pack_editor_core::history;
use pack_editor_core::pack::MediaPack;
use pack_editor_core::session::lifecycle::{
    media_paths, ConversionWarning, EdgewareMedia, ImportProgress, Imports,
};
use pack_editor_core::session::{Applied, Session};
use shared::encode::HardwareEncoder;

use crate::{keep_awake, runtime};

pub enum EditorEvent {
    Changed(Changes),
}

impl EventEmitter<EditorEvent> for Editor {}

/// Asks every view to commit the text field being typed in, before something reads or leaves
/// the pack (saving, closing, switching views). Blur isn't reliably reported, so a field's edit
/// otherwise waits until a view next sees it unfocused. Handlers run in the order effects were
/// queued: emit this, then `defer` whatever should see the committed edits.
pub struct CommitPending;

impl EventEmitter<CommitPending> for Editor {}

/// A save in progress: files written so far, out of how many.
#[derive(Clone, Copy, Default)]
pub struct SaveProgress {
    pub saved: usize,
    pub total: usize,
    /// Held until the running import finishes: a first save or Save As moves the working copy,
    /// which an import is still writing to.
    pub waiting_for_import: bool,
}

pub struct Editor {
    session: Session,
    imports: Imports,
    pub saving: Option<SaveProgress>,
    /// The last write that failed, shown until the next one succeeds or it's dismissed.
    pub error: Option<SharedString>,
    /// Imports into this pack, running and finished, for the import window.
    pub import_progress: ImportProgress,
    /// Imports between being asked for and their pipeline's first event (resolving paths,
    /// waiting on the encoder probe).
    preparing_imports: usize,
    /// What the Edgeware converter couldn't carry over, until the author dismisses it.
    pub import_warnings: Vec<ConversionWarning>,
}

impl Editor {
    pub fn new(session: Session) -> Self {
        let imports = Imports::new(session.pack().clone());
        Self {
            session,
            imports,
            saving: None,
            error: None,
            import_progress: ImportProgress::default(),
            preparing_imports: 0,
            import_warnings: Vec::new(),
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn pack(&self) -> &Arc<MediaPack> {
        self.session.pack()
    }

    pub fn history(&self) -> &history::Status {
        self.session.history()
    }

    pub fn imports(&self) -> &Imports {
        &self.imports
    }

    /// Whether there is work the pack file doesn't have yet. Drafts that were never saved count.
    pub fn is_dirty(&self) -> bool {
        !self.pack().is_saved_now()
    }

    /// Runs one write against the session. On success, emits what changed and returns the value;
    /// on failure nothing changed, and the error is kept for the toolbar to show.
    pub fn apply<T>(
        &mut self,
        cx: &mut Context<Self>,
        write: impl FnOnce(&mut Session) -> anyhow::Result<Applied<T>>,
    ) -> Option<T> {
        let result = write(&mut self.session);
        let value = match result {
            Ok(applied) => {
                self.error = None;
                if !applied.changes.is_empty() {
                    cx.emit(EditorEvent::Changed(applied.changes));
                }
                Some(applied.value)
            }
            Err(error) => {
                tracing::warn!("edit failed: {error:#}");
                self.error = Some(error.to_string().into());
                None
            }
        };
        // History and the saved state move with every write, even one that changed nothing the
        // views show.
        cx.notify();
        value
    }

    /// Emits [`CommitPending`]. Defer the work that depends on it.
    pub fn commit_pending(&mut self, cx: &mut Context<Self>) {
        cx.emit(CommitPending);
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if self.history().can_undo {
            self.apply(cx, Session::undo);
        }
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if self.history().can_redo {
            self.apply(cx, Session::redo);
        }
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        cx.notify();
    }

    /// Shows a message in the toolbar's error slot, as a failed write would.
    pub fn report(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        cx.notify();
    }

    // ── Imports ─────────────────────────────────────────────────────────────────────────────

    /// Whether an import is starting or running.
    pub fn importing(&self) -> bool {
        self.preparing_imports > 0 || self.import_progress.running()
    }

    /// Imports dropped or picked files and folders as one undo entry. Folders are walked;
    /// anything that isn't media is left out, and if that leaves nothing the toolbar says so.
    pub fn import_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        self.preparing_imports += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let found = runtime::blocking(move || (media_paths(paths), encoder())).await;
            let _ = this.update(cx, |this, cx| {
                this.preparing_imports -= 1;
                match found {
                    Ok((found, _)) if found.is_empty() => this.report(
                        "Nothing to import: no images, videos or audio were found there.",
                        cx,
                    ),
                    Ok((found, encoder)) => {
                        let run = this.imports.files(found, this.event_sink(cx), encoder);
                        spawn_awake("Importing media into a pack", run);
                    }
                    Err(error) => {
                        this.report(format!("Couldn't read what was dropped: {error:#}"), cx)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Streams an Edgeware pack's media in, and keeps the converter's warnings for the dialog.
    pub fn import_edgeware(
        &mut self,
        media: EdgewareMedia,
        warnings: Vec<ConversionWarning>,
        cx: &mut Context<Self>,
    ) {
        self.import_warnings = warnings;
        cx.notify();
        if media.is_empty() {
            return;
        }
        self.preparing_imports += 1;
        cx.spawn(async move |this, cx| {
            let encoder = runtime::blocking(encoder).await;
            let _ = this.update(cx, |this, cx| {
                this.preparing_imports -= 1;
                let encoder = encoder.unwrap_or(HardwareEncoder::SoftwareFallback);
                let run = this.imports.edgeware(media, this.event_sink(cx), encoder);
                spawn_awake("Importing an Edgeware pack", run);
                cx.notify();
            });
        })
        .detach();
    }

    /// Stops the running imports: files already in stay, the rest are never started.
    pub fn cancel_import(&mut self, cx: &mut Context<Self>) {
        self.imports.cancel();
        cx.notify();
    }

    /// Clears what a finished import had to report (skips and failures).
    pub fn dismiss_import_report(&mut self, cx: &mut Context<Self>) {
        if self.import_progress.running() {
            self.import_progress.errors.clear();
        } else {
            self.import_progress = ImportProgress::default();
        }
        cx.notify();
    }

    pub fn dismiss_import_warnings(&mut self, cx: &mut Context<Self>) {
        self.import_warnings.clear();
        cx.notify();
    }

    /// Where a pipeline sends its events: a channel drained on the UI thread, a batch at a time.
    fn event_sink(&self, cx: &mut Context<Self>) -> Events {
        let (sender, mut receiver) = futures::channel::mpsc::unbounded::<ImportEvent>();
        cx.spawn(async move |this, cx| {
            while let Some(first) = receiver.next().await {
                // A pipeline adds files far faster than anything needs redrawing: take whatever
                // has queued up and report it as one change.
                let mut batch = vec![first];
                while let Ok(event) = receiver.try_recv() {
                    batch.push(event);
                }
                if this
                    .update(cx, |this, cx| this.import_events(batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Arc::new(move |event| {
            let _ = sender.unbounded_send(event);
        })
    }

    fn import_events(&mut self, events: Vec<ImportEvent>, cx: &mut Context<Self>) {
        let mut changes = Changes::default();
        for event in &events {
            self.import_progress.apply(event);
            match self.session.import_event(event) {
                Ok(Some(applied)) => {
                    changes.scopes |= applied.changes.scopes;
                    changes.media.added.extend(applied.changes.media.added);
                    changes.media.updated.extend(applied.changes.media.updated);
                    changes.media.removed.extend(applied.changes.media.removed);
                }
                Ok(None) => {}
                Err(error) => tracing::warn!("couldn't fold an import event in: {error:#}"),
            }
        }
        if !changes.is_empty() {
            cx.emit(EditorEvent::Changed(changes));
        }
        cx.notify();
    }
}

/// Runs an import pipeline on the tokio runtime with idle suspend held off.
fn spawn_awake(reason: &'static str, run: impl Future<Output = ()> + Send + 'static) {
    runtime::spawn(async move {
        let _awake = keep_awake::acquire(reason).await;
        run.await;
    });
}

static ENCODER: OnceLock<HardwareEncoder> = OnceLock::new();

/// Starts probing for a hardware video encoder in the background, so the first import doesn't
/// wait on it. Call once at startup, after the sidecar paths are set.
pub fn detect_encoder() {
    let probe = std::thread::Builder::new()
        .name("encoder-probe".into())
        .spawn(encoder);
    if let Err(error) = probe {
        tracing::warn!("couldn't start the encoder probe: {error}");
    }
}

/// The detected encoder, waiting for the probe if it's still running. Blocks.
fn encoder() -> HardwareEncoder {
    ENCODER
        .get_or_init(HardwareEncoder::detect_and_test)
        .clone()
}
