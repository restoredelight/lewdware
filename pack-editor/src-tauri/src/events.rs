//! Forwards `pack_editor_core`'s pipeline events to the webview, under the event names and
//! payloads the frontend listens for.

use std::sync::Arc;

use pack_editor_core::events::{Events, ImportEvent};
use tauri::{AppHandle, Emitter};

pub fn emitter(app: &AppHandle) -> Events {
    let app = app.clone();
    Arc::new(move |event| {
        let _ = match event {
            ImportEvent::Start { total } => {
                app.emit("upload:start", serde_json::json!({ "total": total }))
            }
            ImportEvent::Added(file) => app.emit("upload:added", &file),
            ImportEvent::Skipped(path) => app.emit("upload:skipped", &path),
            ImportEvent::Error(error) => app.emit("upload:error", &error),
            ImportEvent::SlotsFilled(filled) => app.emit("import:slots-filled", &filled),
            ImportEvent::FileDone => app.emit("upload:file-done", ()),
            ImportEvent::Done => app.emit("upload:done", ()),
        };
    })
}
