//! Behaviour-document edits: media slots, stages, captions, and what they retire.

use shared::behaviour::{
    Behaviour, ContentGroup, ContentSelection, EndStrategy, EventSchedule, Events, Experience,
    Interval, Stage, StageEnd, TextItem, Timeline, Transition, WebLink,
};
use tempfile::tempdir;

use super::*;
use rusqlite::Transaction;
use shared::behaviour::editor::{self as behaviour_editor, PoolKind};
use shared::behaviour::MediaSlot;
use shared::encode::EncodedFile;

/// Importing a file into a slot is one action to the author, so it has to be one undo step --
/// not "import a file" followed by "set the wallpaper". The import half can't be a changeset
/// (undo soft-deletes the media so its bytes survive for redo), so the slot write rides on
/// the same entry instead; this checks both halves really do move together, in both
/// directions.
/// A caption with nothing else said about it, which is all these tests need one to be.
fn caption(text: &str) -> TextItem {
    TextItem {
        text: text.to_string(),
        tags: vec![],
        timeout_seconds: None,
        summary: None,
    }
}

#[tokio::test]
async fn importing_into_a_slot_undoes_and_redoes_as_one_step() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("fold.lwpack"), data_dir.path(), "Fold").await;

    let bytes = b"scenery bytes";
    let staged = pack.dir.join("media").join("wallpaper.opus");
    tokio::fs::write(&staged, bytes).await.unwrap();
    let history_id = pack.begin_media_import().await.unwrap();
    let media = pack
        .add_file_to_import(
            EncodedFile {
                info: FileInfo::Audio { duration: 1.0 },
                thumbnail: None,
                path: staged.clone(),
                artists: vec![],
                source_url: None,
            },
            Path::new("wallpaper.png"),
            blake3::hash(bytes),
            Some(history_id),
        )
        .await
        .unwrap()
        .unwrap();
    let id = media.file().id;
    pack.fill_media_slot_during_import(MediaSlot::Wallpaper, id, history_id, true)
        .await
        .unwrap();
    let status = pack
        .finish_media_import(history_id, Some("Set wallpaper".to_string()))
        .await
        .unwrap();

    assert_eq!(status.undo_label.as_deref(), Some("Set wallpaper"));
    assert!(status.can_undo);
    let stored = || async { pack.get_behaviour().await.unwrap() };
    assert_eq!(stored().await.content.wallpaper, Some(id));

    // One undo takes the slot *and* the file -- and leaves the bytes on disk, so redo can
    // bring them back (the reason the import half stays a visibility entry).
    pack.undo().await.unwrap();
    assert!(pack.get_files().await.unwrap().is_empty());
    assert_eq!(stored().await.content.wallpaper, None);
    assert!(staged.exists());
    assert!(!pack.history_status().await.unwrap().can_undo);

    pack.redo().await.unwrap();
    assert_eq!(pack.get_files().await.unwrap()[0].id, id);
    assert_eq!(stored().await.content.wallpaper, Some(id));
    assert_eq!(
        pack.get_tags(id).await.unwrap(),
        vec![tags::EXPLICIT_ONLY_TAG]
    );
}

/// The same fold when the file was already in the pack: nothing new to make visible, but the
/// slot write is still a real edit and must not be discarded as an empty import.
#[tokio::test]
async fn filling_a_slot_from_an_existing_file_is_still_one_undoable_step() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("dup.lwpack"), data_dir.path(), "Dup").await;
    let existing = insert_staged_audio(&pack, b"already here").await;

    let history_id = pack.begin_media_import().await.unwrap();
    pack.fill_media_slot_during_import(MediaSlot::Splash, existing, history_id, false)
        .await
        .unwrap();
    let status = pack
        .finish_media_import(history_id, Some("Set splash".to_string()))
        .await
        .unwrap();

    assert_eq!(status.undo_label.as_deref(), Some("Set splash"));
    let stored = || async { pack.get_behaviour().await.unwrap() };
    assert!(stored().await.content.splash.is_some());

    pack.undo().await.unwrap();
    assert_eq!(stored().await.content.splash, None);
    // The file was never this import's to remove -- it was already in the pack.
    assert!(pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == existing));

    pack.redo().await.unwrap();
    assert!(stored().await.content.splash.is_some());
}

