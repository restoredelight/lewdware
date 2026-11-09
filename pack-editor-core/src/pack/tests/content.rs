//! The content track's edits through `Session`: entries, content groups and the pack's media
//! slots. Each is one undo entry, reports the scope it changed, and keeps the media index in step
//! when a slot takes a file with it.

use std::sync::Arc;

use shared::behaviour::editor as behaviour_editor;
use shared::behaviour::MediaSlot;

use super::*;
use crate::changes::Scope;
use crate::history;
use crate::pack::ops::content::{self as content_ops, EntryKind};
use crate::session::Session;
use shared::tags;

async fn session_with_files(count: usize) -> (tempfile::TempDir, Session, Vec<u64>) {
    let dir = tempfile::tempdir().unwrap();
    let pack = new_test_pack(&dir.path().join("test.lwpack"), dir.path(), "Test").await;
    let mut ids = Vec::new();
    for n in 0..count {
        ids.push(insert_staged_audio(&pack, format!("content file {n}").as_bytes()).await);
    }
    let session = tokio::task::spawn_blocking(move || Session::open(Arc::new(pack)))
        .await
        .unwrap()
        .unwrap();
    (dir, session, ids)
}

fn entries(session: &Session, kind: EntryKind) -> Vec<content_ops::Entry> {
    session.read(|c| content_ops::entries(c, kind)).unwrap()
}

fn groups(session: &Session) -> Vec<shared::behaviour::ContentGroup> {
    session.read(behaviour_editor::content_groups).unwrap()
}

fn slots(session: &Session) -> behaviour_editor::MediaSlots {
    session.read(behaviour_editor::media_slots).unwrap()
}

