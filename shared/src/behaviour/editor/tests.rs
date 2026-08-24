use super::*;
use crate::behaviour::storage;
use crate::behaviour::{
    Behaviour, Content, Easing, EventSchedule, Events, Experience, Interval, Timeline,
    TransitionCategory,
};
use rusqlite::OptionalExtension as _;

/// A migrated pack database. The editor module never needs media rows of its own — the entities
/// here are text, links and groups — but the schema's foreign keys want a real database.
fn pack() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    crate::db::migrate(&mut conn).unwrap();
    conn.execute(
        "INSERT INTO media (id, file_name, file_type, \"offset\", length, hash)
         VALUES (1, 'a.png', 'image', 0, 0, x'00'), (2, 'b.png', 'image', 0, 0, x'01'),
                (3, 'c.ogg', 'audio', 0, 0, x'02')",
        [],
    )
    .unwrap();
    conn
}

fn item(text: &str) -> TextItem {
    TextItem {
        text: text.to_string(),
        tags: vec![],
        timeout_seconds: None,
        summary: None,
    }
}

fn tx_on(conn: &mut Connection, body: impl FnOnce(&Transaction<'_>)) {
    let tx = conn.transaction().unwrap();
    body(&tx);
    tx.commit().unwrap();
}

fn texts(conn: &Connection, kind: PoolKind) -> Vec<String> {
    text_pool(conn, kind)
        .unwrap()
        .into_iter()
        .map(|row| row.item.text)
        .collect()
}

#[test]
fn a_pool_entry_round_trips_with_its_id() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_text_item(tx, PoolKind::Caption, &item("first")).unwrap();
    });

    let rows = text_pool(&conn, PoolKind::Caption).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].item.text, "first");
}

/// The point of the module: an edit names the row it means, so it cannot land on a neighbour if
/// the pool has shifted under the editor since it last looked.
#[test]
fn editing_one_entry_leaves_its_neighbours_alone() {
    let mut conn = pack();
    let mut ids = vec![];
    tx_on(&mut conn, |tx| {
        for text in ["one", "two", "three"] {
            ids.push(add_text_item(tx, PoolKind::Caption, &item(text)).unwrap());
        }
    });

    tx_on(&mut conn, |tx| {
        assert!(update_text_item(tx, ids[1], &item("edited")).unwrap());
    });

    assert_eq!(texts(&conn, PoolKind::Caption), ["one", "edited", "three"]);
}

/// An edit that lands on the value already stored is an `oninput` that changed nothing, and must
/// not spend one of the hundred entries undo keeps.
#[test]
fn an_edit_that_changes_nothing_reports_no_change() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_text_item(tx, PoolKind::Caption, &item("same")).unwrap();
    });
    tx_on(&mut conn, |tx| {
        assert!(!update_text_item(tx, id, &item("same")).unwrap());
    });
}

/// A stale editor writing into an entry that has since been removed must be told, not quietly
/// given the entry back — reviving it would undo the removal.
#[test]
fn editing_a_removed_entry_is_refused() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_text_item(tx, PoolKind::Caption, &item("gone")).unwrap();
        assert!(remove_text_item(tx, id).unwrap());
    });
    let tx = conn.transaction().unwrap();
    assert!(update_text_item(&tx, id, &item("back")).is_err());
}

/// `(kind, position)` is unique, so a removal has to close the gap it leaves or the next append
/// collides with it.
#[test]
fn removing_an_entry_closes_the_gap() {
    let mut conn = pack();
    let mut ids = vec![];
    tx_on(&mut conn, |tx| {
        for text in ["one", "two", "three"] {
            ids.push(add_text_item(tx, PoolKind::Caption, &item(text)).unwrap());
        }
        assert!(remove_text_item(tx, ids[0]).unwrap());
        add_text_item(tx, PoolKind::Caption, &item("four")).unwrap();
    });
    assert_eq!(texts(&conn, PoolKind::Caption), ["two", "three", "four"]);
}