/// The lifecycle rule: a file that exists only to be scenery leaves when its slot does, and a
/// file that is also real content doesn't.
#[tokio::test]
async fn clearing_a_slot_deletes_scenery_but_keeps_real_content() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("slots.lwpack"), data_dir.path(), "Slots").await;

    // Filled from the slot, so it carries the marker: only ever scenery.
    let scenery = insert_staged_audio(&pack, b"scenery").await;
    pack.fill_media_slot(MediaSlot::Wallpaper, scenery, true)
        .await
        .unwrap();
    let behaviour = pack.get_behaviour().await.unwrap();
    let slot_media = behaviour.content.wallpaper.unwrap();
    assert!(pack.get_tags(scenery).await.unwrap() == vec![tags::EXPLICIT_ONLY_TAG]);

    let deleted = pack.clear_media_slot(MediaSlot::Wallpaper).await.unwrap();
    let behaviour = pack.get_behaviour().await.unwrap();
    assert_eq!(behaviour.content.wallpaper, None);
    assert_eq!(deleted, Some(scenery));
    assert!(!pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == scenery));

    // Same again with media that already belonged to the pack: selecting it explicitly does
    // not change its pool membership, so it remains real content when the slot is cleared.
    let content = insert_staged_audio(&pack, b"content").await;
    pack.fill_media_slot(MediaSlot::Splash, content, false)
        .await
        .unwrap();
    assert!(pack.get_tags(content).await.unwrap().is_empty());

    let deleted = pack.clear_media_slot(MediaSlot::Splash).await.unwrap();
    let behaviour = pack.get_behaviour().await.unwrap();
    assert_eq!(behaviour.content.splash, None);
    assert_eq!(deleted, None);
    assert!(pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == content));
    assert_eq!(slot_media, scenery);
}

/// The same rule for the other way a slot lets go of a file. Replacing a wallpaper is far more
/// common than clearing one, and leaving the old file behind would put it in the pack marked
/// out of popups and referenced by nothing -- invisible in Popups, absent from every slot and
/// pool, and explicable only in All media.
#[tokio::test]
async fn replacing_a_slot_deletes_the_scenery_it_displaced() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(
        &tmp.path().join("replace.lwpack"),
        data_dir.path(),
        "Replace",
    )
    .await;

    let first = insert_staged_audio(&pack, b"first wallpaper").await;
    pack.fill_media_slot(MediaSlot::Wallpaper, first, true)
        .await
        .unwrap();

    let second = insert_staged_audio(&pack, b"second wallpaper").await;
    let deleted = pack
        .fill_media_slot(MediaSlot::Wallpaper, second, true)
        .await
        .unwrap();
    let behaviour = pack.get_behaviour().await.unwrap();

    assert_eq!(
        deleted,
        Some(first),
        "the displaced scenery left with the slot"
    );
    assert!(behaviour.content.wallpaper.is_some());
    let files = pack.get_files().await.unwrap();
    assert!(!files.iter().any(|f| f.id == first));
    assert!(files.iter().any(|f| f.id == second));

    // Undo puts both halves back: the slot points at the first file again, and the file is
    // there to point at.
    pack.undo().await.unwrap();
    let restored = pack.get_files().await.unwrap();
    assert!(restored.iter().any(|f| f.id == first));
}

/// ...but only when the displaced file was this slot's alone. A file another slot still names,
/// or one the author has un-marked as real content, is theirs and stays.
#[tokio::test]
async fn replacing_a_slot_keeps_a_file_something_else_still_wants() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("keep.lwpack"), data_dir.path(), "Keep").await;

    // Wallpaper and splash both point at it; replacing one leaves the other's.
    let shared_file = insert_staged_audio(&pack, b"used twice").await;
    pack.fill_media_slot(MediaSlot::Wallpaper, shared_file, true)
        .await
        .unwrap();
    pack.fill_media_slot(MediaSlot::Splash, shared_file, false)
        .await
        .unwrap();

    let replacement = insert_staged_audio(&pack, b"new wallpaper").await;
    let deleted = pack
        .fill_media_slot(MediaSlot::Wallpaper, replacement, true)
        .await
        .unwrap();
    assert_eq!(deleted, None, "the splash still names it");
    assert!(pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == shared_file));

    // Un-marked media is real content, whatever displaced it.
    let ordinary = insert_staged_audio(&pack, b"real content").await;
    pack.fill_media_slot(MediaSlot::Splash, ordinary, false)
        .await
        .unwrap();
    let last = insert_staged_audio(&pack, b"final splash").await;
    let deleted = pack
        .fill_media_slot(MediaSlot::Splash, last, true)
        .await
        .unwrap();
    assert_eq!(
        deleted, None,
        "un-marked media is the author's, not the slot's"
    );
    assert!(pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == ordinary));
}

