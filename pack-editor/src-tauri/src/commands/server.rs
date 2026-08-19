//! How the frontend reaches the local media server that streams a pack's files.

use tauri::State;

use crate::media_server;
use crate::{AppState, MediaServerInfo};

#[tauri::command]
pub(crate) fn get_media_server(state: State<'_, AppState>) -> Result<MediaServerInfo, String> {
    state.media_server.get().cloned().ok_or_else(|| {
        "Media previews are unavailable because the local server failed to start".into()
    })
}

/// Puts what the player did (see `MediaDisplay.svelte`) into the same log as the media server's
/// requests, so a stall reads as one ordered story: what the player did, and what -- if anything
/// -- it asked the server for at that moment.
///
/// `info`, unlike the per-request server trail: the front end only calls this when it has had to
/// work around a stalled player, which is rare and worth keeping a record of.
#[tauri::command]
pub(crate) fn trace_media_event(event: String) {
    tracing::info!(target: media_server::MEDIA_TRACE, "player: {event}");
}
