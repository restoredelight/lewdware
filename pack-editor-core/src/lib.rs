//! The pack editor's model, free of any UI toolkit: the open pack and its working database
//! (`pack`, `editor_db`), undo history (`history`), and the media pipelines that bring files in
//! (`encode`, `import`, `thumbnail`).
//!
//! The Tauri editor (`pack-editor/`) wraps this in commands; the gpui port calls it directly.
//!
//! Long imports and saves should hold off idle suspend. That is left to the caller (the
//! platform portals involved pull in executor-specific dependencies), which wraps the futures
//! here in its own keep-awake guard.

pub mod changes;
pub mod editor_db;
pub mod encode;
pub mod events;
pub mod history;
pub mod import;
pub mod pack;
pub mod recents;
pub mod session;
pub mod thumbnail;

use std::sync::Arc;

use tokio::sync::Mutex;

/// The open pack, if any. Shared by everything that reads or writes it; most holders take the
/// lock only long enough to clone the `Arc`.
pub type PackState = Arc<Mutex<Option<Arc<pack::MediaPack>>>>;
