//! Recorded edits for captions, prompts, notifications, web links, content groups and the pack's
//! media slots, as functions over a connection. See `super` for the contract.
//!
//! Each is one transaction and one undo entry, labelled with what the author did. The bodies are
//! the Tauri editor's commands (`pack-editor/src-tauri/src/commands/behaviour/content.rs`,
//! `group.rs`, `slots.rs`) without the IPC: the same `shared::behaviour::editor` calls, the same
//! labels, and the same slot rules (`fill_media_slot_tx`, `delete_unreferenced_scenery`).

use anyhow::{bail, Result};
use rusqlite::{params, Connection, Transaction};
use shared::behaviour::editor as behaviour_editor;
use shared::behaviour::{Caption, ContentGroup, MediaSlot, Notification, Prompt, WebLink};
use shared::tags;

use crate::history;
use crate::pack::behaviour::{fill_media_slot_tx, slot_label};
use crate::pack::media::delete_unreferenced_scenery;
use crate::pack::reject_managed_tag;

/// The four pools of short content entries, which are edited the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    Caption,
    Prompt,
    Notification,
    WebLink,
}

impl EntryKind {
    /// What one is called in an undo label or a button: "caption", "web link".
    pub fn noun(self) -> &'static str {
        match self {
            EntryKind::Caption => "caption",
            EntryKind::Prompt => "prompt",
            EntryKind::Notification => "notification",
            EntryKind::WebLink => "web link",
        }
    }
}

/// One entry of any pool, as the editor lists it. `summary` is a notification's title and
/// `timeout_seconds` a prompt's time limit; both are `None` for the other kinds.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: i64,
    /// A caption's, prompt's or notification's text, or a web link's URL.
    pub text: String,
    pub tags: Vec<String>,
    pub summary: Option<String>,
    pub timeout_seconds: Option<f64>,
}

impl Entry {
    /// Whether removing it would lose anything the author wrote, and so deserves a confirmation.
    pub fn has_content(&self) -> bool {
        !self.text.trim().is_empty()
            || self
                .summary
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            || !self.tags.is_empty()
            || self.timeout_seconds.is_some()
    }
}

/// Every entry of `kind`, in the order the pack stores them.
pub fn entries(connection: &Connection, kind: EntryKind) -> Result<Vec<Entry>> {
    Ok(match kind {
        EntryKind::Caption => behaviour_editor::captions(connection)?
            .into_iter()
            .map(|c| Entry {
                id: c.id,
                text: c.text,
                tags: c.tags,
                summary: None,
                timeout_seconds: None,
            })
            .collect(),
        EntryKind::Prompt => behaviour_editor::prompts(connection)?
            .into_iter()
            .map(|p| Entry {
                id: p.id,
                text: p.text,
                tags: p.tags,
                summary: None,
                timeout_seconds: p.timeout_seconds,
            })
            .collect(),
        EntryKind::Notification => behaviour_editor::notifications(connection)?
            .into_iter()
            .map(|n| Entry {
                id: n.id,
                text: n.text,
                tags: n.tags,
                summary: n.summary,
                timeout_seconds: None,
            })
            .collect(),
        EntryKind::WebLink => behaviour_editor::web_links(connection)?
            .into_iter()
            .map(|l| Entry {
                id: l.id,
                text: l.url,
                tags: l.tags,
                summary: None,
                timeout_seconds: None,
            })
            .collect(),
    })
}

/// Every tag an author can pick: the pack's tags outside the reserved namespace, by name.
pub fn author_tags(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare("SELECT name FROM tags ORDER BY name COLLATE NOCASE")?;
    let names = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut found = Vec::new();
    for name in names {
        let name = name?;
        if !tags::is_managed(&name) {
            found.push(name);
        }
    }
    Ok(found)
}

/// The time limit the bundled modes give a prompt without one, in seconds. Mirrors
/// `pack-editor/src/lib/promptTimeout.ts`: 10s plus 1s per 2.5 characters, at least 15s, rounded
/// up to a multiple of 5.
pub fn automatic_prompt_timeout(text: &str) -> u32 {
    let characters = text.chars().count() as f64;
    let seconds = (10. + characters / 2.5).max(15.);
    ((seconds / 5.).ceil() * 5.) as u32
}

