//! Reading and writing the pack's behaviour document, and the media slots it addresses.

use super::labels::{
    apply_tag, read_behaviour, run_tag_actions_after_write, run_tag_actions_before_write,
    TagActionOutcome,
};
use super::media::delete_unreferenced_scenery;
use super::*;

use crate::history;
use anyhow::{bail, Result};
use rusqlite::{params, OptionalExtension};
use shared::behaviour::editor as behaviour_editor;
use shared::{
    behaviour::{Behaviour, MediaSlot},
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

    /// The stored behaviour document, whole. The editor's surfaces read through typed queries
    /// instead; whole-document reads remain useful at serialization boundaries and in tests.
    pub async fn get_behaviour(&self) -> Result<Behaviour> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| read_behaviour(&conn)).await
    }

    /// Seeds a newly-created draft with converted behaviour and metadata.
    ///
    /// Conversion establishes the draft's initial contents; it is not an author edit and must not
    /// occupy undo history. The pack remains unsaved, so the first later author edit still gets an
    /// ordinary history entry relative to this imported baseline.
    pub async fn initialize_from_conversion(
        &self,
        behaviour: Behaviour,
        metadata: Metadata,
    ) -> Result<()> {
        let _handle = self.saving.read().await;
        let metadata_for_db = metadata.clone();
        self.db_execute(move |mut connection| {
            let tx =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            write_behaviour(&tx, &behaviour)?;
            metadata_for_db.update(&tx)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
        *self.metadata.write().unwrap() = metadata;
        self.mark_unsaved().await
    }

    /// Replaces the whole document in one history entry.
    ///
    /// Author edits normally go through [`MediaPack::behaviour_edit`]. This remains available for
    /// callers that genuinely need an undoable whole-document replacement and for test setup.
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
    pub async fn fill_empty_slots_from_conversion(
        &self,
        slots: Vec<MediaSlot>,
        media_id: u64,
    ) -> Result<Vec<MediaSlot>> {
        let _handle = self.saving.read().await;
        let filled = self
            .db_execute(move |mut connection| {
                let tx = connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let mut filled = Vec::new();

                for slot in &slots {
                    if behaviour_editor::slot_value(&tx, slot)?.is_some() {
                        continue;
                    }

                    if behaviour_editor::set_media_slot(&tx, slot, Some(media_id)).is_ok() {
                        filled.push(slot.clone());
                    }
                }

                tx.commit()?;
                Ok(filled)
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
    /// - **The action is prepared first**, inside the same transaction, and what it works out is
    ///   what the rest of it runs on. [`Prepared::tag_actions`] run before the document write and
    ///   [`Prepared::retiring`] after it, so a rename and the tag it renames cannot come apart under
    ///   undo, and "is this still referenced?" is asked of the document the edit produced.
    /// - **The two phases share one decision, by value.** `prepare` returns it and `apply` receives
    ///   it. Restricting a stage names its new tag once; naming it again in the second phase would
    ///   give a different answer, because the first has already taken the name.
    ///
    /// `apply` writes; it does not first check whether writing is necessary. An `oninput` that
    /// lands on the value already stored still runs its UPDATE, and still spends no undo entry and
    /// leaves the pack saved -- because the changeset it produces is empty, which is what both of
    /// those now key off. Asking the database what it already holds, so as to not write it again,
    /// only bought a second answer to a question the changeset was already answering.
    pub async fn behaviour_edit<P: Send + 'static>(
        &self,
        action: BehaviourAction<P>,
    ) -> Result<BehaviourOutcome> {
        let _handle = self.saving.read().await;
        let BehaviourAction {
            label,
            prepare,
            apply,
        } = action;
        let mut parts = Some((prepare, apply));
        let (outcome, changed) = self
            .db_execute(move |mut connection| {
                let (prepare, apply) = parts.take().expect("the edit runs once");
                history::record_with_media_refs(&mut connection, &label, 0, move |tx| {
                    // Asked of the pack as it stands *now*, before anything is written: what this
                    // stage's scenery is, and what its tag is called, are questions only the
                    // pre-edit document can answer. One read, so the answers agree with each other.
                    let Prepared {
                        value,
                        tag_actions,
                        retiring,
                    } = prepare(tx)?;

                    // Before the edit, so the write lands against the tag names the action uses
                    // rather than re-creating the ones a rename just left behind.
                    let mut tags = TagActionOutcome::default();
                    run_tag_actions_before_write(tx, &tag_actions, &mut tags)?;

                    // Handed what `prepare` worked out, so a decision the tag phase already acted
                    // on is the same one the rows are written from.
                    apply(tx, value)?;

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
                        media_tags: tags.media_tags,
                    };
                    Ok((outcome, refs))
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
    behaviour.write(tx)
}