/// The other half of the lifecycle rule: only the operation that *brought a file in* may call
/// it scenery.
///
/// Picking a wallpaper whose bytes the pack already holds resolves to the existing file (see
/// `AddOutcome::Duplicate`). Marking that file would take ordinary media out of the media grid
/// and, when the slot was later emptied, delete it outright -- losing a file the author
/// imported on purpose, for the crime of also being the wallpaper.
#[tokio::test]
async fn filling_a_slot_from_media_already_in_the_pack_leaves_it_ordinary_content() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("dedup.lwpack"), data_dir.path(), "Dedup").await;
    let existing = insert_staged_audio(&pack, b"already the author's").await;

    pack.fill_media_slot(MediaSlot::Wallpaper, existing, false)
        .await
        .unwrap();
    assert!(
        pack.get_tags(existing).await.unwrap().is_empty(),
        "a file the pack already had is not scenery"
    );

    let deleted = pack.clear_media_slot(MediaSlot::Wallpaper).await.unwrap();
    let behaviour = pack.get_behaviour().await.unwrap();
    assert_eq!(behaviour.content.wallpaper, None);
    assert_eq!(deleted, None, "emptying the slot must not delete the file");
    assert!(pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == existing));
}

#[tokio::test]
async fn audio_role_is_an_undoable_popup_marker() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(
        &tmp.path().join("audio-role.lwpack"),
        data_dir.path(),
        "Audio",
    )
    .await;
    let first = insert_staged_audio(&pack, b"first audio").await;
    let second = insert_staged_audio(&pack, b"second audio").await;

    pack.set_popup_audio(vec![first, second], true)
        .await
        .unwrap();
    assert_eq!(
        pack.get_tags(first).await.unwrap(),
        vec![tags::POPUP_AUDIO_TAG]
    );
    assert_eq!(
        pack.get_tags(second).await.unwrap(),
        vec![tags::POPUP_AUDIO_TAG]
    );
    assert_eq!(
        pack.history_status().await.unwrap().undo_label.as_deref(),
        Some("Move audio to Popup")
    );

    pack.undo().await.unwrap();
    assert!(pack.get_tags(first).await.unwrap().is_empty());
    assert!(pack.get_tags(second).await.unwrap().is_empty());

    pack.redo().await.unwrap();
    pack.set_popup_audio(vec![first], false).await.unwrap();
    assert!(pack.get_tags(first).await.unwrap().is_empty());
    assert_eq!(
        pack.get_tags(second).await.unwrap(),
        vec![tags::POPUP_AUDIO_TAG]
    );
}

/// A base wallpaper reused by a timeline stage is Edgeware's ordinary case, so clearing one
/// slot must not delete a file the other still points at.
#[tokio::test]
async fn clearing_a_slot_keeps_a_file_another_slot_still_references() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("shared.lwpack"), data_dir.path(), "Shared").await;
    let shared = insert_staged_audio(&pack, b"shared").await;

    let behaviour = Behaviour {
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![Stage {
                    id: "stage-1".to_string(),
                    label: "Stage 1".to_string(),
                    end: None,
                    content: ContentSelection::default(),
                    events: Events::default(),
                    movement: None,
                    mitosis: None,
                    on_enter: Default::default(),
                    prompt: Default::default(),
                }],
                transitions: vec![],
            },
            label: None,
        }),
        ..Behaviour::default()
    };
    pack.replace_behaviour(behaviour.clone(), "Seed behaviour".to_string())
        .await
        .unwrap();

    pack.fill_media_slot(MediaSlot::Wallpaper, shared, true)
        .await
        .unwrap();
    pack.fill_media_slot(
        MediaSlot::StageWallpaper {
            stage: "stage-1".to_string(),
        },
        shared,
        false,
    )
    .await
    .unwrap();

    let deleted = pack
        .clear_media_slot(MediaSlot::StageWallpaper {
            stage: "stage-1".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(deleted, None, "the base wallpaper still points at it");
    assert!(read_pack_behaviour(&pack).await.content.wallpaper.is_some());
    assert!(pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == shared));

    // With the last reference gone it is scenery again, and goes.
    let deleted = pack.clear_media_slot(MediaSlot::Wallpaper).await.unwrap();
    assert_eq!(deleted, Some(shared));
}

