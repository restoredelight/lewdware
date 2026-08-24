//! Reading and writing the pack's behaviour document, and the media slots it addresses.

use super::labels::{
    apply_tag, read_behaviour, run_tag_actions_after_write, run_tag_actions_before_write,
    TagActionOutcome,
};
use super::media::delete_unreferenced_scenery;
use super::*;

use crate::history;
use shared::behaviour::editor as behaviour_editor;
use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};
use shared::{
    behaviour::{storage as behaviour_storage, Behaviour, MediaSlot},
    tags,
};

impl MediaPack {
    /// Writes a named blob into the pack's generic `pack_data` table (e.g. `"behaviour"` for the
    /// Edgeware importer's converted behaviour.json) -- `name` is the table's primary key, so a
    /// repeat write for the same name replaces it.
    pub async fn set_pack_data(&self, name: &str, blob: Vec<u8>) -> Result<()> {
        let _handle = self.saving.read().await;
        let name = name.to_string();
        let label = format!("Edit pack data “{name}”");
        self.db_execute(move |mut connection| {
            history::record(&mut connection, &label, 0, &[], |tx| {
                tx.execute(
                    "INSERT OR REPLACE INTO pack_data (name, blob) VALUES (?, ?)",
                    params![name, blob],
                )?;
                Ok(())
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    /// Reads a named blob from the pack's `pack_data` table (e.g. `"behaviour"`). `None` if the
    /// pack doesn't carry an entry with this name -- a pack predating this key, or never written
    /// by a tool that populates it, is expected, not an error. Mirrors the engine's own
    /// `lewdware::media::pack::MediaPack::get_pack_data`.
    pub async fn get_pack_data(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let _handle = self.saving.read().await;
        let name = name.to_string();
        self.db_execute(move |conn| {
            conn.query_row(
                "SELECT blob FROM pack_data WHERE name = ?",
                params![name],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        })
        .await
    }

    /// The stored behaviour document, whole.
    ///
    /// For the writers that still deal in whole documents — the Edgeware importer, and saving.
    /// The editor's surfaces read through the typed queries instead.
    pub async fn get_behaviour(&self) -> Result<Behaviour> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| read_behaviour(&conn)).await
    }

    /// Replaces the whole document in one history entry.
    ///
    /// For a writer that really does produce a document rather than edit one: the Edgeware
    /// importer's converted output. Author edits go through [`MediaPack::behaviour_edit`].
    pub async fn replace_behaviour(&self, behaviour: Behaviour, label: String) -> Result<()> {
        let _handle = self.saving.read().await;
        self.db_execute(move |mut connection| {
            history::record(&mut connection, &label, 0, &[], |tx| {
                write_behaviour(tx, &behaviour)
            })
        })
        .await?;
        self.mark_unsaved().await
    }

    /// Points each of `slots` at `media_id`, but only those still empty.
    ///
    /// The Edgeware importer's tail: the converted document named a file for each slot, and this is
    /// the moment that file finally arrives. Fill, not set — an author who picked their own
    /// wallpaper while the import was running keeps it.
    ///
    /// Returns the slots it actually filled, which is what the front end is told about.
    pub async fn fill_empty_slots(
        &self,
        slots: Vec<MediaSlot>,
        media_id: u64,
        label: String,
    ) -> Result<Vec<MediaSlot>> {
        let _handle = self.saving.read().await;
        let filled = self
            .db_execute(move |mut connection| {
                let slots = slots.clone();
                history::record_with_media_refs(&mut connection, &label, 0, move |tx| {
                    let mut filled = Vec::new();
                    for slot in slots {
                        if behaviour_editor::slot_value(tx, &slot)?.is_some() {
                            continue;
                        }
                        // A slot on a stage that has since been removed simply goes unfilled --
                        // this is a late arrival, not an author action, so it has nothing to say.
                        if behaviour_editor::set_media_slot(tx, &slot, Some(media_id)).is_ok() {
                            filled.push(slot);
                        }
                    }
                    let refs = if filled.is_empty() {
                        vec![]
                    } else {
                        vec![media_id]
                    };
                    Ok((filled, refs))
                })
            })
            .await?;
        if !filled.is_empty() {
            self.mark_unsaved().await?;
        }
        Ok(filled)
    }

    /// Runs one read against the behaviour tables.
    ///
    /// The query half of [`MediaPack::behaviour_edit`]: takes the saving lock, hops to the blocking
    /// pool, and hands the closure a connection. Every typed query goes through it so none of them
    /// has to remember the lock.
    pub async fn behaviour_query<T, F>(&self, query: F) -> Result<T>
    where
        T: Send + 'static,
        F: Fn(&rusqlite::Connection) -> Result<T> + Send + 'static,
    {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| query(&conn)).await
    }

    /// Runs one author action against the behaviour tables.
    ///
    /// Every typed behaviour mutation goes through here, because they all need the same envelope
    /// and getting it subtly different in twenty places is exactly how the old whole-document
    /// writer accumulated its defects. The envelope is:
    ///
    /// - **One transaction, one undo entry.** `edit` runs inside `history::record_with_media_refs`,
    ///   which records a changeset the size of what actually changed and skips the entry entirely
    ///   when nothing did. `label` is the author's word for what they did ("Edit caption"), because
    ///   that is what they will look for in the undo list.
    /// - **`tag_actions` run in the same transaction**, so renaming a stage and renaming the tag it
    ///   owns cannot come apart under undo.
    /// - **`retiring` names media the action *deliberately* lets go of** — a stage's wallpaper when
    ///   the stage itself is going. Each is dropped if it was only ever that slot's scenery.
    ///   Deliberate rather than inferred, because the two are not the same thing and the difference
    ///   is destructive: switching the timeline off drops every stage on purpose *without* retiring
    ///   their wallpapers. See `design/behaviour-storage.md`, "Invariants to preserve".
    ///
    /// `edit` reports whether it changed anything. Returning `false` for an edit that landed on the
    /// value already stored is what keeps an `oninput` that changed nothing from spending one of
    /// the hundred entries undo keeps.
    pub async fn behaviour_edit<F>(&self, action: BehaviourAction<F>) -> Result<BehaviourOutcome>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<bool> + Send + 'static,
    {
        let _handle = self.saving.read().await;
        let BehaviourAction {
            label,
            retiring,
            tag_actions,
            edit,
        } = action;
        let mut edit = Some(edit);
        let (outcome, changed) = self
            .db_execute(move |mut connection| {
                let edit = edit.take().expect("the edit runs once");
                let retiring = retiring.clone();
                let tag_actions = tag_actions.clone();
                history::record_with_media_refs(&mut connection, &label, 0, move |tx| {
                    // Before the edit, so the write lands against the tag names the action uses
                    // rather than re-creating the ones a rename just left behind.
                    let mut tags = TagActionOutcome::default();
                    run_tag_actions_before_write(tx, &tag_actions, &mut tags)?;

                    let document_changed = edit(tx)?;

                    // Asked *after* the edit, so "still referenced" means what it means once the
                    // action has happened -- a wallpaper the retired stage shared with another
                    // stage stays.
                    let mut deleted_ids = Vec::new();
                    for media in &retiring {
                        if let Some(id) = delete_unreferenced_scenery(tx, *media)? {
                            deleted_ids.push(id);
                        }
                    }
                    run_tag_actions_after_write(tx, &tag_actions, &mut tags)?;

                    // The retired file's bytes have to outlive the entry that dropped it, or undo
                    // would restore a slot pointing at media the collector has taken away.
                    let mut refs = deleted_ids.clone();
                    refs.extend(tags.media_refs);
                    let outcome = BehaviourOutcome {
                        deleted_ids,
                        removed_tags: tags.removed,
                        renamed_tags: tags.renamed,
                    };
                    let changed =
                        document_changed || !outcome.deleted_ids.is_empty() || tags.changed;
                    if !changed {
                        return Ok(((outcome, false), vec![]));
                    }
                    Ok(((outcome, true), refs))
                })
            })
            .await?;
        if changed {
            self.mark_unsaved().await?;
        }
        Ok(outcome)
    }
}

/// Points `slot` at `media_id`, marking that file as scenery if the slot is what brought it into
/// the pack, and dropping the file it displaced if that file was only ever this slot's. The body
/// of `fill_media_slot`, separated so it can run either in its own history entry or folded into an
/// open import.
///
/// Returns the id of the displaced file, if it deleted one.
pub(super) fn fill_media_slot_tx(
    tx: &rusqlite::Transaction<'_>,
    slot: &MediaSlot,
    media_id: u64,
    new_to_pack: bool,
) -> Result<Option<u64>> {
    // The row still has to exist: a slot may only point at media the pack really has, and the
    // picker's answer can be stale by the time it gets here.
    let exists = tx
        .query_row(
            "SELECT 1 FROM media WHERE id = ? AND deleted = 0",
            params![media_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        bail!("That media file is no longer in the pack");
    }
    if new_to_pack {
        apply_tag(tx, media_id, tags::EXPLICIT_ONLY_TAG)?;
    }

    let displaced = behaviour_editor::slot_value(tx, slot)?;
    // Set, not fill: replacing what a slot points at is the whole of "Replace".
    behaviour_editor::set_media_slot(tx, slot, Some(media_id))?;
    // Replacing a slot's file is the other way it stops being used, and it has to clean up after
    // itself exactly as clearing does -- otherwise every Replace leaves the old wallpaper in the
    // pack as a file marked out of popups and referenced by nothing.
    let deleted = match displaced {
        Some(previous) if previous != media_id => delete_unreferenced_scenery(tx, previous)?,
        _ => None,
    };
    Ok(deleted)
}

/// The history label a slot operation records under. Named for what the author did, not for the
/// field: "Set wallpaper" is what they'll look for in the undo list.
pub(super) fn slot_label(slot: &MediaSlot) -> &'static str {
    match slot {
        MediaSlot::Wallpaper | MediaSlot::StageWallpaper { .. } => "Set wallpaper",
        MediaSlot::Splash => "Set splash",
        MediaSlot::StageAudio { .. } => "Set stage audio",
        MediaSlot::StageEntrySplash { .. } => "Set stage splash",
        MediaSlot::StageEntrySound { .. } | MediaSlot::StagePromptSound { .. } => "Set stage sound",
    }
}

pub(super) fn write_behaviour(tx: &rusqlite::Transaction<'_>, behaviour: &Behaviour) -> Result<()> {
    behaviour_storage::write(tx, behaviour)
}

/// Applies `edit` to the stored behaviour, recording its own history entry only when the edit
/// reported a change. Returns the behaviour either way, since callers hand it back to the front
/// end to replace its copy.
pub(super) fn conn_write_behaviour_if_changed(
    conn: &mut rusqlite::Connection,
    label: &str,
    edit: impl FnOnce(&mut Behaviour) -> bool,
) -> Result<Behaviour> {
    let mut behaviour = read_behaviour(conn)?;
    if !edit(&mut behaviour) {
        return Ok(behaviour);
    }
    let updated = behaviour.clone();
    history::record(conn, label, 0, &[], |tx| write_behaviour(tx, &updated))?;
    Ok(behaviour)
}
