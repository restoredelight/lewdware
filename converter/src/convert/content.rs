//! Filling the behaviour document's content pools -- captions, prompts, notifications and
//! web links -- from the Edgeware index, and the content groups its moods become.

use std::borrow::Cow;
use std::collections::HashSet;

use crate::model::{CorruptionLevel, EdgewareIndex, EdgewareMood};
use crate::slug::unique_slug;
use shared::behaviour::{Content, ContentGroup, TextItem, WebLink};
use shared::tags;

pub(super) fn build_content_groups(
    index: &EdgewareIndex,
    used_ids: &mut HashSet<String>,
) -> Vec<ContentGroup> {
    index
        .moods
        .iter()
        .map(|mood| ContentGroup {
            id: unique_slug(&mood.name, used_ids),
            label: mood.name.clone(),
            description: None,
            tags: vec![mood.name.clone()],
            enabled_by_default: true,
        })
        .collect()
}

/// Builds a `TextItem` pool from `default_pool` (untagged) plus each mood's own pool (tagged
/// `[mood.name]`), via `pool` selecting which `MoodBase` field to read. Shared by captions,
/// prompts and notifications -- they're all `{ tags: string[] }` pools over the
/// same mood structure.
pub(super) fn text_items(
    default_pool: &[String],
    moods: &[EdgewareMood],
    pool: impl Fn(&crate::model::MoodBase) -> &Vec<String>,
) -> Vec<TextItem> {
    let mut items: Vec<TextItem> = default_pool
        .iter()
        .map(|text| TextItem {
            text: text.clone(),
            tags: vec![],
            timeout_seconds: None,
            summary: None,
        })
        .collect();
    for mood in moods {
        for text in pool(&mood.base) {
            items.push(TextItem {
                text: text.clone(),
                tags: vec![mood.name.clone()],
                timeout_seconds: None,
                summary: None,
            });
        }
    }
    items
}

pub(super) fn populate_captions(index: &EdgewareIndex, content: &mut Content) {
    content.captions = text_items(&index.default.captions, &index.moods, |base| &base.captions);
    // Fold the close-button label into captions (per `behaviour-design/edgeware-compat.md`) --
    // only when the author actually set it, so a pack that never touched `popupClose` doesn't
    // get Edgeware's own boilerplate ("I Submit <3") injected as a fake caption.
    if let Some(popup_close) = &index.default_extra.popup_close {
        content.captions.push(TextItem {
            text: popup_close.clone(),
            tags: vec![],
            timeout_seconds: None,
            summary: None,
        });
    }
}

/// Rewrites any mood name in the reserved tag namespace to an escaped one, everywhere a mood
/// name lives: the mood list itself, the media -> mood map, and `corruption.json`'s per-level
/// mood deltas (which are matched against those same names by `build_timeline`). See
/// `shared::tags::escape`.
pub(super) fn escape_managed_mood_names(index: &mut EdgewareIndex, levels: &mut [CorruptionLevel]) {
    for mood in &mut index.moods {
        if let Cow::Owned(escaped) = tags::escape_managed_prefix(&mood.name) {
            mood.name = escaped;
        }
    }
    for mood in index.media_moods.values_mut() {
        if let Cow::Owned(escaped) = tags::escape_managed_prefix(mood) {
            *mood = escaped;
        }
    }
    for level in levels {
        for mood in level.added_moods.iter_mut().chain(&mut level.removed_moods) {
            if let Cow::Owned(escaped) = tags::escape_managed_prefix(mood) {
                *mood = escaped;
            }
        }
    }
}

/// Edgeware's own submit-button label (`promptSubmit`/`subtext`) has nowhere to go: every prompt
/// submits with a plain "Submit", so the pack's override is read past rather than carried over.
pub(super) fn populate_prompts(index: &EdgewareIndex, content: &mut Content) {
    content.prompts = text_items(&index.default.prompts, &index.moods, |base| &base.prompts);
}

pub(super) fn populate_notifications(index: &EdgewareIndex, content: &mut Content) {
    content.notifications = text_items(&index.default.notifications, &index.moods, |base| {
        &base.notifications
    });
}

pub(super) fn populate_web_links(index: &EdgewareIndex, content: &mut Content) {
    let mut links: Vec<WebLink> = index
        .default
        .web
        .iter()
        .map(|entry| WebLink {
            url: entry.url.clone(),
            args: entry.args.clone(),
            tags: vec![],
        })
        .collect();
    for mood in &index.moods {
        for entry in &mood.base.web {
            links.push(WebLink {
                url: entry.url.clone(),
                args: entry.args.clone(),
                tags: vec![mood.name.clone()],
            });
        }
    }
    content.web_links = links;
}
