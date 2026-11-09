//! `crate::session::Session`: that what it reports as changed is what changed, and that its media
//! index keeps up.

use std::sync::Arc;

use rusqlite::Transaction;
use shared::behaviour::editor as behaviour_editor;

use super::*;
use crate::changes::{Scope, Scopes};
use crate::session::Session;

async fn session_with_files(count: usize) -> (tempfile::TempDir, Session, Vec<u64>) {
    let dir = tempfile::tempdir().unwrap();
    let pack = new_test_pack(&dir.path().join("test.lwpack"), dir.path(), "Test").await;
    let mut ids = Vec::new();
    for n in 0..count {
        ids.push(insert_staged_audio(&pack, format!("file {n}").as_bytes()).await);
    }
    let session = tokio::task::spawn_blocking(move || Session::open(Arc::new(pack)))
        .await
        .unwrap()
        .unwrap();
    (dir, session, ids)
}

fn tags_of(session: &Session, id: u64) -> Vec<String> {
    let mut tags = session.media().get(id).unwrap().tags.clone();
    tags.sort();
    tags
}

fn enable_timeline(session: &mut Session) -> String {
    session
        .edit(BehaviourAction::new("Enable", |tx: &Transaction<'_>| {
            behaviour_editor::set_timeline_enabled(tx, true)?;
            behaviour_editor::add_stage(tx, None, None, "First").map(drop)
        }))
        .unwrap();
    session.read(behaviour_editor::stages).unwrap().remove(0).id
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_loads_every_file() {
    let (_dir, session, ids) = session_with_files(3).await;
    assert_eq!(session.media().len(), 3);
    assert_eq!(
        session.media().iter().map(|f| f.id).collect::<Vec<_>>(),
        ids
    );
    assert!(!session.history().can_undo);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_behaviour_edit_reports_its_scope_and_history() {
    let (_dir, mut session, _) = session_with_files(1).await;
    let stage = enable_timeline(&mut session);

    let applied = session
        .edit(BehaviourAction::new(
            "Rename stage",
            move |tx: &Transaction<'_>| behaviour_editor::set_stage_label(tx, &stage, "Renamed"),
        ))
        .unwrap();

    assert!(applied.changes.scopes.contains(Scope::Timeline));
    assert!(!applied.changes.scopes.contains(Scope::Media));
    assert!(applied.changes.media.is_empty());
    assert!(applied.history.can_undo);
    assert_eq!(applied.history.undo_label.as_deref(), Some("Rename stage"));
    assert!(!applied.history.at_saved_state);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_that_changes_nothing_reports_nothing() {
    let (_dir, mut session, _) = session_with_files(1).await;
    enable_timeline(&mut session);
    let before = session.history().clone();

    let applied = session
        .edit(BehaviourAction::new(
            "Enable again",
            |tx: &Transaction<'_>| behaviour_editor::set_timeline_enabled(tx, true),
        ))
        .unwrap();

    assert!(applied.changes.is_empty());
    assert_eq!(applied.history.undo_label, before.undo_label);
}

#[tokio::test(flavor = "multi_thread")]
async fn tagging_files_updates_them_in_the_index() {
    let (_dir, mut session, ids) = session_with_files(3).await;

    let applied = session.add_tag_to_files(&ids[..2], "red").unwrap();

    assert!(applied.changes.scopes.contains(Scope::Tags));
    assert!(applied.changes.scopes.contains(Scope::Media));
    assert_eq!(applied.changes.media.updated, ids[..2]);
    assert!(applied.changes.media.added.is_empty() && applied.changes.media.removed.is_empty());
    assert_eq!(tags_of(&session, ids[0]), ["red"]);
    assert_eq!(tags_of(&session, ids[2]), Vec::<String>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_tag_reaches_every_file_carrying_it() {
    let (_dir, mut session, ids) = session_with_files(3).await;
    session.add_tag_to_files(&ids[..2], "red").unwrap();

    // Only the `tags` row changes; no file's row does. The session still has to update both files.
    let applied = session.rename_tag("red", "crimson").unwrap();

    assert_eq!(applied.changes.media.updated, ids[..2]);
    assert_eq!(tags_of(&session, ids[0]), ["crimson"]);
    assert_eq!(tags_of(&session, ids[1]), ["crimson"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_tag_reaches_files_through_the_cascade() {
    let (_dir, mut session, ids) = session_with_files(2).await;
    session.add_tag_to_files(&ids, "red").unwrap();

    let applied = session.delete_tag("red").unwrap();

    assert_eq!(applied.changes.media.updated, ids);
    assert!(tags_of(&session, ids[0]).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn merging_tags_moves_files_to_the_target() {
    let (_dir, mut session, ids) = session_with_files(2).await;
    session.add_tag_to_files(&ids[..1], "red").unwrap();
    session.add_tag_to_files(&ids[1..], "crimson").unwrap();

    session.merge_tag("red", "crimson").unwrap();

    assert_eq!(tags_of(&session, ids[0]), ["crimson"]);
    assert_eq!(tags_of(&session, ids[1]), ["crimson"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_and_redo_report_what_they_reverse() {
    let (_dir, mut session, ids) = session_with_files(2).await;
    session.add_tag_to_files(&ids, "red").unwrap();

    let undone = session.undo().unwrap();
    assert_eq!(undone.changes.media.updated, ids);
    assert!(undone.changes.scopes.contains(Scope::Tags));
    assert!(tags_of(&session, ids[0]).is_empty());
    assert!(undone.history.can_redo);

    let redone = session.redo().unwrap();
    assert_eq!(redone.changes.media.updated, ids);
    assert_eq!(tags_of(&session, ids[0]), ["red"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_edit_changes_nothing() {
    let (_dir, mut session, ids) = session_with_files(1).await;
    session.add_tag_to_files(&ids, "red").unwrap();
    session.add_tag_to_files(&ids, "blue").unwrap();
    let before = session.history().clone();

    let error = session.rename_tag("red", "blue").unwrap_err();

    assert!(error.to_string().contains("already exists"), "{error}");
    assert_eq!(tags_of(&session, ids[0]), ["blue", "red"]);
    assert_eq!(session.history().undo_label, before.undo_label);
}

#[tokio::test(flavor = "multi_thread")]
async fn hiding_and_restoring_a_file_reports_removed_and_added() {
    let (_dir, mut session, ids) = session_with_files(2).await;
    let id = ids[0];
    let hide = |hidden: bool| {
        move |c: &rusqlite::Connection| {
            history::record(c, "Hide", 0, &[id], |tx| {
                tx.execute(
                    "UPDATE media SET deleted = ? WHERE id = ?",
                    params![hidden, id],
                )?;
                Ok(())
            })
        }
    };

    let hidden = session.write_for_tests(hide(true)).unwrap();
    assert_eq!(hidden.changes.media.removed, [id]);
    assert!(session.media().get(id).is_none());

    let undone = session.undo().unwrap();
    assert_eq!(undone.changes.media.added, [id]);
    assert!(session.media().get(id).is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn edits_wait_for_no_save() {
    let (_dir, mut session, ids) = session_with_files(1).await;
    let pack = session.pack().clone();
    let _saving = pack.saving.clone().try_write_owned().unwrap();

    let error = session.add_tag_to_files(&ids, "red").unwrap_err();
    assert!(error.to_string().contains("being saved"), "{error}");
}

#[test]
fn every_scope_is_in_all() {
    let all = Scopes::all();
    for scope in [
        Scope::Metadata,
        Scope::Modes,
        Scope::PackData,
        Scope::Slots,
        Scope::Entries,
        Scope::ContentGroups,
        Scope::Timeline,
        Scope::Attributes,
        Scope::Tags,
        Scope::Artists,
        Scope::Media,
    ] {
        assert!(all.contains(scope));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn undoing_back_to_the_save_marks_the_pack_saved() {
    let (_dir, mut session, ids) = session_with_files(1).await;
    let pack = session.pack().clone();
    pack.save(|_, _| {}).await.unwrap();
    assert!(pack.is_saved().await);

    let edited = session.add_tag_to_files(&ids, "red").unwrap();
    assert!(!edited.history.at_saved_state);
    assert!(!pack.is_saved().await);
    assert!(pack.dir().join("UNSAVED").exists());

    let undone = session.undo().unwrap();
    assert!(undone.history.at_saved_state);
    assert!(pack.is_saved().await);
    assert!(!pack.dir().join("UNSAVED").exists());
}
