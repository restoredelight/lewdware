//! Progress from the media pipelines (`encode::process_files`, `encode::process_file_into_slot`,
//! `import::run_import`), reported through a callback rather than any one UI's event system.
//!
//! The callback runs on whichever tokio worker the pipeline is on, in the order the events
//! happen. Keep it cheap: forward the event to the UI thread and return.

use std::sync::Arc;

use crate::encode::UploadErrorPayload;
use crate::import::FilledSlot;
use crate::pack::MediaFile;

#[derive(Debug, Clone)]
pub enum ImportEvent {
    /// A batch began; `total` files will each end in one `FileDone`.
    Start { total: usize },
    /// A file was added to the pack.
    Added(MediaFile),
    /// A file was not media, or was junk, and was passed over without an error.
    Skipped(String),
    /// A file, or the batch, failed.
    Error(UploadErrorPayload),
    /// The Edgeware importer filled media slots with a file it had just added.
    SlotsFilled(Vec<FilledSlot>),
    /// One file finished, however it ended.
    FileDone,
    /// The batch finished, was cancelled, or never started. Always the last event.
    Done,
}

pub type Events = Arc<dyn Fn(ImportEvent) + Send + Sync>;