#[test]
fn the_three_pools_do_not_see_each_other() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        add_text_item(tx, PoolKind::Caption, &item("a caption")).unwrap();
        add_text_item(tx, PoolKind::Prompt, &item("a prompt")).unwrap();
        add_text_item(tx, PoolKind::Notification, &item("a notification")).unwrap();
    });
    assert_eq!(texts(&conn, PoolKind::Caption), ["a caption"]);
    assert_eq!(texts(&conn, PoolKind::Prompt), ["a prompt"]);
    assert_eq!(texts(&conn, PoolKind::Notification), ["a notification"]);
}

#[test]
fn reordering_a_pool_rewrites_its_order() {
    let mut conn = pack();
    let mut ids = vec![];
    tx_on(&mut conn, |tx| {
        for text in ["one", "two", "three"] {
            ids.push(add_text_item(tx, PoolKind::Caption, &item(text)).unwrap());
        }
        assert!(reorder_text_items(tx, PoolKind::Caption, &[ids[2], ids[0], ids[1]]).unwrap());
    });
    assert_eq!(texts(&conn, PoolKind::Caption), ["three", "one", "two"]);
}

/// A reordering that does not name exactly the entries the pool holds is a stale editor acting on
/// a pool that has changed; applying the part that matches would silently drop the rest.
#[test]
fn an_incomplete_reordering_is_refused() {
    let mut conn = pack();
    let mut ids = vec![];
    tx_on(&mut conn, |tx| {
        for text in ["one", "two"] {
            ids.push(add_text_item(tx, PoolKind::Caption, &item(text)).unwrap());
        }
    });
    let tx = conn.transaction().unwrap();
    assert!(reorder_text_items(&tx, PoolKind::Caption, &[ids[0]]).is_err());
}

#[test]
fn an_entrys_tags_round_trip_and_become_real_tags() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        add_text_item(
            tx,
            PoolKind::Caption,
            &TextItem {
                tags: vec!["imp".to_string(), "succubus".to_string()],
                ..item("tagged")
            },
        )
        .unwrap();
    });
    let rows = text_pool(&conn, PoolKind::Caption).unwrap();
    assert_eq!(rows[0].item.tags, ["imp", "succubus"]);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM tags", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn a_web_link_round_trips_with_its_args_and_tags() {
    let mut conn = pack();
    let link = WebLink {
        url: "https://example.invalid".to_string(),
        args: vec!["a".to_string(), "b".to_string(), "a".to_string()],
        tags: vec!["imp".to_string()],
    };
    tx_on(&mut conn, |tx| {
        add_web_link(tx, &link).unwrap();
    });
    let rows = web_links(&conn).unwrap();
    assert_eq!(rows.len(), 1);
    // The same suffix twice is a legitimate way to weight it, so args are a list, not a set.
    assert_eq!(rows[0].link, link);
}

#[test]
fn shortening_a_links_args_drops_the_tail() {
    let mut conn = pack();
    let mut id = 0;
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
        assert!(update_web_link(
            tx,
            id,
            &WebLink {
                url: "https://example.invalid".to_string(),
                args: vec!["a".to_string()],
                tags: vec![],
            }
        )
        .unwrap());
    });
    assert_eq!(web_links(&conn).unwrap()[0].link.args, ["a"]);
}

#[test]
fn a_content_group_round_trips() {
    let mut conn = pack();
    let group = ContentGroup {
        id: "feet".to_string(),
        label: "Feet".to_string(),
        description: Some("Toes and such".to_string()),
        tags: vec!["feet".to_string()],
        enabled_by_default: false,
    };
    tx_on(&mut conn, |tx| {
        add_content_group(tx, &group).unwrap();
    });
    assert_eq!(content_groups(&conn).unwrap(), [group]);
}

