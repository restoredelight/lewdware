//! The queries: what each surface reads when it renders.

use shared::behaviour::editor::{
    self, CaptionRecord, MediaSlots, NotificationRecord, PromptRecord, WebLinkRecord,
};
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
                    captions: editor::captions(conn)?.len(),
                    prompts: editor::prompts(conn)?.len(),
                    notifications: editor::notifications(conn)?.len(),
                    web_links: editor::web_links(conn)?.len(),
                    content_groups: editor::content_groups(conn)?.len(),
                    has_timeline: timeline.is_some(),
                    timeline_enabled: timeline.as_ref().is_some_and(|view| view.enabled),
                    timeline_label: timeline.and_then(|view| view.timeline.label),
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

#[derive(serde::Serialize)]
pub struct StageRemovalImpact {
    tag: Option<String>,
    media: u64,
    stages: Vec<String>,
    content: u64,
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
                    label: view.timeline.label,
                }))
            })
            .await
        })
        .await
}

/// The timeline membership rows one media inspector renders, already evaluated against its tags.
#[tauri::command]
pub async fn get_stage_membership(
    state: State<'_, AppState>,
    media: u64,
) -> Result<Vec<editor::StageMembership>, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn| editor::stage_membership(conn, media))
                .await
        })
        .await
}

#[tauri::command]
pub async fn get_stage_removal_impact(
    state: State<'_, AppState>,
    id: String,
) -> Result<StageRemovalImpact, String> {
    state
        .with_pack(async |pack| {
            pack.behaviour_query(move |conn| {
                let tag = editor::stage_owned_tags(conn, &id)?
                    .into_iter()
                    .find(|owned| !owned.excluded)
                    .map(|owned| owned.tag);
                let Some(tag) = tag else {
                    return Ok(StageRemovalImpact { tag: None, media: 0, stages: vec![], content: 0 });
                };
                let media = conn.query_row(
                    "SELECT COUNT(*) FROM media_tags mt JOIN tags t ON t.id = mt.tag_id WHERE t.name = ?1",
                    [&tag], |row| row.get(0),
                )?;
                let stages = conn.prepare(
                    "SELECT s.label FROM behaviour_stage_tag st JOIN tags t ON t.id = st.tag_id JOIN behaviour_stage s ON s.id = st.stage_id WHERE t.name = ?1 AND s.id <> ?2 ORDER BY s.position",
                )?.query_map([&tag, &id], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
                let content = conn.query_row(
                    "SELECT (SELECT COUNT(*) FROM behaviour_content_group_tag x JOIN tags t ON t.id = x.tag_id WHERE t.name = ?1) + (SELECT COUNT(*) FROM behaviour_content_entry_tag x JOIN tags t ON t.id = x.tag_id WHERE t.name = ?1)",
                    [&tag], |row| row.get(0),
                )?;
                Ok(StageRemovalImpact { tag: Some(tag), media, stages, content })
            }).await
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
            pack.behaviour_query(move |conn: &rusqlite::Connection| {
                editor::popup_attributes(conn, &ids)
            })
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
            pack.behaviour_query(move |conn: &rusqlite::Connection| {
                editor::audio_attributes(conn, &ids)
            })
            .await
        })
        .await
}

#[tauri::command]
pub async fn get_captions(state: State<'_, AppState>) -> Result<Vec<CaptionRecord>, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::captions).await)
        .await
}

#[tauri::command]
pub async fn get_prompts(state: State<'_, AppState>) -> Result<Vec<PromptRecord>, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::prompts).await)
        .await
}

#[tauri::command]
pub async fn get_notifications(
    state: State<'_, AppState>,
) -> Result<Vec<NotificationRecord>, String> {
    state
        .with_pack(async |pack| pack.behaviour_query(editor::notifications).await)
        .await
}

#[tauri::command]
pub async fn get_web_links(state: State<'_, AppState>) -> Result<Vec<WebLinkRecord>, String> {
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