fn record<T>(
    connection: &Connection,
    label: &str,
    operation: impl FnOnce(&Transaction<'_>) -> Result<T>,
) -> Result<T> {
    history::record(connection, label, 0, &[], operation)
}

/// Blank is the absence of a value, not an empty string: a notification without a title, a group
/// without a description.
fn optional_text(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

// ── Entries ─────────────────────────────────────────────────────────────────────────────────

/// Adds an empty entry at the end of its pool and returns its id.
pub fn add_entry(connection: &Connection, kind: EntryKind) -> Result<i64> {
    record(
        connection,
        &format!("Add {}", kind.noun()),
        |tx| match kind {
            EntryKind::Caption => behaviour_editor::add_caption(
                tx,
                Caption {
                    id: 0,
                    text: String::new(),
                    tags: Vec::new(),
                },
            ),
            EntryKind::Prompt => behaviour_editor::add_prompt(
                tx,
                Prompt {
                    id: 0,
                    text: String::new(),
                    tags: Vec::new(),
                    timeout_seconds: None,
                },
            ),
            EntryKind::Notification => behaviour_editor::add_notification(
                tx,
                Notification {
                    id: 0,
                    text: String::new(),
                    tags: Vec::new(),
                    summary: None,
                },
            ),
            EntryKind::WebLink => behaviour_editor::add_web_link(
                tx,
                WebLink {
                    id: 0,
                    url: String::new(),
                    tags: Vec::new(),
                },
            ),
        },
    )
}

pub fn remove_entry(connection: &Connection, kind: EntryKind, id: i64) -> Result<()> {
    record(
        connection,
        &format!("Remove {}", kind.noun()),
        |tx| match kind {
            EntryKind::Caption => behaviour_editor::remove_caption(tx, id),
            EntryKind::Prompt => behaviour_editor::remove_prompt(tx, id),
            EntryKind::Notification => behaviour_editor::remove_notification(tx, id),
            EntryKind::WebLink => behaviour_editor::remove_web_link(tx, id),
        },
    )
}

/// A caption's, prompt's or notification's text (a notification's body), or a web link's URL.
pub fn set_entry_text(connection: &Connection, kind: EntryKind, id: i64, text: &str) -> Result<()> {
    record(
        connection,
        &format!("Edit {}", kind.noun()),
        |tx| match kind {
            EntryKind::Caption => behaviour_editor::set_caption_text(tx, id, text),
            EntryKind::Prompt => behaviour_editor::set_prompt_text(tx, id, text),
            EntryKind::Notification => behaviour_editor::set_notification_body(tx, id, text),
            EntryKind::WebLink => behaviour_editor::set_web_link_url(tx, id, text),
        },
    )
}

/// A notification's title. Blank clears it, and the notification is shown body-only.
pub fn set_notification_summary(
    connection: &Connection,
    id: i64,
    summary: Option<&str>,
) -> Result<()> {
    record(connection, "Edit notification title", |tx| {
        behaviour_editor::set_notification_summary(tx, id, optional_text(summary))
    })
}

/// A prompt's time limit in seconds; `None` leaves it to the mode (see
/// [`automatic_prompt_timeout`]).
pub fn set_prompt_timeout(connection: &Connection, id: i64, seconds: Option<f64>) -> Result<()> {
    if let Some(seconds) = seconds {
        if !seconds.is_finite() || seconds <= 0. {
            bail!("A time limit has to be a positive number of seconds");
        }
    }
    record(connection, "Edit prompt time limit", |tx| {
        behaviour_editor::set_prompt_timeout(tx, id, seconds)
    })
}

pub fn add_entry_tag(connection: &Connection, kind: EntryKind, id: i64, tag: &str) -> Result<()> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Ok(());
    }
    reject_managed_tag(tag)?;
    record(connection, "Add tag", |tx| match kind {
        EntryKind::Caption => behaviour_editor::add_caption_tag(tx, id, tag),
        EntryKind::Prompt => behaviour_editor::add_prompt_tag(tx, id, tag),
        EntryKind::Notification => behaviour_editor::add_notification_tag(tx, id, tag),
        EntryKind::WebLink => behaviour_editor::add_web_link_tag(tx, id, tag),
    })
}

pub fn remove_entry_tag(
    connection: &Connection,
    kind: EntryKind,
    id: i64,
    tag: &str,
) -> Result<()> {
    record(connection, "Remove tag", |tx| match kind {
        EntryKind::Caption => behaviour_editor::remove_caption_tag(tx, id, tag),
        EntryKind::Prompt => behaviour_editor::remove_prompt_tag(tx, id, tag),
        EntryKind::Notification => behaviour_editor::remove_notification_tag(tx, id, tag),
        EntryKind::WebLink => behaviour_editor::remove_web_link_tag(tx, id, tag),
    })
}

// ── Content groups ──────────────────────────────────────────────────────────────────────────

/// The label a fresh group gets, and the one that means "the author hasn't named it yet".
pub const NEW_GROUP_LABEL: &str = "New group";

/// Whether removing `group` would lose anything the author did, and so deserves a confirmation.
pub fn group_has_content(group: &ContentGroup) -> bool {
    !group.tags.is_empty()
        || group
            .description
            .as_deref()
            .is_some_and(|d| !d.trim().is_empty())
        || group.label != NEW_GROUP_LABEL
}