/// The reason behaviour is edited by typed command and not by replacement document.
///
/// The front end holds a copy and edits it as the author types. If it saved by sending that
/// copy, a slot the backend filled in between -- an import finishing, a wallpaper chosen on
/// another tab -- would be gone the moment the debounce fired, because the copy predates it.
/// A patch names only what the author touched, so the two edits compose.
#[tokio::test]
async fn an_edit_cannot_undo_a_backend_change_it_never_saw() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("race.lwpack"), data_dir.path(), "Race").await;
    let file = insert_staged_audio(&pack, b"wallpaper").await;

    // What the front end is holding: a document with no wallpaper.
    let held = read_pack_behaviour(&pack).await;
    assert_eq!(held.content.wallpaper, None);

    pack.fill_media_slot(MediaSlot::Wallpaper, file, true)
        .await
        .unwrap();

    // The author was typing a caption throughout, and the debounce fires now.
    pack.behaviour_edit(BehaviourAction::new("Edit caption", |tx: &Transaction<'_>| {
        behaviour_editor::add_text_item(tx, PoolKind::Caption, &caption("typed"))?;
        Ok(true)
    }))
    .await
    .unwrap();

    let stored = read_pack_behaviour(&pack).await;
    assert_eq!(stored.content.captions[0].text, "typed");
    assert!(
        stored.content.wallpaper.is_some(),
        "the slot filled while the author was typing has to survive the caption edit"
    );
}

/// An `oninput` that fires without changing anything is not an edit. Recording it would spend
/// one of the hundred entries history keeps and push a real one off the bottom of the list.
#[tokio::test]
async fn an_edit_that_changes_nothing_records_no_history_entry() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("noop.lwpack"), data_dir.path(), "Noop").await;

    let id = std::sync::Arc::new(std::sync::Mutex::new(0i64));
    let created = id.clone();
    pack.behaviour_edit(BehaviourAction::new("Add caption", move |tx: &Transaction<'_>| {
        *created.lock().unwrap() =
            behaviour_editor::add_text_item(tx, PoolKind::Caption, &caption("typed"))?;
        Ok(true)
    }))
    .await
    .unwrap();
    let id = *id.lock().unwrap();

    pack.behaviour_edit(BehaviourAction::new("Edit caption", move |tx: &Transaction<'_>| {
        behaviour_editor::set_text_item_text(tx, id, "edited")
    }))
    .await
    .unwrap();
    let after_real_edit = pack.history_status().await.unwrap();
    assert_eq!(after_real_edit.undo_label.as_deref(), Some("Edit caption"));

    // The same value again: an `oninput` that changed nothing.
    pack.behaviour_edit(BehaviourAction::new("Edit caption", move |tx: &Transaction<'_>| {
        behaviour_editor::set_text_item_text(tx, id, "edited")
    }))
    .await
    .unwrap();
    let after_noop = pack.history_status().await.unwrap();
    assert_eq!(
        after_noop.undo_label, after_real_edit.undo_label,
        "the second write set the same value, so there is nothing to undo past the first"
    );

    // One entry, not two: undoing once takes back the edit, not half of it.
    pack.undo().await.unwrap();
    let behaviour = read_pack_behaviour(&pack).await;
    assert_eq!(behaviour.content.captions[0].text, "typed");

    // And the add is still its own entry underneath.
    pack.undo().await.unwrap();
    assert!(read_pack_behaviour(&pack).await.content.captions.is_empty());
}

/// Leaving one stage can require a replacement tag for another stage that shared its only
/// tag. The document patch and both media-tag changes are one author action and must undo as
/// one; otherwise undo can restore a membership without restoring the stage selection that
/// makes it mean anything (or the reverse).
#[tokio::test]
async fn a_stage_membership_rewrite_is_one_undoable_edit() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(
        &tmp.path().join("membership.lwpack"),
        data_dir.path(),
        "Membership",
    )
    .await;
    let media = insert_staged_audio(&pack, b"member").await;
    pack.add_tags(media, vec!["shared".to_string()])
        .await
        .unwrap();

    let make_stage = |id: &str| Stage {
        id: id.to_string(),
        label: id.to_string(),
        end: None,
        content: ContentSelection {
            tags: Some(vec!["shared".to_string()]),
            owned_tag: None,
            wallpaper: None,
            audio: None,
            audio_random: false,
        },
        events: Events::default(),
        movement: None,
        mitosis: None,
        on_enter: Default::default(),
        prompt: Default::default(),
    };
    pack.replace_behaviour(
        Behaviour {
            experience: Some(Experience {
                timeline: Timeline {
                    stages: vec![make_stage("leave"), make_stage("keep")],
                    transitions: vec![],
                },
                label: None,
            }),
            ..Behaviour::default()
        },
        "Seed behaviour".to_string(),
    )
    .await
    .unwrap();

    pack.behaviour_edit(
        BehaviourAction::new("Remove from leave", |tx: &Transaction<'_>| {
            behaviour_editor::set_stage_content_tags(
                tx,
                "keep",
                Some(&["shared".to_string(), "stage-keep".to_string()]),
                Some("stage-keep"),
            )
        })
        .with_tag_actions(vec![
            TagAction::Apply {
                tag: "stage-keep".to_string(),
                media: Some(vec![media]),
            },
            TagAction::Remove {
                tag: "shared".to_string(),
                media: vec![media],
            },
        ]),
    )
    .await
    .unwrap();

    let tags = pack.get_tags(media).await.unwrap();
    assert!(tags.contains(&"stage-keep".to_string()));
    assert!(!tags.contains(&"shared".to_string()));
    let stages = read_pack_behaviour(&pack)
        .await
        .experience
        .unwrap()
        .timeline
        .stages;
    assert_eq!(
        stages[1].content.tags.as_deref(),
        Some(&["shared".to_string(), "stage-keep".to_string()][..])
    );

    pack.undo().await.unwrap();
    assert_eq!(pack.get_tags(media).await.unwrap(), vec!["shared"]);
    let stages = read_pack_behaviour(&pack)
        .await
        .experience
        .unwrap()
        .timeline
        .stages;
    assert_eq!(
        stages[1].content.tags.as_deref(),
        Some(&["shared".to_string()][..])
    );
    assert_eq!(stages[1].content.owned_tag, None);
}

