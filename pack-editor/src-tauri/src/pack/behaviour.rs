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
use shared::{
    behaviour::{storage as behaviour_storage, Behaviour, MediaSlot, Patch},
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

    /// Applies one author action to the behaviour document, described as [`Patch`]es rather than
    /// sent as a replacement document.
    ///
    /// This is the only way the editor's front end writes behaviour, and the reason is that a
    /// whole-document write cannot be applied without overwriting. The document is also edited
    /// here in the backend -- media slots, tag renames, file removal -- so a front end that sent
    /// the version it happened to be holding would silently undo whichever of those landed while
    /// it was typing. Patching against the stored document means the two cannot collide, and the
    /// call sites no longer have to flush a pending write before every such command. See
    /// `design/behaviour-storage.md`.
    ///
    /// `label` names the entry in the undo list, so it is the author's action ("Edit caption")
    /// rather than the storage ("Edit pack behaviour"). Returns the stored document either way:
    /// the caller reconciles its optimistic copy against it.
    /// The stored behaviour document.
    pub async fn get_behaviour(&self) -> Result<Behaviour> {
        let _handle = self.saving.read().await;
        self.db_execute(move |conn| read_behaviour(&conn)).await
    }

    /// Replaces the whole document in one history entry.
    ///
    /// For a writer that really does produce a document rather than edit one: the Edgeware
    /// importer's converted output, and the slot fills it makes as its media lands. Author edits
    /// go through [`MediaPack::edit_behaviour`] instead, which describes what changed.
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

    /// What one behaviour edit produced: the stored document, and any scenery that left with it.
    ///
    /// `retiring` names media the edit *deliberately* lets go of — a timeline stage's wallpaper
    /// when the stage itself is being removed. Each one is dropped from the pack if it was only
    /// ever that slot's scenery and the patched document no longer refers to it, by exactly the
    /// rule `clear_media_slot` uses.
    ///
    /// Deliberate, rather than inferred from what the document stopped referencing, because the
    /// two are not the same thing and the difference is destructive: disabling the Experience
    /// section drops every stage on purpose *without* retiring their wallpapers, and a cleanup
    /// driven by "which references went away?" would delete all of them on toggle-off. See
    /// `design/behaviour-storage.md`, "Invariants to preserve".
    pub async fn edit_behaviour(
        &self,
        patches: Vec<Patch>,
        label: String,
        retiring: Vec<u64>,
        tag_actions: Vec<TagAction>,
    ) -> Result<BehaviourEdit> {
        let _handle = self.saving.read().await;
        let (edit, changed) = self
            .db_execute(move |mut connection| {
                history::record_with_media_refs(&mut connection, &label, 0, |tx| {
                    let stored = read_behaviour(tx)?;
                    let patched = stored.patched(&patches)?;

                    // Before the write, so the document is stored against the tag names the patch
                    // uses rather than re-creating the ones a rename just left behind.
                    let mut tags = TagActionOutcome::default();
                    run_tag_actions_before_write(tx, &tag_actions, &mut tags)?;

                    // Against the *patched* document, so "still referenced" means after the edit
                    // -- a wallpaper the retired stage shared with another stage stays.
                    let mut deleted_ids = Vec::new();
                    for media in &retiring {
                        if let Some(id) = delete_unreferenced_scenery(tx, &patched, *media)? {
                            deleted_ids.push(id);
                        }
                    }

                    // A patch that lands on the value already there is an `oninput` that changed
                    // nothing, not an edit. Writing it would cost an undo entry out of the
                    // hundred history keeps, evicting a real one off the bottom of the list.
                    let document_changed = patched != stored;
                    if document_changed {
                        write_behaviour(tx, &patched)?;
                    }
                    run_tag_actions_after_write(tx, &tag_actions, &mut tags)?;

                    // The retired file's bytes have to outlive the entry that dropped it, or undo
                    // would restore a slot pointing at media the collector has taken away. The tag
                    // actions' media is here for the same reason association-wide tag edits collect
                    // it (see `history::record_with_media_refs`).
                    let mut refs = deleted_ids.clone();
                    refs.extend(tags.media_refs);
                    let edit = BehaviourEdit {
                        behaviour: patched,
                        deleted_ids,
                        removed_tags: tags.removed,
                        renamed_tags: tags.renamed,
                    };
                    let changed = document_changed || !edit.deleted_ids.is_empty() || tags.changed;
                    if !changed {
                        return Ok(((edit, false), vec![]));
                    }
                    Ok(((edit, true), refs))
                })
            })
            .await?;
        if changed {
            self.mark_unsaved().await?;
        }
        Ok(edit)
    }
}