/// A key no group has yet, starting from `base`: `base`, then `base-2`, `base-3`...
///
/// The key becomes the suffix of the mode option the resolver synthesises for the group, so it
/// stays fixed once made; the label is what the author edits.
fn free_group_key(tx: &Transaction<'_>, base: &str) -> Result<String> {
    let taken = |key: &str| -> Result<bool> {
        Ok(tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM behaviour_content_group WHERE key = ?)",
            params![key],
            |row| row.get(0),
        )?)
    };
    if !taken(base)? {
        return Ok(base.to_string());
    }
    let mut n = 2;
    loop {
        let key = format!("{base}-{n}");
        if !taken(&key)? {
            return Ok(key);
        }
        n += 1;
    }
}

fn insert_group(
    tx: &Transaction<'_>,
    key_base: &str,
    label: String,
    tags: Vec<String>,
) -> Result<String> {
    let id = free_group_key(tx, key_base)?;
    behaviour_editor::add_content_group(
        tx,
        &ContentGroup {
            id: id.clone(),
            label,
            description: None,
            tags,
            enabled_by_default: true,
        },
    )?;
    Ok(id)
}

/// Adds an unnamed, enabled group at the end and returns its key.
pub fn add_content_group(connection: &Connection) -> Result<String> {
    record(connection, "Add content group", |tx| {
        insert_group(tx, "group", NEW_GROUP_LABEL.to_string(), Vec::new())
    })
}

/// The shortcut: an existing tag becomes a group people can switch off, named after it.
pub fn add_content_group_from_tag(connection: &Connection, tag: &str) -> Result<String> {
    let tag = tag.trim();
    if tag.is_empty() {
        bail!("Pick a tag to make toggleable");
    }
    reject_managed_tag(tag)?;
    let mut label = tag.to_string();
    if let Some(first) = label.chars().next() {
        let upper: String = first.to_uppercase().collect();
        label.replace_range(..first.len_utf8(), &upper);
    }
    record(connection, "Add content group", |tx| {
        insert_group(tx, tag, label, vec![tag.to_string()])
    })
}

pub fn remove_content_group(connection: &Connection, key: &str) -> Result<()> {
    record(connection, "Remove content group", |tx| {
        behaviour_editor::remove_content_group(tx, key)
    })
}

pub fn set_content_group_label(connection: &Connection, key: &str, label: &str) -> Result<()> {
    let label = label.trim();
    if label.is_empty() {
        bail!("A content group needs a name");
    }
    record(connection, "Edit group name", |tx| {
        behaviour_editor::set_content_group_label(tx, key, label)
    })
}

/// Blank clears the description.
pub fn set_content_group_description(
    connection: &Connection,
    key: &str,
    description: Option<&str>,
) -> Result<()> {
    record(connection, "Edit group description", |tx| {
        behaviour_editor::set_content_group_description(tx, key, optional_text(description))
    })
}

pub fn set_content_group_enabled_by_default(
    connection: &Connection,
    key: &str,
    enabled: bool,
) -> Result<()> {
    record(connection, "Change group default", |tx| {
        behaviour_editor::set_content_group_enabled_by_default(tx, key, enabled)
    })
}

pub fn add_content_group_tag(connection: &Connection, key: &str, tag: &str) -> Result<()> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Ok(());
    }
    reject_managed_tag(tag)?;
    record(connection, "Add tag", |tx| {
        behaviour_editor::add_content_group_tag(tx, key, tag)
    })
}

pub fn remove_content_group_tag(connection: &Connection, key: &str, tag: &str) -> Result<()> {
    record(connection, "Remove tag", |tx| {
        behaviour_editor::remove_content_group_tag(tx, key, tag)
    })
}

// ── Media slots ─────────────────────────────────────────────────────────────────────────────

fn clear_label(slot: &MediaSlot) -> &'static str {
    match slot {
        MediaSlot::Wallpaper => "Clear wallpaper",
        MediaSlot::Splash => "Clear splash",
        other => slot_label(other),
    }
}

/// Points `slot` at a file the pack already has (`MediaPack::fill_media_slot` with
/// `new_to_pack: false`). Unlike filling it from an import, this never changes the file's role:
/// choosing an existing popup for the wallpaper must not take it out of popups. The file the slot
/// held before leaves the pack if it was only ever that slot's scenery; its id is returned.
pub fn set_media_slot(
    connection: &Connection,
    slot: &MediaSlot,
    media_id: u64,
) -> Result<Option<u64>> {
    let (deleted, _) = history::record_with_media_refs(connection, slot_label(slot), 0, |tx| {
        let deleted = fill_media_slot_tx(tx, slot, media_id, false)?;
        // The displaced file's bytes have to outlive the entry that dropped it, or undo would
        // restore a slot pointing at media the collector has taken away.
        let refs = [Some(media_id), deleted].into_iter().flatten().collect();
        Ok((deleted, refs))
    })?;
    Ok(deleted)
}