/// The group id is the author-facing key the resolver turns into `content_group.<id>`, so two
/// groups cannot share one.
#[test]
fn a_duplicate_group_id_is_refused() {
    let mut conn = pack();
    let group = ContentGroup {
        id: "feet".to_string(),
        label: "Feet".to_string(),
        description: None,
        tags: vec![],
        enabled_by_default: true,
    };
    tx_on(&mut conn, |tx| {
        add_content_group(tx, &group).unwrap();
    });
    let tx = conn.transaction().unwrap();
    assert!(add_content_group(&tx, &group).is_err());
}

#[test]
fn renaming_a_group_carries_its_tags_across() {
    let mut conn = pack();
    let group = ContentGroup {
        id: "feet".to_string(),
        label: "Feet".to_string(),
        description: None,
        tags: vec!["feet".to_string()],
        enabled_by_default: true,
    };
    tx_on(&mut conn, |tx| {
        add_content_group(tx, &group).unwrap();
        assert!(update_content_group(
            tx,
            "feet",
            &ContentGroup {
                id: "toes".to_string(),
                ..group.clone()
            }
        )
        .unwrap());
    });
    let groups = content_groups(&conn).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, "toes");
    assert_eq!(groups[0].tags, ["feet"]);
}

/// Everything this module writes has to be readable through the whole-document path the engine,
/// the converter and `lw` still use.
#[test]
fn what_the_editor_writes_the_engine_reads() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        add_text_item(tx, PoolKind::Caption, &item("hello")).unwrap();
        add_web_link(
            tx,
            &WebLink {
                url: "https://example.invalid".to_string(),
                args: vec![],
                tags: vec![],
            },
        )
        .unwrap();
        add_content_group(
            tx,
            &ContentGroup {
                id: "feet".to_string(),
                label: "Feet".to_string(),
                description: None,
                tags: vec![],
                enabled_by_default: true,
            },
        )
        .unwrap();
    });

    let document = storage::read(&conn).unwrap();
    assert_eq!(
        document.content,
        Content {
            captions: vec![item("hello")],
            web_links: vec![WebLink {
                url: "https://example.invalid".to_string(),
                args: vec![],
                tags: vec![],
            }],
            content_groups: vec![ContentGroup {
                id: "feet".to_string(),
                label: "Feet".to_string(),
                description: None,
                tags: vec![],
                enabled_by_default: true,
            }],
            ..Behaviour::new().content
        }
    );
}

// ── The timeline ─────────────────────────────────────────────────────────────

fn timeline_pack(stage_ids: &[&str]) -> Connection {
    let mut conn = pack();
    let behaviour = Behaviour {
        experience: Some(Experience {
            timeline: Timeline {
                stages: stage_ids.iter().map(|id| stage(id)).collect(),
                transitions: vec![],
            },
            label: None,
        }),
        ..Behaviour::new()
    };
    let tx = conn.transaction().unwrap();
    storage::write(&tx, &behaviour).unwrap();
    tx.commit().unwrap();
    // Written straight from a document, the stage ends are whatever the document said. Normalize
    // through a real edit so the fixture starts in the state the invariants describe.
    let mut conn = conn;
    tx_on(&mut conn, |tx| {
        let id = add_stage(tx, None, None, "scratch").unwrap();
        remove_stage(tx, &id).unwrap();
    });
    conn
}

fn stage(id: &str) -> Stage {
    Stage {
        id: id.to_string(),
        label: id.to_string(),
        end: None,
        content: ContentSelection::default(),
        events: Events::default(),
        movement: None,
        mitosis: None,
        on_enter: Default::default(),
        prompt: Default::default(),
    }
}

fn stage_ids(conn: &Connection) -> Vec<String> {
    stages(conn).unwrap().into_iter().map(|s| s.id).collect()
}

fn edges(conn: &Connection) -> Vec<(String, String)> {
    transitions(conn)
        .unwrap()
        .into_iter()
        .map(|t| (t.from_stage, t.to_stage))
        .collect()
}