/// Points `slot` at `media_id`, marking that file as scenery if the slot is what brought it into
/// the pack, and dropping the file it displaced if that file was only ever this slot's. The body
/// of `fill_media_slot`, separated so it can run either in its own history entry or folded into an
/// open import.
///
/// Returns the updated behaviour and the id of the displaced file, if it deleted one.
pub(super) fn fill_media_slot_tx(
    tx: &rusqlite::Transaction<'_>,
    slot: &MediaSlot,
    media_id: u64,
    new_to_pack: bool,
) -> Result<(Behaviour, Option<u64>)> {
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

    let mut behaviour = read_behaviour(tx)?;
    let displaced = slot_value(&behaviour, slot);
    // Set, not fill: replacing what a slot points at is the whole of "Replace".
    if !set_slot(&mut behaviour, slot, Some(media_id)) {
        bail!("That stage is no longer in the timeline");
    }
    // Replacing a slot's file is the other way it stops being used, and it has to clean up after
    // itself exactly as clearing does -- otherwise every Replace leaves the old wallpaper in the
    // pack as a file marked out of popups and referenced by nothing.
    let deleted = match displaced {
        Some(previous) if previous != media_id => {
            delete_unreferenced_scenery(tx, &behaviour, previous)?
        }
        _ => None,
    };
    write_behaviour(tx, &behaviour)?;
    Ok((behaviour, deleted))
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

/// Reads what a slot currently points at.
pub(super) fn slot_value(behaviour: &Behaviour, slot: &MediaSlot) -> Option<u64> {
    match slot {
        MediaSlot::Wallpaper => behaviour.content.wallpaper,
        MediaSlot::Splash => behaviour.content.splash,
        MediaSlot::StageWallpaper { stage } => {
            behaviour
                .experience
                .as_ref()?
                .timeline
                .stages
                .iter()
                .find(|candidate| &candidate.id == stage)?
                .content
                .wallpaper
        }
        MediaSlot::StageAudio { stage } => {
            behaviour
                .experience
                .as_ref()?
                .timeline
                .stages
                .iter()
                .find(|candidate| &candidate.id == stage)?
                .content
                .audio
        }
        MediaSlot::StageEntrySplash { stage } => {
            behaviour
                .experience
                .as_ref()?
                .timeline
                .stages
                .iter()
                .find(|candidate| &candidate.id == stage)?
                .on_enter
                .splash
        }
        MediaSlot::StageEntrySound { stage } => {
            behaviour
                .experience
                .as_ref()?
                .timeline
                .stages
                .iter()
                .find(|candidate| &candidate.id == stage)?
                .on_enter
                .sound
        }
        MediaSlot::StagePromptSound { stage } => {
            behaviour
                .experience
                .as_ref()?
                .timeline
                .stages
                .iter()
                .find(|candidate| &candidate.id == stage)?
                .prompt
                .sound
        }
    }
}

/// Writes a slot outright, unlike `Behaviour::fill_media_reference` (which won't overwrite): the
/// author pointing a slot somewhere new is exactly the case that has to win. Returns false if the
/// slot doesn't exist -- only possible for a stage deleted from under the editor.
pub(super) fn set_slot(behaviour: &mut Behaviour, slot: &MediaSlot, media: Option<u64>) -> bool {
    let target = match slot {
        MediaSlot::Wallpaper => &mut behaviour.content.wallpaper,
        MediaSlot::Splash => &mut behaviour.content.splash,
        MediaSlot::StageWallpaper { stage } => {
            let Some(target) = behaviour.experience.as_mut().and_then(|experience| {
                experience
                    .timeline
                    .stages
                    .iter_mut()
                    .find(|candidate| &candidate.id == stage)
            }) else {
                return false;
            };
            &mut target.content.wallpaper
        }
        MediaSlot::StageAudio { stage } => {
            let Some(target) = behaviour.experience.as_mut().and_then(|experience| {
                experience
                    .timeline
                    .stages
                    .iter_mut()
                    .find(|candidate| &candidate.id == stage)
            }) else {
                return false;
            };
            target.content.audio_random = false;
            &mut target.content.audio
        }
        MediaSlot::StageEntrySplash { stage } => {
            let Some(target) = behaviour.experience.as_mut().and_then(|experience| {
                experience
                    .timeline
                    .stages
                    .iter_mut()
                    .find(|candidate| &candidate.id == stage)
            }) else {
                return false;
            };
            &mut target.on_enter.splash
        }
        MediaSlot::StageEntrySound { stage } => {
            let Some(target) = behaviour.experience.as_mut().and_then(|experience| {
                experience
                    .timeline
                    .stages
                    .iter_mut()
                    .find(|candidate| &candidate.id == stage)
            }) else {
                return false;
            };
            &mut target.on_enter.sound
        }
        MediaSlot::StagePromptSound { stage } => {
            let Some(target) = behaviour.experience.as_mut().and_then(|experience| {
                experience
                    .timeline
                    .stages
                    .iter_mut()
                    .find(|candidate| &candidate.id == stage)
            }) else {
                return false;
            };
            &mut target.prompt.sound
        }
    };
    *target = media;
    true
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