/// Empties `slot`, deleting the file it held if that file was only ever scenery and nothing else
/// points at it (`MediaPack::clear_media_slot`). Returns the deleted file's id.
pub fn clear_media_slot(connection: &Connection, slot: &MediaSlot) -> Result<Option<u64>> {
    let (deleted, _) = history::record_with_media_refs(connection, clear_label(slot), 0, |tx| {
        let Some(media) = behaviour_editor::slot_value(tx, slot)? else {
            return Ok((None, Vec::new()));
        };
        behaviour_editor::set_media_slot(tx, slot, None)?;
        // After the slot is emptied, so "still referenced" means what it means once the slot has
        // let go of it.
        let deleted = delete_unreferenced_scenery(tx, media)?;
        Ok((deleted, deleted.into_iter().collect()))
    })?;
    Ok(deleted)
}

/// A file's stored thumbnail (a small WebP made at import), if it has one.
pub fn thumbnail(connection: &Connection, id: u64) -> Result<Option<Vec<u8>>> {
    use rusqlite::OptionalExtension;
    Ok(connection
        .query_row(
            "SELECT thumbnail FROM media WHERE id = ?",
            params![id],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()?
        .flatten())
}

/// The file types a slot's file picker offers, with the filter's name: a wallpaper has to be an
/// image, a splash may also be a video (`fill_media_slot_dialog` in the Tauri editor).
pub fn slot_file_filter(slot: &MediaSlot) -> (&'static str, &'static [&'static str]) {
    match slot {
        MediaSlot::StageAudio { .. }
        | MediaSlot::StageEntrySound { .. }
        | MediaSlot::StagePromptSound { .. } => (
            "Audio",
            &["mp3", "ogg", "wav", "flac", "m4a", "aac", "opus"],
        ),
        MediaSlot::Splash | MediaSlot::StageEntrySplash { .. } => (
            "Image or video",
            &[
                "png", "jpg", "jpeg", "webp", "avif", "bmp", "gif", "mp4", "webm", "mkv", "mov",
            ],
        ),
        MediaSlot::Wallpaper | MediaSlot::StageWallpaper { .. } => {
            ("Image", &["png", "jpg", "jpeg", "webp", "avif", "bmp"])
        }
    }
}

/// What importing a file into a slot did: the file the slot now holds (`added` if the import
/// brought it in, rather than finding the same bytes already in the pack), and the file it
/// displaced, if that one left the pack.
pub struct SlotImport {
    pub file: crate::pack::MediaFile,
    pub added: bool,
    pub deleted: Option<u64>,
}

/// Imports `path` into `slot`: encodes it, adds it (or finds the copy the pack already has), and
/// points the slot at it, as one undo entry. Not a synchronous edit like the rest of this module:
/// it runs ffmpeg, so it belongs on a background runtime, and its writes go through the pack's
/// own pool. Afterwards, `Session::slot_filled_by_import` brings the session up to date.
///
/// The gpui editor's way into `encode::process_file_into_slot`, which wants the Tauri app's
/// shared state: this supplies the pack, an encoder (detected once per process) and an upload
/// lock of its own. Once the gpui editor has a general import pipeline, the two should share one
/// lock, as the Tauri editor's commands do.
pub async fn import_into_slot(
    pack: std::sync::Arc<crate::pack::MediaPack>,
    path: &std::path::Path,
    slot: MediaSlot,
) -> Result<SlotImport> {
    use shared::encode::HardwareEncoder;
    use std::sync::{Arc, LazyLock};
    use tokio::sync::{Mutex, OnceCell, RwLock};

    static ENCODER: OnceCell<HardwareEncoder> = OnceCell::const_new();
    static UPLOAD_LOCK: LazyLock<Arc<RwLock<()>>> = LazyLock::new(|| Arc::new(RwLock::new(())));

    let encoder = ENCODER
        .get_or_init(|| async {
            tokio::task::spawn_blocking(HardwareEncoder::detect_and_test)
                .await
                .unwrap_or(HardwareEncoder::SoftwareFallback)
        })
        .await
        .clone();
    let state: crate::PackState = Arc::new(Mutex::new(Some(pack)));
    let events: crate::events::Events = Arc::new(|_| {});
    let label = slot_label(&slot).to_string();
    let (outcome, deleted) = crate::encode::process_file_into_slot(
        &state,
        path,
        &events,
        encoder,
        &UPLOAD_LOCK,
        slot,
        label,
    )
    .await?;
    Ok(SlotImport {
        added: matches!(outcome, crate::pack::AddOutcome::Added(_)),
        file: outcome.file().clone(),
        deleted,
    })
}
