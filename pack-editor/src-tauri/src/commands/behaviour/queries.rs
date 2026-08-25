//! The queries: what each surface reads when it renders.

use shared::behaviour::editor::{self, MediaSlots, PoolKind, TextItemRow, WebLinkRow};
use shared::behaviour::{AudioMedia, ContentGroup, PopupMedia, Stage, Transition};
use tauri::State;

use crate::AppState;

/// What the Content tab's badges and the Experience tab's header need, without reading either
/// surface's contents.
#[derive(serde::Serialize)]
pub struct BehaviourSummary {
    captions: usize,
    prompts: usize,
    notifications: usize,
    web_links: usize,
    content_groups: usize,
    /// Whether the pack has a timeline section at all.
    has_timeline: bool,
    /// Whether it plays: a timeline the author switched off is present but disabled.
    timeline_enabled: bool,
    /// The timeline's mode-name override, if any.
    timeline_label: Option<String>,
}

#[tauri::command]
pub async fn get_behaviour_summary(state: State<'_, AppState>) -> Result<BehaviourSummary, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(|conn: &rusqlite::Connection| {
                let timeline = editor::timeline(conn)?;
                Ok(BehaviourSummary {
                    captions: editor::text_pool(conn, PoolKind::Caption)?.len(),
                    prompts: editor::text_pool(conn, PoolKind::Prompt)?.len(),
                    notifications: editor::text_pool(conn, PoolKind::Notification)?.len(),
                    web_links: editor::web_links(conn)?.len(),
                    content_groups: editor::content_groups(conn)?.len(),
                    has_timeline: timeline.is_some(),
                    timeline_enabled: timeline.as_ref().is_some_and(|view| view.enabled),
                    timeline_label: timeline.and_then(|view| view.label),
                })
            })
            .await
        })
        .await
}

/// The timeline as the editor shows it: present even while switched off.
#[derive(serde::Serialize)]
pub struct TimelineDto {
    stages: Vec<Stage>,
    transitions: Vec<Transition>,
    enabled: bool,
    label: Option<String>,
}

#[tauri::command]
pub async fn get_timeline(state: State<'_, AppState>) -> Result<Option<TimelineDto>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(|conn: &rusqlite::Connection| {
                Ok(editor::timeline(conn)?.map(|view| TimelineDto {
                    stages: view.timeline.stages,
                    transitions: view.timeline.transitions,
                    enabled: view.enabled,
                    label: view.label,
                }))
            })
            .await
        })
        .await
}

// ── Media slots ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_media_slots(state: State<'_, AppState>) -> Result<MediaSlots, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::media_slots).await)
        .await
}

#[tauri::command]
pub async fn get_popup_attributes(
    state: State<'_, AppState>,
    ids: Vec<u64>,
) -> Result<Vec<(u64, PopupMedia)>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn: &rusqlite::Connection| editor::popup_attributes(conn, &ids))
                .await
        })
        .await
}

#[tauri::command]
pub async fn get_audio_attributes(
    state: State<'_, AppState>,
    ids: Vec<u64>,
) -> Result<Vec<(u64, AudioMedia)>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn: &rusqlite::Connection| editor::audio_attributes(conn, &ids))
                .await
        })
        .await
}


#[tauri::command]
pub async fn get_text_pool(
    state: State<'_, AppState>,
    kind: PoolKind,
) -> Result<Vec<TextItemRow>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn: &rusqlite::Connection| editor::text_pool(conn, kind))
                .await
        })
        .await
}

#[tauri::command]
pub async fn get_web_links(state: State<'_, AppState>) -> Result<Vec<WebLinkRow>, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::web_links).await)
        .await
}

#[tauri::command]
pub async fn get_content_groups(state: State<'_, AppState>) -> Result<Vec<ContentGroup>, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::content_groups).await)
        .await
}

/// What taking `media` out of `stage` would cost beyond the membership itself.
///
/// Empty is the safe case: the stage selects by a tag of its own, so removing it says only what the
/// author asked. A non-empty answer names the author's tags that would come off the file and what
/// else they are doing, which is what the confirmation says before `set_stage_membership` is sent
/// with `accept_collateral`.
///
/// Asked of the pack as it stands, so it can be shown *before* the author decides; the command
/// re-asks it inside the transaction, which is the answer that governs.
#[tauri::command]
pub async fn get_stage_membership_cost(
    state: State<'_, AppState>,
    media: u64,
    stage: String,
) -> Result<Vec<editor::Collateral>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn: &rusqlite::Connection| {
                Ok(editor::plan_stage_membership(conn, media, &stage, false)?.collateral)
            })
            .await
        })
        .await
}