/// Marks a file as a slot's scenery, the way importing it into a slot does.
fn make_scenery(session: &mut Session, id: u64) {
    session
        .write_for_tests(move |c| {
            history::record(c, "Mark scenery", 0, &[id], |tx| {
                crate::pack::labels::apply_tag(tx, id, tags::EXPLICIT_ONLY_TAG)
            })
        })
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_kind_of_entry_adds_edits_and_removes_as_one_undo_entry_each() {
    let (_dir, mut session, _) = session_with_files(0).await;
    for kind in [
        EntryKind::Caption,
        EntryKind::Prompt,
        EntryKind::Notification,
        EntryKind::WebLink,
    ] {
        let added = session.add_entry(kind).unwrap();
        assert!(added.changes.scopes.contains(Scope::Entries));
        assert_eq!(
            added.history.undo_label.as_deref(),
            Some(format!("Add {}", kind.noun()).as_str())
        );
        let id = added.value;

        let edited = session.set_entry_text(kind, id, "Hello").unwrap();
        assert!(edited.changes.scopes.contains(Scope::Entries));
        assert_eq!(
            edited.history.undo_label.as_deref(),
            Some(format!("Edit {}", kind.noun()).as_str())
        );
        assert_eq!(entries(&session, kind)[0].text, "Hello");

        // One step back is the edit, not the add.
        session.undo().unwrap();
        assert_eq!(entries(&session, kind)[0].text, "");
        session.redo().unwrap();

        session.remove_entry(kind, id).unwrap();
        assert!(entries(&session, kind).is_empty());
        session.undo().unwrap();
        assert_eq!(entries(&session, kind)[0].text, "Hello");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn writing_the_same_text_again_records_nothing() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let id = session.add_entry(EntryKind::Caption).unwrap().value;
    session
        .set_entry_text(EntryKind::Caption, id, "Same")
        .unwrap();
    let again = session
        .set_entry_text(EntryKind::Caption, id, "Same")
        .unwrap();
    assert!(again.changes.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blank_notification_title_is_no_title() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let id = session.add_entry(EntryKind::Notification).unwrap().value;

    session.set_notification_summary(id, Some("Hey")).unwrap();
    assert_eq!(
        entries(&session, EntryKind::Notification)[0]
            .summary
            .as_deref(),
        Some("Hey")
    );
    session.set_notification_summary(id, Some("   ")).unwrap();
    assert_eq!(entries(&session, EntryKind::Notification)[0].summary, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn prompt_time_limits_are_positive_or_automatic() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let id = session.add_entry(EntryKind::Prompt).unwrap().value;

    session.set_prompt_timeout(id, Some(20.)).unwrap();
    assert_eq!(
        entries(&session, EntryKind::Prompt)[0].timeout_seconds,
        Some(20.)
    );
    assert!(session.set_prompt_timeout(id, Some(0.)).is_err());
    assert!(session.set_prompt_timeout(id, Some(f64::NAN)).is_err());
    session.set_prompt_timeout(id, None).unwrap();
    assert_eq!(
        entries(&session, EntryKind::Prompt)[0].timeout_seconds,
        None
    );

    assert_eq!(content_ops::automatic_prompt_timeout(""), 15);
    assert_eq!(content_ops::automatic_prompt_timeout(&"x".repeat(50)), 30);
    assert_eq!(content_ops::automatic_prompt_timeout(&"x".repeat(51)), 35);
}

#[tokio::test(flavor = "multi_thread")]
async fn entry_tags_create_author_tags_and_refuse_reserved_ones() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let id = session.add_entry(EntryKind::Caption).unwrap().value;

    let added = session
        .add_entry_tag(EntryKind::Caption, id, " feet ")
        .unwrap();
    assert!(added.changes.scopes.contains(Scope::Entries));
    // A tag the document names is a real tag row, so the Tags view hears about it.
    assert!(added.changes.scopes.contains(Scope::Tags));
    assert_eq!(entries(&session, EntryKind::Caption)[0].tags, ["feet"]);
    assert_eq!(
        session.read(content_ops::author_tags).unwrap(),
        vec!["feet".to_string()]
    );

    assert!(session
        .add_entry_tag(EntryKind::Caption, id, tags::NON_POPUP_TAG)
        .is_err());

    session
        .remove_entry_tag(EntryKind::Caption, id, "feet")
        .unwrap();
    assert!(entries(&session, EntryKind::Caption)[0].tags.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_an_entry_that_is_gone_fails_and_changes_nothing() {
    let (_dir, mut session, _) = session_with_files(0).await;
    let id = session.add_entry(EntryKind::Caption).unwrap().value;
    session.remove_entry(EntryKind::Caption, id).unwrap();
    let before = session.history().clone();

    assert!(session.set_entry_text(EntryKind::Caption, id, "x").is_err());
    assert!(session.add_entry_tag(EntryKind::Caption, id, "x").is_err());
    assert_eq!(session.history().undo_label, before.undo_label);
    // Not even the tag the failed edit would have created.
    assert!(session.read(content_ops::author_tags).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn content_groups_are_made_named_described_and_toggled() {
    let (_dir, mut session, _) = session_with_files(0).await;

    let first = session.add_content_group().unwrap();
    assert!(first.changes.scopes.contains(Scope::ContentGroups));
    let second = session.add_content_group().unwrap().value;
    assert_ne!(first.value, second);
    let key = first.value;

    assert!(!content_ops::group_has_content(&groups(&session)[0]));
    session.set_content_group_label(&key, " Feet ").unwrap();
    assert_eq!(groups(&session)[0].label, "Feet");
    assert!(session.set_content_group_label(&key, "  ").is_err());

    session
        .set_content_group_description(&key, Some("All of it"))
        .unwrap();
    assert_eq!(
        groups(&session)[0].description.as_deref(),
        Some("All of it")
    );
    session
        .set_content_group_description(&key, Some(""))
        .unwrap();
    assert_eq!(groups(&session)[0].description, None);

    session
        .set_content_group_enabled_by_default(&key, false)
        .unwrap();
    assert!(!groups(&session)[0].enabled_by_default);
    assert_eq!(
        session.history().undo_label.as_deref(),
        Some("Change group default")
    );

    session.add_content_group_tag(&key, "feet").unwrap();
    assert_eq!(groups(&session)[0].tags, ["feet"]);
    assert!(content_ops::group_has_content(&groups(&session)[0]));
    session.remove_content_group_tag(&key, "feet").unwrap();
    assert!(groups(&session)[0].tags.is_empty());

    session.remove_content_group(&key).unwrap();
    assert_eq!(groups(&session).len(), 1);
    session.undo().unwrap();
    assert_eq!(groups(&session).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tag_becomes_a_group_named_after_it() {
    let (_dir, mut session, _) = session_with_files(0).await;

    let key = session.add_content_group_from_tag("feet").unwrap().value;
    let group = &groups(&session)[0];
    assert_eq!(key, "feet");
    assert_eq!(group.label, "Feet");
    assert_eq!(group.tags, ["feet"]);
    assert!(group.enabled_by_default);

    // A second group from the same tag doesn't collide with the first's key.
    let again = session.add_content_group_from_tag("feet").unwrap().value;
    assert_eq!(again, "feet-2");
    assert!(session.add_content_group_from_tag(" ").is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn choosing_an_existing_file_for_a_slot_leaves_its_role_alone() {
    let (_dir, mut session, ids) = session_with_files(2).await;

    let set = session
        .set_media_slot(&MediaSlot::Wallpaper, ids[0])
        .unwrap();
    assert!(set.changes.scopes.contains(Scope::Slots));
    assert_eq!(set.value, None);
    assert_eq!(set.history.undo_label.as_deref(), Some("Set wallpaper"));
    assert_eq!(slots(&session).wallpaper, Some(ids[0]));
    // Not marked as scenery: it's still the popup it was.
    assert!(session.media().get(ids[0]).unwrap().tags.is_empty());

    // Replacing a file that isn't scenery keeps it in the pack.
    let replaced = session
        .set_media_slot(&MediaSlot::Wallpaper, ids[1])
        .unwrap();
    assert_eq!(replaced.value, None);
    assert!(session.media().get(ids[0]).is_some());

    let cleared = session.clear_media_slot(&MediaSlot::Wallpaper).unwrap();
    assert_eq!(cleared.value, None);
    assert_eq!(
        cleared.history.undo_label.as_deref(),
        Some("Clear wallpaper")
    );
    assert_eq!(slots(&session).wallpaper, None);
    session.undo().unwrap();
    assert_eq!(slots(&session).wallpaper, Some(ids[1]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slots_own_scenery_leaves_with_it_and_comes_back_on_undo() {
    let (_dir, mut session, ids) = session_with_files(2).await;
    session.set_media_slot(&MediaSlot::Splash, ids[0]).unwrap();
    make_scenery(&mut session, ids[0]);

    // Replacing it drops the scenery file, and the index hears about it.
    let replaced = session.set_media_slot(&MediaSlot::Splash, ids[1]).unwrap();
    assert_eq!(replaced.value, Some(ids[0]));
    assert_eq!(replaced.changes.media.removed, [ids[0]]);
    assert!(session.media().get(ids[0]).is_none());

    let undone = session.undo().unwrap();
    assert_eq!(undone.changes.media.added, [ids[0]]);
    assert_eq!(slots(&session).splash, Some(ids[0]));

    // So does clearing it, unless another slot still points at it.
    session
        .set_media_slot(&MediaSlot::Wallpaper, ids[0])
        .unwrap();
    let cleared = session.clear_media_slot(&MediaSlot::Splash).unwrap();
    assert_eq!(cleared.value, None);
    let cleared = session.clear_media_slot(&MediaSlot::Wallpaper).unwrap();
    assert_eq!(cleared.value, Some(ids[0]));
    assert!(session.media().get(ids[0]).is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slot_filled_by_an_import_reaches_the_session() {
    let (_dir, mut session, ids) = session_with_files(1).await;
    let pack = session.pack().clone();
    // What `encode::process_file_into_slot` does, on its own connection.
    pack.fill_media_slot(MediaSlot::Wallpaper, ids[0], true)
        .await
        .unwrap();

    let applied = session.slot_filled_by_import(&[ids[0]]).unwrap();
    assert!(applied.changes.scopes.contains(Scope::Slots));
    assert_eq!(applied.changes.media.updated, [ids[0]]);
    assert!(applied.history.can_undo);
    assert!(session
        .media()
        .get(ids[0])
        .unwrap()
        .tags
        .iter()
        .any(|tag| tag == tags::EXPLICIT_ONLY_TAG));
}