/// An edit the tables can't take leaves them exactly as they were -- the editor keeps showing
/// what is really stored rather than a half-applied edit.
#[tokio::test]
async fn a_rejected_edit_leaves_the_document_untouched() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("bad.lwpack"), data_dir.path(), "Bad").await;

    let id = std::sync::Arc::new(std::sync::Mutex::new(0i64));
    let created = id.clone();
    pack.behaviour_edit(BehaviourAction::new("Add caption", move |tx: &Transaction<'_>| {
        *created.lock().unwrap() =
            behaviour_editor::add_text_item(tx, PoolKind::Caption, &caption("kept"))?;
        Ok(true)
    }))
    .await
    .unwrap();
    let good = *id.lock().unwrap();

    let error = pack
        .behaviour_edit(BehaviourAction::new(
            "Edit caption",
            move |tx: &Transaction<'_>| {
                // Both edits are one author action, so the good one must not land on its own.
                behaviour_editor::set_text_item_text(tx, good, "changed")?;
                behaviour_editor::set_text_item_text(tx, good + 999, "no such entry")
            },
        ))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("no longer in the pack"),
        "unexpected error: {error}"
    );

    let behaviour = read_pack_behaviour(&pack).await;
    assert_eq!(behaviour.content.captions[0].text, "kept");
}

/// Two stages, one wallpaper each. Removing a stage is one author action, so it has to be one
/// undo entry -- the stage *and* the wallpaper that existed only for it going together, and
/// coming back together.
///
/// It used to be two: the front end cleared the slot through `clear_media_slot` and then wrote
/// the shortened timeline, so a single undo gave back a stage with its wallpaper missing --
/// a state the author never created. See `design/behaviour-storage.md`.
#[tokio::test]
async fn removing_a_stage_retires_its_wallpaper_in_one_undo_entry() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("stage.lwpack"), data_dir.path(), "Stage").await;
    let scenery = insert_staged_audio(&pack, b"stage wallpaper").await;

    let stage = |id: &str| Stage {
        id: id.to_string(),
        label: id.to_string(),
        end: None,
        content: ContentSelection::default(),
        events: Events::default(),
        movement: None,
        mitosis: None,
        on_enter: Default::default(),
        prompt: Default::default(),
    };
    let behaviour = Behaviour {
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![stage("stage-1"), stage("stage-2")],
                transitions: vec![],
            },
            label: None,
        }),
        ..Behaviour::default()
    };
    pack.replace_behaviour(behaviour.clone(), "Seed behaviour".to_string())
        .await
        .unwrap();
    pack.fill_media_slot(
        MediaSlot::StageWallpaper {
            stage: "stage-2".to_string(),
        },
        scenery,
        true,
    )
    .await
    .unwrap();

    // What the editor sends: remove the stage, plus the wallpaper it deliberately lets go.
    let edit = pack
        .behaviour_edit(
            BehaviourAction::new("Remove stage", |tx: &Transaction<'_>| {
                behaviour_editor::remove_stage(tx, "stage-2")
            })
            .retiring(vec![scenery]),
        )
        .await
        .unwrap();

    assert_eq!(edit.deleted_ids, vec![scenery]);
    assert_eq!(
        read_pack_behaviour(&pack)
            .await
            .experience
            .unwrap()
            .timeline
            .stages
            .len(),
        1
    );
    assert!(!pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == scenery));

    // One entry, not two: undoing once puts the stage *and* its wallpaper back.
    let status = pack.history_status().await.unwrap();
    assert_eq!(status.undo_label.as_deref(), Some("Remove stage"));
    pack.undo().await.unwrap();

    let behaviour = read_pack_behaviour(&pack).await;
    let stages = behaviour.experience.unwrap().timeline.stages;
    assert_eq!(stages.len(), 2);
    assert_eq!(
        stages[1].content.wallpaper,
        Some(scenery),
        "the stage came back with the wallpaper it had"
    );
    assert!(pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == scenery));
}

