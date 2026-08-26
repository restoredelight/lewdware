use super::{arg_values, item, pack, timeline_pack, tx_on};
use crate::behaviour::editor::{
    PoolKind, add_stage_tag, add_text_item, add_text_item_tag, add_web_link, add_web_link_arg,
    remove_stage_tag, remove_text_item, remove_text_item_tag, remove_web_link_arg,
    set_stage_content_tags, set_transition_affected, set_transition_category, text_pool,
    transitions, web_links,
};
use crate::behaviour::schema::{TextItem, TransitionCategory, WebLink};
use crate::behaviour::storage::read_one_stage;

// ── One item of a collection ─────────────────────────────────────────────────

/// The property item-level commands buy: two removals in quick succession, each built against the
/// list as it was before either of them, must not put the other's tag back.
#[test]
fn two_tag_removals_do_not_restore_each_other() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_text_item(
            tx,
            PoolKind::Caption,
            &TextItem {
                tags: vec!["a".to_string(), "b".to_string(), "c".to_string()],
                ..item("tagged")
            },
        )
        .unwrap();
        assert!(remove_text_item_tag(tx, id, "a").unwrap());
        assert!(remove_text_item_tag(tx, id, "b").unwrap());
    });
    assert_eq!(
        text_pool(&conn, PoolKind::Caption).unwrap()[0].item.tags,
        ["c"]
    );
}

#[test]
fn adding_a_tag_that_is_already_there_reports_no_change() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_text_item(tx, PoolKind::Caption, &item("tagged")).unwrap();
        assert!(add_text_item_tag(tx, id, "imp").unwrap());
        assert!(!add_text_item_tag(tx, id, "imp").unwrap());
        assert!(!remove_text_item_tag(tx, id, "absent").unwrap());
    });
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
        assert!(remove_stage_tag(tx, "a", "stage-a").unwrap());
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.content.tags.as_deref(), Some(&[][..]));
    assert_eq!(stage.content.owned_tag, None);
}

/// The same lost-update property for URL suffixes, which are ordered and may repeat — so they are
/// removed by position rather than by value.
#[test]
fn suffixes_are_added_and_removed_one_at_a_time() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_web_link(
            tx,
            &WebLink {
                url: "https://example.invalid".to_string(),
                args: vec![],
                tags: vec![],
            },
        )
        .unwrap();
        add_web_link_arg(tx, id, "a").unwrap();
        add_web_link_arg(tx, id, "b").unwrap();
        add_web_link_arg(tx, id, "a").unwrap();
        let args = web_links(tx).unwrap()[0].args.clone();
        assert!(remove_web_link_arg(tx, id, args[1].id).unwrap());
        add_web_link_arg(tx, id, "c").unwrap();
    });
    assert_eq!(arg_values(&web_links(&conn).unwrap()[0]), ["a", "a", "c"]);
}

/// Two removals in quick succession are each aimed at what the author saw, and the second must not
/// land on whatever has moved into that place. Addressed by position this fails either way: closing
/// the gap slides C into B's slot, and leaving it means the next suffix added takes the number back.
#[test]
fn two_suffix_removals_hit_the_two_that_were_clicked() {
    let mut conn = pack();
    let mut id = 0;
    let mut positions = vec![];
    tx_on(&mut conn, |tx| {
        id = add_web_link(
            tx,
            &WebLink {
                url: "https://example.invalid".to_string(),
                args: vec!["a".to_string(), "b".to_string(), "c".to_string()],
                tags: vec![],
            },
        )
        .unwrap();
        positions = web_links(tx).unwrap()[0]
            .args
            .iter()
            .map(|arg| arg.id)
            .collect();
    });

    tx_on(&mut conn, |tx| {
        // Both aimed at the list as the author saw it, neither having seen the other land.
        assert!(remove_web_link_arg(tx, id, positions[0]).unwrap());
        assert!(remove_web_link_arg(tx, id, positions[1]).unwrap());
    });

    assert_eq!(arg_values(&web_links(&conn).unwrap()[0]), ["c"]);
}

/// Early builds stored one entry for a whole group. Ticking any member has to replace the broad
/// entry with its siblings, or switching one off would silently switch the rest off too.
#[test]
fn toggling_a_category_expands_the_legacy_group_it_belonged_to() {
    let mut conn = timeline_pack(&["a", "b"]);
    let id = transitions(&conn).unwrap()[0].id.clone();
    tx_on(&mut conn, |tx| {
        set_transition_affected(tx, &id, &[TransitionCategory::Events]).unwrap();
        assert!(
            set_transition_category(tx, &id, TransitionCategory::PopupInterval, false).unwrap()
        );
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

/// Removing the *last* suffix and adding another is where a position-as-identity scheme fails
/// silently: the new suffix takes the number the removed one had, so a stale Remove still on screen
/// deletes the wrong thing. An id outlives the row it named.
#[test]
fn a_removed_suffixs_id_is_never_given_to_the_next_one() {
    let mut conn = pack();
    let mut id = 0;
    let mut gone = 0;
    tx_on(&mut conn, |tx| {
        id = add_web_link(
            tx,
            &WebLink {
                url: "https://example.invalid".to_string(),
                args: vec!["a".to_string(), "b".to_string()],
                tags: vec![],
            },
        )
        .unwrap();
        let args = web_links(tx).unwrap()[0].args.clone();
        gone = args[1].id;
        assert!(remove_web_link_arg(tx, id, gone).unwrap());
        add_web_link_arg(tx, id, "c").unwrap();
    });

    let args = web_links(&conn).unwrap()[0].args.clone();
    assert_eq!(
        args.iter().map(|arg| &arg.value).collect::<Vec<_>>(),
        ["a", "c"]
    );
    assert!(
        args.iter().all(|arg| arg.id != gone),
        "the new suffix must not inherit the removed one's id: {args:?}"
    );
    // And a stale removal aimed at the one that went finds nothing rather than taking its place.
    let tx = conn.transaction().unwrap();
    assert!(!remove_web_link_arg(&tx, id, gone).unwrap());
}

/// The same rule for the pools, which are addressed by id from the editor for the same reason.
#[test]
fn a_removed_entrys_id_is_never_given_to_the_next_one() {
    let mut conn = pack();
    let mut gone = 0;
    tx_on(&mut conn, |tx| {
        add_text_item(tx, PoolKind::Caption, &item("first")).unwrap();
        gone = add_text_item(tx, PoolKind::Caption, &item("last")).unwrap();
        assert!(remove_text_item(tx, gone).unwrap());
    });
    let mut added = 0;
    tx_on(&mut conn, |tx| {
        added = add_text_item(tx, PoolKind::Caption, &item("new")).unwrap();
    });
    assert_ne!(added, gone, "a stale Remove would have taken the new entry");
}