/// The first invariant: there is nothing after the last stage to move on to, so an end condition
/// there would be a promise the mode cannot keep.
#[test]
fn only_the_final_stage_is_unbounded() {
    let conn = timeline_pack(&["a", "b"]);
    let stages = stages(&conn).unwrap();
    assert_eq!(
        stages[0].end.as_ref().unwrap().duration_seconds,
        Some(DEFAULT_STAGE_SECONDS)
    );
    assert_eq!(stages[1].end, None);
    assert_eq!(edges(&conn).len(), 1);
}

/// The second invariant, and the reason transitions are matched by endpoint rather than rebuilt:
/// reordering stages around a transition must not reset a duration the author set on it.
#[test]
fn a_transition_between_still_adjacent_stages_keeps_its_settings() {
    let mut conn = timeline_pack(&["a", "b", "c"]);
    let ab = transitions(&conn)
        .unwrap()
        .into_iter()
        .find(|t| t.from_stage == "a" && t.to_stage == "b")
        .unwrap();
    tx_on(&mut conn, |tx| {
        assert!(update_transition(
            tx,
            &ab.id,
            &Transition {
                duration_seconds: 12.5,
                easing: Easing::EaseInOut,
                affected: vec![TransitionCategory::PopupInterval],
                ..ab.clone()
            }
        )
        .unwrap());
        // Moving "c" to the end leaves a→b adjacent, so its transition must survive intact.
        move_stage(tx, "c", 2).unwrap();
    });

    let kept = transitions(&conn)
        .unwrap()
        .into_iter()
        .find(|t| t.from_stage == "a" && t.to_stage == "b")
        .unwrap();
    assert_eq!(kept.id, ab.id);
    assert_eq!(kept.duration_seconds, 12.5);
    assert_eq!(kept.easing, Easing::EaseInOut);
    assert_eq!(kept.affected, [TransitionCategory::PopupInterval]);
}

#[test]
fn moving_a_stage_rebuilds_only_the_edges_that_changed() {
    let mut conn = timeline_pack(&["a", "b", "c"]);
    tx_on(&mut conn, |tx| {
        assert!(move_stage(tx, "c", 1).unwrap());
    });
    assert_eq!(stage_ids(&conn), ["a", "c", "b"]);
    assert_eq!(
        edges(&conn),
        [
            ("a".to_string(), "c".to_string()),
            ("c".to_string(), "b".to_string())
        ]
    );
}

#[test]
fn moving_a_stage_to_where_it_already_is_reports_no_change() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        assert!(!move_stage(tx, "a", 0).unwrap());
    });
}

#[test]
fn a_duplicate_carries_the_settings_but_not_the_tag_ownership() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        let mut source = read_one_stage(tx, "a").unwrap().unwrap();
        source.events.popup = Some(EventSchedule {
            interval: Interval::Fixed { seconds: 20.0 },
            initial_delay_seconds: None,
            max_concurrent: None,
        });
        source.content.tags = Some(vec!["stage-a".to_string()]);
        source.content.owned_tag = Some("stage-a".to_string());
        assert!(update_stage(tx, "a", &source).unwrap());
    });

    let mut copy_id = String::new();
    tx_on(&mut conn, |tx| {
        copy_id = duplicate_stage(tx, "a").unwrap();
    });

    let copy = read_one_stage(&conn, &copy_id).unwrap().unwrap();
    assert_ne!(copy.id, "a");
    assert_eq!(copy.label, "a copy");
    assert_eq!(copy.events.popup.unwrap().interval, Interval::Fixed { seconds: 20.0 });
    // The copy keeps the selection -- it would show nothing otherwise -- but owns none of it: two
    // stages claiming one tag would mean renaming either rewrote a name the other reads.
    assert_eq!(copy.content.tags, Some(vec!["stage-a".to_string()]));
    assert_eq!(copy.content.owned_tag, None);
    assert_eq!(stage_ids(&conn), ["a", &copy_id, "b"]);
}