/// Retiring is a request, not an instruction: the file stays if the patched document still
/// points at it from somewhere else. A base wallpaper reused by a stage is Edgeware's ordinary
/// case, so removing that stage must not take the pack's own wallpaper with it.
#[tokio::test]
async fn retiring_media_another_slot_still_uses_keeps_it() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(
        &tmp.path().join("shared2.lwpack"),
        data_dir.path(),
        "Shared",
    )
    .await;
    let shared = insert_staged_audio(&pack, b"shared").await;

    pack.fill_media_slot(MediaSlot::Wallpaper, shared, true)
        .await
        .unwrap();

    let edit = pack
        .behaviour_edit(
            BehaviourAction::new("Remove stage", |_tx: &Transaction<'_>| Ok(false))
                .retiring(vec![shared]),
        )
        .await
        .unwrap();

    assert!(edit.deleted_ids.is_empty());
    assert_eq!(
        read_pack_behaviour(&pack).await.content.wallpaper,
        Some(shared)
    );
    assert!(pack
        .get_files()
        .await
        .unwrap()
        .iter()
        .any(|f| f.id == shared));
}

/// The invariant that makes retirement explicit rather than inferred: suspending the Experience
/// section drops every stage on purpose, and a cleanup driven by "which references went away?"
/// would delete every stage wallpaper the moment the author toggled the timeline off.
#[tokio::test]
async fn suspending_the_timeline_retires_nothing() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("suspend.lwpack"), data_dir.path(), "Sus").await;
    let scenery = insert_staged_audio(&pack, b"stage wallpaper").await;

    let behaviour = Behaviour {
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![Stage {
                    id: "stage-1".to_string(),
                    label: "Stage 1".to_string(),
                    end: None,
                    content: ContentSelection::default(),
                    events: Events::default(),
                    movement: None,
                    mitosis: None,
                    on_enter: Default::default(),
                    prompt: Default::default(),
                }],
                transitions: vec![],
            },
            label: None,
        }),
        ..Behaviour::default()
    };
    pack.replace_behaviour(behaviour.clone(), "Seed behaviour".to_string())
        .await
        .unwrap();
    pack.fill_media_slot(
        MediaSlot::StageWallpaper {
            stage: "stage-1".to_string(),
        },
        scenery,
        true,
    )
    .await
    .unwrap();

    // Toggling the timeline off: it stops playing, but nothing is retired.
    let edit = pack
        .behaviour_edit(BehaviourAction::new(
            "Disable timeline",
            |tx: &Transaction<'_>| behaviour_editor::set_experience_enabled(tx, false),
        ))
        .await
        .unwrap();

    assert!(read_pack_behaviour(&pack).await.experience.is_none());
    assert!(edit.deleted_ids.is_empty());
    assert!(
        pack.get_files()
            .await
            .unwrap()
            .iter()
            .any(|f| f.id == scenery),
        "the stage wallpaper has to survive so re-enabling the timeline restores a working one"
    );
}

