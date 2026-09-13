//! Filling the behaviour document's content pools -- captions, prompts, notifications and
//! web links -- from the Edgeware index, and the content groups its moods become.

use std::borrow::Cow;
use std::collections::HashSet;

use crate::model::{CorruptionLevel, EdgewareIndex, EdgewareMood, WebEntry};
use crate::slug::unique_slug;
use shared::behaviour::{Caption, Content, ContentGroup, Notification, Prompt, WebLink};
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

/// Builds a text pool from `default_pool` (untagged) plus each mood's own pool (tagged
/// `[mood.name]`), via `pool` selecting which `MoodBase` field to read. Shared by captions,
/// prompts and notifications -- they're all `{ tags: string[] }` pools over the
/// same mood structure.
fn tagged_texts(
    default_pool: &[String],
    moods: &[EdgewareMood],
    pool: impl Fn(&crate::model::MoodBase) -> &Vec<String>,
) -> Vec<(String, Vec<String>)> {
    let mut items: Vec<(String, Vec<String>)> = default_pool
        .iter()
        .map(|text| (text.clone(), vec![]))
        .collect();
    for mood in moods {
        for text in pool(&mood.base) {
            items.push((text.clone(), vec![mood.name.clone()]));
        }
    }
    items
}

pub(super) fn populate_captions(index: &EdgewareIndex, content: &mut Content) {
    content.captions = tagged_texts(&index.default.captions, &index.moods, |base| &base.captions)
        .into_iter()
        .map(|(text, tags)| Caption { id: 0, text, tags })
        .collect();
    // Fold the close-button label into captions (per `behaviour-design/edgeware-compat.md`) --
    // only when the author actually set it, so a pack that never touched `popupClose` doesn't
    // get Edgeware's own boilerplate ("I Submit <3") injected as a fake caption.
    if let Some(popup_close) = &index.default_extra.popup_close {
        content.captions.push(Caption {
            id: 0,
            text: popup_close.clone(),
            tags: vec![],
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
    content.prompts = tagged_texts(&index.default.prompts, &index.moods, |base| &base.prompts)
        .into_iter()
        .map(|(text, tags)| Prompt {
            id: 0,
            text,
            tags,
            timeout_seconds: None,
        })
        .collect();
}

pub(super) fn populate_notifications(index: &EdgewareIndex, content: &mut Content) {
    content.notifications = tagged_texts(&index.default.notifications, &index.moods, |base| {
        &base.notifications
    })
    .into_iter()
    .map(|(text, tags)| Notification {
        id: 0,
        text,
        tags,
        summary: None,
    })
    .collect();
}

pub(super) fn populate_web_links(index: &EdgewareIndex, content: &mut Content) {
    let mut links = vec![];
    append_web_links(&mut links, &index.default.web, &[]);
    for mood in &index.moods {
        append_web_links(&mut links, &mood.base.web, std::slice::from_ref(&mood.name));
    }
    content.web_links = links;
}

/// Edgeware models a link as a base URL plus a randomly selected suffix. Lewdware models every
/// possible destination as its own item. Repeated suffixes collapse to one destination because
/// Lewdware links have no weighting model. An absent suffix list means the base URL itself is the
/// item.
fn append_web_links(links: &mut Vec<WebLink>, entries: &[WebEntry], tags: &[String]) {
    for entry in entries {
        if entry.args.is_empty() {
            links.push(WebLink {
                id: 0,
                url: entry.url.clone(),
                tags: tags.to_vec(),
            });
        } else {
            let mut seen = HashSet::new();
            links.extend(entry.args.iter().filter_map(|suffix| {
                let url = format!("{}{suffix}", entry.url);
                seen.insert(url.clone()).then(|| WebLink {
                    id: 0,
                    url,
                    tags: tags.to_vec(),
                })
            }));
        }
    }
}

#[cfg(test)]
mod web_link_tests {
    use super::*;

    #[test]
    fn suffixes_expand_into_distinct_tagged_links_without_weighting() {
        let entries = [WebEntry {
            url: "https://example.invalid/?q=".to_string(),
            args: vec!["one".to_string(), "two".to_string(), "one".to_string()],
        }];
        let tags = ["mood".to_string()];
        let mut links = vec![];

        append_web_links(&mut links, &entries, &tags);

        assert_eq!(
            links,
            [
                WebLink {
                    id: 0,
                    url: "https://example.invalid/?q=one".to_string(),
                    tags: tags.to_vec(),
                },
                WebLink {
                    id: 0,
                    url: "https://example.invalid/?q=two".to_string(),
                    tags: tags.to_vec(),
                },
            ]
        );
    }

    #[test]
    fn a_link_without_suffixes_stays_as_one_item() {
        let entries = [WebEntry {
            url: "https://example.invalid/".to_string(),
            args: vec![],
        }];
        let mut links = vec![];

        append_web_links(&mut links, &entries, &[]);

        assert_eq!(
            links,
            [WebLink {
                id: 0,
                url: "https://example.invalid/".to_string(),
                tags: vec![],
            }]
        );
    }
}