#[test]
fn adding_a_stage_lands_after_the_one_it_was_added_from() {
    let mut conn = timeline_pack(&["a", "b"]);
    let mut id = String::new();
    tx_on(&mut conn, |tx| {
        id = add_stage(tx, Some("a"), None, "new").unwrap();
    });
    assert_eq!(stage_ids(&conn), ["a", id.as_str(), "b"]);
}

#[test]
fn removing_a_stage_closes_the_timeline_over_it() {
    let mut conn = timeline_pack(&["a", "b", "c"]);
    tx_on(&mut conn, |tx| {
        assert!(remove_stage(tx, "b").unwrap());
    });
    assert_eq!(stage_ids(&conn), ["a", "c"]);
    assert_eq!(edges(&conn), [("a".to_string(), "c".to_string())]);
}

/// A timeline with no stages is not a state the editor can show or the mode can play.
#[test]
fn the_last_stage_cannot_be_removed() {
    let mut conn = timeline_pack(&["a"]);
    let tx = conn.transaction().unwrap();
    assert!(remove_stage(&tx, "a").is_err());
}

/// Editing one stage must not disturb the rows of the stages around it — the reason undo can
/// afford a hundred entries.
#[test]
fn editing_one_stage_leaves_its_neighbours_alone() {
    let mut conn = timeline_pack(&["a", "b", "c"]);
    let before = stages(&conn).unwrap();
    tx_on(&mut conn, |tx| {
        let mut target = read_one_stage(tx, "b").unwrap().unwrap();
        target.label = "renamed".to_string();
        assert!(update_stage(tx, "b", &target).unwrap());
    });
    let after = stages(&conn).unwrap();
    assert_eq!(after[0], before[0]);
    assert_eq!(after[2], before[2]);
    assert_eq!(after[1].label, "renamed");
}

/// A stage's id addresses the row and the transitions point at it, so an update cannot move it.
#[test]
fn updating_a_stage_cannot_change_its_id() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        let mut target = read_one_stage(tx, "a").unwrap().unwrap();
        target.id = "elsewhere".to_string();
        update_stage(tx, "a", &target).unwrap();
    });
    assert_eq!(stage_ids(&conn), ["a", "b"]);
}

#[test]
fn a_stage_edit_that_changes_nothing_reports_no_change() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        let target = read_one_stage(tx, "a").unwrap().unwrap();
        assert!(!update_stage(tx, "a", &target).unwrap());
    });
}

/// The editor's timeline view and the engine's document view have to agree about the stages, or
/// the author is editing something the pack will not play.
#[test]
fn the_editors_timeline_is_the_documents_timeline() {
    let conn = timeline_pack(&["a", "b"]);
    let view = timeline(&conn).unwrap().unwrap();
    let document = storage::read(&conn).unwrap().experience.unwrap();
    assert_eq!(view.timeline, document.timeline);
}

// ── Per-file attributes ──────────────────────────────────────────────────────

fn popups_of(conn: &Connection, id: u64) -> Option<PopupMedia> {
    popup_attributes(conn, &[id])
        .unwrap()
        .into_iter()
        .next()
        .map(|(_, entry)| entry)
}

/// The rule the whole section is built around: a file the author has said nothing about has no
/// entry at all, as distinct from an entry full of today's defaults.
#[test]
fn a_file_with_no_opinion_has_no_entry() {
    let conn = pack();
    assert_eq!(popup_attributes(&conn, &[1, 2, 3]).unwrap(), []);
}

#[test]
fn setting_one_field_leaves_the_others_unset() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                scale: Some(Some(2.0)),
                ..Default::default()
            }
        )
        .unwrap());
    });
    let entry = popups_of(&conn, 1).unwrap();
    assert_eq!(entry.scale, Some(2.0));
    assert_eq!(entry.weight, None);
    assert_eq!(entry.caption, None);
}