/// The point of moving the timeline out of the blob: an undo entry is a changeset, so editing
/// one stage's label should record that stage and nothing else.
///
/// While the whole document was one `pack_data` row, every entry carried the entire document
/// twice (forward and inverse) whatever the author had touched -- so a minute of typing pushed
/// real work off the bottom of the hundred entries history keeps.
#[tokio::test]
async fn editing_one_stage_records_a_changeset_the_size_of_that_stage() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("size.lwpack"), data_dir.path(), "Size").await;

    let stage = |id: &str| Stage {
        id: id.to_string(),
        label: id.to_string(),
        end: None,
        content: ContentSelection::default(),
        events: Events::default(),
        movement: None,
        mitosis: None,
        on_enter: Default::default(),
        prompt: Default::default(),
    };
    // Enough captions that the blob is comfortably larger than one stage row, so a
    // whole-document write would be unmistakable in the numbers below.
    let behaviour = Behaviour {
        content: shared::behaviour::Content {
            captions: (0..200)
                .map(|index| TextItem {
                    text: format!("Caption number {index}, padded out to have some length."),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                })
                .collect(),
            ..Default::default()
        },
        experience: Some(Experience {
            timeline: Timeline {
                stages: (0..20)
                    .map(|index| stage(&format!("stage-{index}")))
                    .collect(),
                transitions: vec![],
            },
            label: None,
        }),
    };
    pack.replace_behaviour(behaviour, "Seed behaviour".to_string())
        .await
        .unwrap();

    pack.behaviour_edit(BehaviourAction::new(
        "Rename stage",
        |tx: &Transaction<'_>| {
            behaviour_editor::set_stage_label(tx, "stage-7", "Renamed")
        },
    ))
    .await
    .unwrap();

    let bytes: i64 = pack
        .db_execute(|conn| {
            conn.query_row(
                "SELECT LENGTH(forward_changeset) + LENGTH(inverse_changeset)
                 FROM history_entries ORDER BY sequence DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .await
        .unwrap();

    // Around 120 bytes in practice, against a document of roughly eleven kilobytes -- which
    // is what every entry used to cost, twice over, whatever the author had touched.
    assert!(
        bytes < 1024,
        "renaming one stage recorded {bytes} bytes of changeset, which is the whole document"
    );
    assert_eq!(
        read_pack_behaviour(&pack)
            .await
            .experience
            .unwrap()
            .timeline
            .stages[7]
            .label,
        "Renamed"
    );
}

/// A stage slot is addressed by stage id, not position, so it keeps pointing at the right
/// stage across the reordering the timeline editor does freely.
#[tokio::test]
async fn stage_wallpaper_slots_are_addressed_by_stage_id() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("stages.lwpack"), data_dir.path(), "Stages").await;
    let first = insert_staged_audio(&pack, b"first").await;
    let second = insert_staged_audio(&pack, b"second").await;

    let stage = |id: &str| Stage {
        id: id.to_string(),
        label: id.to_string(),
        end: None,
        content: ContentSelection::default(),
        events: Events::default(),
        movement: None,
        mitosis: None,
        on_enter: Default::default(),
        prompt: Default::default(),
    };
    let behaviour = Behaviour {
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![stage("stage-1"), stage("stage-2")],
                transitions: vec![],
            },
            label: None,
        }),
        ..Behaviour::default()
    };
    pack.replace_behaviour(behaviour.clone(), "Seed behaviour".to_string())
        .await
        .unwrap();

    let slot = |id: &str| MediaSlot::StageWallpaper {
        stage: id.to_string(),
    };
    pack.fill_media_slot(slot("stage-2"), first, true)
        .await
        .unwrap();
    let _ = pack
        .fill_media_slot(slot("stage-1"), second, true)
        .await
        .unwrap();
    let behaviour = pack.get_behaviour().await.unwrap();

    let stages = &behaviour.experience.as_ref().unwrap().timeline.stages;
    assert!(stages[0].content.wallpaper.is_some());
    assert_ne!(stages[0].content.wallpaper, stages[1].content.wallpaper);
    assert_eq!(
        behaviour.content.wallpaper, None,
        "the pack slot is untouched"
    );

    // Naming a stage that isn't there is an error, not a silent no-op: the editor can delete a
    // stage while a slot dialog is open.
    assert!(pack
        .fill_media_slot(slot("gone"), first, true)
        .await
        .is_err());
}

