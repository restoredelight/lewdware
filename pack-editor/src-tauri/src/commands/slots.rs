//! Media slots -- wallpaper, splash, per-stage wallpaper -- and the popup roles a file
//! can be given.

use serde::Serialize;
use shared::encode::HardwareEncoder;
use tauri::{AppHandle, State};

use crate::pack::MediaFile;
use crate::{encode, pack, AppState};

/// What filling a slot produced: the file the slot now points at, so the media grid can show it
/// without waiting for a refresh. `added` distinguishes a fresh import from a file the pack
/// already had (see `AddOutcome`), which the grid already knows about.
///
/// No behaviour document: the slot's new value is the caller's own argument, and any surface that
/// renders slots refetches. Handing back the whole document for the front end to adopt is the
/// arrangement `design/editor-data-flow.md` replaced.
#[derive(Serialize)]
pub struct SlotFilled {
    pub file: MediaFile,
    pub added: bool,
    /// The file this replaced, when it was only ever this slot's scenery and left with the slot --
    /// the same rule and the same purpose as `SlotCleared::deleted_id`.
    pub deleted_id: Option<u64>,
}

/// Picks one media file and puts it in `slot`. The picker is filtered to what the slot can
/// actually use -- a wallpaper must be an image, a splash may also be a video -- since offering
/// a file the feature would silently skip is the honesty problem `pack_has_*` exists to fix.
#[tauri::command]
pub async fn fill_media_slot_dialog(
    state: State<'_, AppState>,
    app: AppHandle,
    slot: shared::behaviour::MediaSlot,
) -> Result<Option<SlotFilled>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (extensions, label): (&[&str], &str) = match slot {
        shared::behaviour::MediaSlot::StageAudio { .. }
        | shared::behaviour::MediaSlot::StageEntrySound { .. }
        | shared::behaviour::MediaSlot::StagePromptSound { .. } => (
            &["mp3", "ogg", "wav", "flac", "m4a", "aac", "opus"],
            "Audio",
        ),
        shared::behaviour::MediaSlot::Splash
        | shared::behaviour::MediaSlot::StageEntrySplash { .. } => (
            &[
                "png", "jpg", "jpeg", "webp", "avif", "bmp", "gif", "mp4", "webm", "mkv", "mov",
            ],
            "Image or video",
        ),
        _ => (&["png", "jpg", "jpeg", "webp", "avif", "bmp"], "Image"),
    };
    let app_c = app.clone();
    let picked = tokio::task::spawn_blocking(move || {
        app_c
            .dialog()
            .file()
            .set_title("Select a file")
            .add_filter(label, extensions)
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;

    let Some(handle) = picked else {
        return Ok(None);
    };
    let path = handle.into_path().map_err(|e| e.to_string())?;

    let encoder = state
        .hardware_encoder
        .get()
        .cloned()
        .unwrap_or(HardwareEncoder::SoftwareFallback);
    let label = match slot {
        shared::behaviour::MediaSlot::Splash => "Set splash",
        shared::behaviour::MediaSlot::StageEntrySplash { .. } => "Set stage splash",
        shared::behaviour::MediaSlot::StageAudio { .. } => "Set stage audio",
        shared::behaviour::MediaSlot::StageEntrySound { .. }
        | shared::behaviour::MediaSlot::StagePromptSound { .. } => "Set stage sound",
        _ => "Set wallpaper",
    };
    let (outcome, deleted_id) = encode::process_file_into_slot(
        &state.pack,
        &path,
        &app,
        encoder,
        &state.upload_lock,
        slot,
        label.to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;

    Ok(Some(SlotFilled {
        added: matches!(outcome, pack::AddOutcome::Added(_)),
        file: outcome.file().clone(),
        deleted_id,
    }))
}

/// What emptying a slot produced: the file it deleted (if the file was
/// only ever scenery -- see `MediaPack::clear_media_slot`) so the grid can drop it.
#[derive(Serialize)]
pub struct SlotCleared {
    pub deleted_id: Option<u64>,
}

#[tauri::command]
pub async fn clear_media_slot(
    state: State<'_, AppState>,
    slot: shared::behaviour::MediaSlot,
) -> Result<Option<SlotCleared>, String> {
    state
        .with_pack_or(None, async |pack| {
            let deleted_id = pack.clear_media_slot(slot).await?;
            Ok(Some(SlotCleared { deleted_id }))
        })
        .await
}

/// Points a slot at media which is already in the pack. Unlike the import command this never
/// changes the file's pool membership: choosing an existing popup or background track explicitly
/// must not silently remove it from the role the author already gave it.
#[tauri::command]
pub async fn set_media_slot(
    state: State<'_, AppState>,
    slot: shared::behaviour::MediaSlot,
    media_id: u64,
) -> Result<Option<SlotCleared>, String> {
    state
        .with_pack_or(None, async |pack| {
            let deleted_id = pack.fill_media_slot(slot, media_id, false).await?;
            Ok(Some(SlotCleared { deleted_id }))
        })
        .await
}

#[tauri::command]
pub async fn set_shown_as_popup(
    state: State<'_, AppState>,
    id: u64,
    shown: bool,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.set_shown_as_popup(id, shown).await)
        .await
}

#[tauri::command]
pub async fn set_popup_audio(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    popup: bool,
) -> Result<(), String> {
    state
        .with_pack_or((), async |pack| pack.set_popup_audio(ids, popup).await)
        .await
}