/// The distinction the double option exists for: an omitted field is left alone, a null one is
/// cleared. Collapsing them would make "set the scale" silently wipe the caption.
#[test]
fn an_omitted_field_is_left_alone_and_a_null_one_is_cleared() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                scale: Some(Some(2.0)),
                caption: Some(Some("hello".to_string())),
                ..Default::default()
            },
        )
        .unwrap();
        // Mentions scale only.
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                scale: Some(Some(3.0)),
                ..Default::default()
            },
        )
        .unwrap();
    });
    let entry = popups_of(&conn, 1).unwrap();
    assert_eq!(entry.scale, Some(3.0));
    assert_eq!(entry.caption.as_deref(), Some("hello"));

    tx_on(&mut conn, |tx| {
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                caption: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    });
    let entry = popups_of(&conn, 1).unwrap();
    assert_eq!(entry.caption, None);
    assert_eq!(entry.scale, Some(3.0));
}

/// Clearing the last thing an entry said removes the entry, rather than leaving a row that means
/// nothing — the same answer a whole-document write reaches.
#[test]
fn clearing_the_last_field_drops_the_entry() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                scale: Some(Some(2.0)),
                ..Default::default()
            },
        )
        .unwrap();
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                scale: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

/// The inspector edits a selection, so one command has to reach every file in it.
#[test]
fn a_change_applies_across_a_selection() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_popup_attributes(
            tx,
            &[1, 2],
            &PopupChanges {
                weight: Some(Some(5.0)),
                ..Default::default()
            }
        )
        .unwrap());
    });
    assert_eq!(popups_of(&conn, 1).unwrap().weight, Some(5.0));
    assert_eq!(popups_of(&conn, 2).unwrap().weight, Some(5.0));
}

#[test]
fn an_attribute_change_that_changes_nothing_reports_no_change() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                scale: Some(Some(2.0)),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                scale: Some(Some(2.0)),
                ..Default::default()
            }
        )
        .unwrap());
    });
}

/// Clearing a field on a file that never had an entry must not create one just to hold nothing.
#[test]
fn clearing_a_field_on_a_file_with_no_entry_writes_nothing() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(!set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                scale: Some(None),
                ..Default::default()
            }
        )
        .unwrap());
    });
    assert_eq!(popups_of(&conn, 1), None);
}

/// An empty caption says nothing, so it clears the field rather than storing "" — otherwise the
/// entry survives holding a caption the author deleted.
#[test]
fn an_empty_caption_clears_rather_than_stores() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                caption: Some(Some("hello".to_string())),
                ..Default::default()
            },
        )
        .unwrap();
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                caption: Some(Some(String::new())),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

#[test]
fn explicit_sound_pairings_round_trip() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                audio: Some(vec![3]),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(popups_of(&conn, 1).unwrap().audio, [3]);

    // A set has no separate null: emptying the list is how a pairing is removed, and that leaves
    // the entry saying nothing.
    tx_on(&mut conn, |tx| {
        set_popup_attributes(
            tx,
            &[1],
            &PopupChanges {
                audio: Some(vec![]),
                ..Default::default()
            },
        )
        .unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

#[test]
fn audio_attributes_follow_the_same_rules() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_audio_attributes(
            tx,
            &[3],
            &AudioChanges {
                volume: Some(Some(0.5))
            }
        )
        .unwrap());
    });
    assert_eq!(
        audio_attributes(&conn, &[3]).unwrap(),
        [(3, AudioMedia { volume: Some(0.5) })]
    );

    tx_on(&mut conn, |tx| {
        assert!(set_audio_attributes(tx, &[3], &AudioChanges { volume: Some(None) }).unwrap());
    });
    assert_eq!(audio_attributes(&conn, &[3]).unwrap(), []);
}