/// The other half of the changeset win, and the one the design doc promised loudest: typing in
/// a text pool is the commonest debounced edit there is, and it used to store the whole
/// document twice per pause.
#[tokio::test]
async fn editing_one_caption_records_a_changeset_the_size_of_that_caption() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("caption.lwpack"), data_dir.path(), "Cap").await;

    let behaviour = Behaviour {
        content: shared::behaviour::Content {
            captions: (0..200)
                .map(|index| TextItem {
                    text: format!("Caption number {index}, padded out to have some length."),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                })
                .collect(),
            ..Default::default()
        },
        ..Behaviour::new()
    };
    pack.replace_behaviour(behaviour, "Seed behaviour".to_string())
        .await
        .unwrap();

    pack.behaviour_edit(BehaviourAction::new(
        "Edit caption",
        |tx: &Transaction<'_>| {
            let target = behaviour_editor::text_pool(tx, PoolKind::Caption)?[99].id;
            behaviour_editor::set_text_item_text(tx, target, "Edited.")
        },
    ))
    .await
    .unwrap();

    let bytes: i64 = pack
        .db_execute(|conn| {
            conn.query_row(
                "SELECT LENGTH(forward_changeset) + LENGTH(inverse_changeset)
                 FROM history_entries ORDER BY sequence DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .await
        .unwrap();

    assert!(
        bytes < 1024,
        "editing one caption recorded {bytes} bytes of changeset, which is the whole document"
    );
    assert_eq!(
        read_pack_behaviour(&pack).await.content.captions[99].text,
        "Edited."
    );
}

#[tokio::test]
async fn set_pack_data_round_trips_a_named_blob() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
    pack.set_pack_data("custom", b"first".to_vec())
        .await
        .unwrap();
    // A repeat write for the same name replaces it, rather than erroring or duplicating.
    pack.set_pack_data("custom", b"second".to_vec())
        .await
        .unwrap();

    let blob: Vec<u8> = pack
        .db_execute(|conn| {
            conn.query_row(
                "SELECT blob FROM pack_data WHERE name = 'custom'",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
        })
        .await
        .unwrap();
    assert_eq!(blob, b"second");
    assert!(!pack.is_saved().await);
}

/// Exercises the exact path the pack editor's `get_behaviour`/`set_behaviour` Tauri commands
/// use (`Behaviour::to_json_bytes`/`from_json_bytes` over `set_pack_data`/`get_pack_data`),
/// with a document touching every new Content/Experience tab section -- content groups,
/// captions, web links, and an Experience section with a timeline level -- through a real
/// save + reopen, not just an in-memory round-trip.
#[tokio::test]
async fn content_and_experience_behaviour_survives_save_and_reopen() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let mut behaviour = Behaviour::new();
    behaviour.content.content_groups.push(ContentGroup {
        id: "kinky".to_string(),
        label: "Kinky".to_string(),
        description: Some("Kinky-tagged content".to_string()),
        tags: vec!["kinky".to_string()],
        enabled_by_default: true,
    });
    behaviour.content.captions.push(TextItem {
        text: "Obey.".to_string(),
        tags: vec!["kinky".to_string()],
        timeout_seconds: None,
        summary: None,
    });
    behaviour.content.web_links.push(WebLink {
        url: "https://duckduckgo.com/?q=".to_string(),
        args: vec!["edgeware packs".to_string()],
        tags: vec![],
    });
    behaviour.content.wallpaper = Some(1);
    behaviour.experience = Some(Experience {
        timeline: Timeline {
            stages: vec![
                Stage {
                    id: "stage-1".to_string(),
                    label: "Stage 1".to_string(),
                    end: Some(StageEnd {
                        duration_seconds: Some(300.0),
                        event_count: None,
                        strategy: EndStrategy::Any,
                    }),
                    content: ContentSelection::default(),
                    events: Events {
                        popup: Some(EventSchedule {
                            interval: Interval::Fixed { seconds: 30.0 },
                            initial_delay_seconds: None,
                            max_concurrent: None,
                        }),
                        ..Events::default()
                    },
                    movement: None,
                    mitosis: None,
                    on_enter: Default::default(),
                    prompt: Default::default(),
                },
                Stage {
                    id: "stage-2".to_string(),
                    label: "Stage 2".to_string(),
                    end: None,
                    content: ContentSelection {
                        tags: Some(vec!["kinky".to_string()]),
                        owned_tag: None,
                        wallpaper: None,
                        audio: None,
                        audio_random: false,
                    },
                    events: Events {
                        popup: Some(EventSchedule {
                            interval: Interval::Fixed { seconds: 45.0 },
                            initial_delay_seconds: None,
                            max_concurrent: None,
                        }),
                        ..Events::default()
                    },
                    movement: None,
                    mitosis: None,
                    on_enter: Default::default(),
                    prompt: Default::default(),
                },
            ],
            transitions: vec![Transition {
                id: "transition-1".to_string(),
                from_stage: "stage-1".to_string(),
                to_stage: "stage-2".to_string(),
                duration_seconds: 0.0,
                easing: Default::default(),
                affected: vec![],
            }],
        },
        label: None,
    });

    let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
    assert_eq!(pack.get_behaviour().await.unwrap(), Behaviour::new());
    // The wallpaper slot is a foreign key now, so the file it names has to be real.
    behaviour.content.wallpaper = Some(insert_staged_audio(&pack, b"wallpaper").await);

    pack.replace_behaviour(behaviour.clone(), "Seed behaviour".to_string())
        .await
        .unwrap();
    assert!(!pack.is_saved().await);
    pack.save(|_, _| {}).await.unwrap();
    drop(pack);

    // The whole round trip: the editor's tables out through `export_runtime` into the
    // `.lwpack`, and back in through `import_runtime` on reopen. A behaviour table missing
    // from either copy list shows up here as a timeline that did not survive the save.
    let pack2 = MediaPack::open(pack_path, data_dir.path()).await.unwrap();
    assert_eq!(pack2.get_behaviour().await.unwrap(), behaviour);
}

#[tokio::test]
async fn get_pack_data_returns_none_when_absent_and_the_written_blob_otherwise() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
    assert_eq!(pack.get_pack_data("custom").await.unwrap(), None);

    pack.set_pack_data("custom", b"the-blob".to_vec())
        .await
        .unwrap();
    assert_eq!(
        pack.get_pack_data("custom").await.unwrap(),
        Some(b"the-blob".to_vec())
    );
}
