use super::{caption, pack, timeline_pack, tx_on};
use crate::behaviour::db::read_one_stage;
use crate::behaviour::editor::{
    add_caption, add_caption_tag, add_stage_tag, captions, remove_caption, remove_caption_tag,
    remove_stage_tag, set_stage_content_tags, set_transition_affected, set_transition_category,
    transitions,
};
use crate::behaviour::schema::{Caption, TransitionCategory};

// ── One item of a collection ─────────────────────────────────────────────────

/// The property item-level commands buy: two removals in quick succession, each built against the
/// list as it was before either of them, must not put the other's tag back.
#[test]
fn two_tag_removals_do_not_restore_each_other() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_caption(
            tx,
            &Caption {
                tags: vec!["a".to_string(), "b".to_string(), "c".to_string()],
                ..caption("tagged")
            },
        )
        .unwrap();
        remove_caption_tag(tx, id, "a").unwrap();
        remove_caption_tag(tx, id, "b").unwrap();
    });
    assert_eq!(captions(&conn).unwrap()[0].tags, ["c"]);
}

/// Both are ordinary writes that happen to write nothing, not errors and not duplicates.
#[test]
fn adding_a_tag_twice_leaves_one_and_removing_an_absent_one_is_harmless() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_caption(tx, &caption("tagged")).unwrap();
        add_caption_tag(tx, id, "imp").unwrap();
        add_caption_tag(tx, id, "imp").unwrap();
        remove_caption_tag(tx, id, "absent").unwrap();
    });
    assert_eq!(captions(&conn).unwrap()[0].tags, ["imp"]);
}

#[test]
fn readding_a_removed_tag_puts_it_last() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_caption(
            tx,
            &Caption {
                tags: vec!["first".to_string(), "second".to_string()],
                ..caption("tagged")
            },
        )
        .unwrap();
        remove_caption_tag(tx, id, "first").unwrap();
        add_caption_tag(tx, id, "first").unwrap();
    });
    assert_eq!(captions(&conn).unwrap()[0].tags, ["second", "first"]);
}

/// A stage that shows everything has no selection for a tag to join; switching the restriction on
/// is a different action, because it has to seed the tag onto what the stage was showing.
#[test]
fn a_tag_cannot_join_an_unrestricted_stage() {
    let mut conn = timeline_pack(&["a", "b"]);
    let tx = conn.transaction().unwrap();
    assert!(add_stage_tag(&tx, "a", "imp").is_err());
}

/// Ownership lives on the association row, so a stage that stops selecting by its tag stops owning
/// it without a second step.
#[test]
fn removing_a_stages_owned_tag_removes_its_ownership() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(tx, "a", Some(&["stage-a".to_string()]), Some("stage-a")).unwrap();
        remove_stage_tag(tx, "a", "stage-a").unwrap();
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.content.tags.as_deref(), Some(&[][..]));
    assert_eq!(stage.content.owned_tag, None);
}

/// Early builds stored one entry for a whole group. Ticking any member has to replace the broad
/// entry with its siblings, or switching one off would silently switch the rest off too.
#[test]
fn toggling_a_category_expands_the_legacy_group_it_belonged_to() {
    let mut conn = timeline_pack(&["a", "b"]);
    let id = transitions(&conn).unwrap()[0].id.clone();
    tx_on(&mut conn, |tx| {
        set_transition_affected(tx, &id, &[TransitionCategory::Events]).unwrap();
        set_transition_category(tx, &id, TransitionCategory::PopupInterval, false).unwrap();
    });
    let affected = transitions(&conn).unwrap()[0].affected.clone();
    assert!(
        !affected.contains(&TransitionCategory::Events),
        "expanded away"
    );
    assert!(
        !affected.contains(&TransitionCategory::PopupInterval),
        "the one switched off"
    );
    assert!(
        affected.contains(&TransitionCategory::WebInterval),
        "its siblings survive: {affected:?}"
    );
}

#[test]
fn two_category_toggles_do_not_undo_each_other() {
    let mut conn = timeline_pack(&["a", "b"]);
    let id = transitions(&conn).unwrap()[0].id.clone();
    tx_on(&mut conn, |tx| {
        set_transition_category(tx, &id, TransitionCategory::PopupInterval, true).unwrap();
        set_transition_category(tx, &id, TransitionCategory::Crossfade, true).unwrap();
    });
    let affected = transitions(&conn).unwrap()[0].affected.clone();
    assert!(affected.contains(&TransitionCategory::PopupInterval));
    assert!(affected.contains(&TransitionCategory::Crossfade));
}

/// The same rule for content entries, which are addressed by id for the same reason.
#[test]
fn a_removed_entrys_id_is_never_given_to_the_next_one() {
    let mut conn = pack();
    let mut gone = 0;
    tx_on(&mut conn, |tx| {
        add_caption(tx, &caption("first")).unwrap();
        gone = add_caption(tx, &caption("last")).unwrap();
        remove_caption(tx, gone).unwrap();
    });
    let mut added = 0;
    tx_on(&mut conn, |tx| {
        added = add_caption(tx, &caption("new")).unwrap();
    });
    assert_ne!(added, gone, "a stale Remove would have taken the new entry");
}