/// Absent and null have to survive the IPC boundary as different messages, or the whole double
/// option is decoration.
#[test]
fn absent_and_null_deserialize_differently() {
    let absent: PopupChanges = serde_json::from_str("{}").unwrap();
    assert_eq!(absent.scale, None);
    let cleared: PopupChanges = serde_json::from_str(r#"{"scale":null}"#).unwrap();
    assert_eq!(cleared.scale, Some(None));
    let set: PopupChanges = serde_json::from_str(r#"{"scale":2.0}"#).unwrap();
    assert_eq!(set.scale, Some(Some(2.0)));
}

#[test]
fn the_pack_media_slots_round_trip() {
    let mut conn = pack();
    let behaviour = Behaviour {
        content: Content {
            wallpaper: Some(1),
            splash: Some(2),
            ..Behaviour::new().content
        },
        ..Behaviour::new()
    };
    tx_on(&mut conn, |tx| {
        storage::write(tx, &behaviour).unwrap();
    });
    assert_eq!(
        media_slots(&conn).unwrap(),
        MediaSlots {
            wallpaper: Some(1),
            splash: Some(2)
        }
    );
}

// ── Slots ────────────────────────────────────────────────────────────────────

use crate::behaviour::MediaSlot;

#[test]
fn a_pack_slot_reads_back_what_was_written() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap());
    });
    assert_eq!(slot_value(&conn, &MediaSlot::Wallpaper).unwrap(), Some(1));
    assert_eq!(slot_value(&conn, &MediaSlot::Splash).unwrap(), None);
    assert_eq!(media_slots(&conn).unwrap().wallpaper, Some(1));
}

#[test]
fn emptying_a_slot_clears_it() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap();
        assert!(set_media_slot(tx, &MediaSlot::Wallpaper, None).unwrap());
    });
    assert_eq!(slot_value(&conn, &MediaSlot::Wallpaper).unwrap(), None);
}

#[test]
fn setting_a_slot_to_what_it_holds_reports_no_change() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap();
        assert!(!set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap());
    });
}

#[test]
fn a_stages_optional_rows_are_created_when_a_slot_first_fills_one() {
    let mut conn = timeline_pack(&["a", "b"]);
    let slot = MediaSlot::StageEntrySplash {
        stage: "a".to_string(),
    };
    assert_eq!(slot_value(&conn, &slot).unwrap(), None);
    tx_on(&mut conn, |tx| {
        assert!(set_media_slot(tx, &slot, Some(1)).unwrap());
    });
    assert_eq!(slot_value(&conn, &slot).unwrap(), Some(1));

    let prompt = MediaSlot::StagePromptSound {
        stage: "a".to_string(),
    };
    tx_on(&mut conn, |tx| {
        assert!(set_media_slot(tx, &prompt, Some(3)).unwrap());
    });
    assert_eq!(slot_value(&conn, &prompt).unwrap(), Some(3));
}

/// Naming a track and "pick one at random on entry" are alternatives, so choosing a track has to
/// switch the random flag off or the stage claims both.
#[test]
fn naming_a_stage_track_switches_off_random_selection() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        let mut target = read_one_stage(tx, "a").unwrap().unwrap();
        target.content.audio_random = true;
        update_stage(tx, "a", &target).unwrap();
        set_media_slot(
            tx,
            &MediaSlot::StageAudio {
                stage: "a".to_string(),
            },
            Some(3),
        )
        .unwrap();
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.content.audio, Some(3));
    assert!(!stage.content.audio_random);
}

#[test]
fn a_slot_on_a_removed_stage_is_refused() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        remove_stage(tx, "b").unwrap();
    });
    let tx = conn.transaction().unwrap();
    assert!(set_media_slot(
        &tx,
        &MediaSlot::StageWallpaper {
            stage: "b".to_string()
        },
        Some(1)
    )
    .is_err());
}

/// The hazard this section removes: a slot written while the timeline is switched off used to go
/// through a whole-document round trip, and a document read in that state has no stages in it.
#[test]
fn filling_a_slot_while_the_timeline_is_off_keeps_the_stages() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        set_experience_enabled(tx, false).unwrap();
        set_media_slot(tx, &MediaSlot::Wallpaper, Some(1)).unwrap();
    });
    assert_eq!(stage_ids(&conn), ["a", "b"]);
    assert_eq!(slot_value(&conn, &MediaSlot::Wallpaper).unwrap(), Some(1));
}
