use super::*;
use crate::behaviour::storage;
use crate::behaviour::{
    Behaviour, Content, Easing, EventKind, EventSchedule, Events, Experience, Interval, Movement,
    Timeline, TransitionCategory,
};

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

fn arg_values(row: &WebLinkRow) -> Vec<String> {
    row.args.iter().map(|arg| arg.value.clone()).collect()
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
        assert!(set_text_item_text(tx, ids[1], "edited").unwrap());
    });

    assert_eq!(texts(&conn, PoolKind::Caption), ["one", "edited", "three"]);
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
    assert_eq!(rows[0].url, link.url);
    assert_eq!(rows[0].tags, link.tags);
    // The same suffix twice is a legitimate way to weight it, so args are a list, not a set.
    assert_eq!(arg_values(&rows[0]), link.args);
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
        let args = web_links(tx).unwrap()[0].args.clone();
        assert!(remove_web_link_arg(tx, id, args[1].id).unwrap());
        assert!(remove_web_link_arg(tx, id, args[2].id).unwrap());
    });
    assert_eq!(arg_values(&web_links(&conn).unwrap()[0]), ["a"]);
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
        assert!(set_transition_duration(tx, &ab.id, 12.5).unwrap());
        assert!(set_transition_easing(tx, &ab.id, Easing::EaseInOut).unwrap());
        assert!(
            set_transition_affected(tx, &ab.id, &[TransitionCategory::PopupInterval]).unwrap()
        );
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
        set_stage_event(
            tx,
            "a",
            EventKind::Popup,
            Some(&EventSchedule {
                interval: Interval::Fixed { seconds: 20.0 },
                initial_delay_seconds: None,
                max_concurrent: None,
            }),
        )
        .unwrap();
        assert!(
            set_stage_content_tags(tx, "a", Some(&["stage-a".to_string()]), Some("stage-a"))
                .unwrap()
        );
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
        assert!(set_stage_label(tx, "b", "renamed").unwrap());
    });
    let after = stages(&conn).unwrap();
    assert_eq!(after[0], before[0]);
    assert_eq!(after[2], before[2]);
    assert_eq!(after[1].label, "renamed");
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
        assert!(set_popup_scale(tx, &[1], Some(2.0)).unwrap());
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
        set_popup_scale(tx, &[1], Some(2.0)).unwrap();
        set_popup_caption(tx, &[1], Some("hello")).unwrap();
        // Setting the scale again says nothing about the caption.
        set_popup_scale(tx, &[1], Some(3.0)).unwrap();
    });
    let entry = popups_of(&conn, 1).unwrap();
    assert_eq!(entry.scale, Some(3.0));
    assert_eq!(entry.caption.as_deref(), Some("hello"));

    tx_on(&mut conn, |tx| {
        set_popup_caption(tx, &[1], None).unwrap();
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
        set_popup_scale(tx, &[1], Some(2.0)).unwrap();
        set_popup_scale(tx, &[1], None).unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

/// The inspector edits a selection, so one command has to reach every file in it.
#[test]
fn a_change_applies_across_a_selection() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_popup_weight(tx, &[1, 2], Some(5.0)).unwrap());
    });
    assert_eq!(popups_of(&conn, 1).unwrap().weight, Some(5.0));
    assert_eq!(popups_of(&conn, 2).unwrap().weight, Some(5.0));
}

#[test]
fn an_attribute_change_that_changes_nothing_reports_no_change() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_scale(tx, &[1], Some(2.0)).unwrap();
        assert!(!set_popup_scale(tx, &[1], Some(2.0)).unwrap());
    });
}

/// Clearing a field on a file that never had an entry must not create one just to hold nothing.
#[test]
fn clearing_a_field_on_a_file_with_no_entry_writes_nothing() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(!set_popup_scale(tx, &[1], None).unwrap());
    });
    assert_eq!(popups_of(&conn, 1), None);
}

/// An empty caption says nothing, so it clears the field rather than storing "" — otherwise the
/// entry survives holding a caption the author deleted.
#[test]
fn an_empty_caption_clears_rather_than_stores() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_caption(tx, &[1], Some("hello")).unwrap();
        set_popup_caption(tx, &[1], Some("")).unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

#[test]
fn explicit_sound_pairings_round_trip() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        edit_popup_entries(tx, &[1], |entry| entry.audio = vec![3]).unwrap();
    });
    assert_eq!(popups_of(&conn, 1).unwrap().audio, [3]);

    // A set has no separate null: emptying the list is how a pairing is removed, and that leaves
    // the entry saying nothing.
    tx_on(&mut conn, |tx| {
        edit_popup_entries(tx, &[1], |entry| entry.audio = vec![]).unwrap();
    });
    assert_eq!(popups_of(&conn, 1), None);
}

#[test]
fn audio_attributes_follow_the_same_rules() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        assert!(set_audio_volume(tx, &[3], Some(0.5)).unwrap());
    });
    assert_eq!(
        audio_attributes(&conn, &[3]).unwrap(),
        [(3, AudioMedia { volume: Some(0.5) })]
    );

    tx_on(&mut conn, |tx| {
        assert!(set_audio_volume(tx, &[3], None).unwrap());
    });
    assert_eq!(audio_attributes(&conn, &[3]).unwrap(), []);
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
        set_stage_audio_random(tx, "a", true).unwrap();
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

// ── Setting one field ────────────────────────────────────────────────────────

/// The property the whole per-field design rests on: two writes touching different fields of one
/// entity commute, so neither carries a stale copy of the other's work. This is what made the
/// editor's draft-merging machinery unnecessary.
#[test]
fn two_fields_of_one_entry_do_not_clobber_each_other() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_text_item(tx, PoolKind::Notification, &item("body")).unwrap();
        // Both writes are built against the entry as it was *before* either of them.
        set_text_item_summary(tx, id, Some("title")).unwrap();
        set_text_item_text(tx, id, "new body").unwrap();
    });

    let stored = &text_pool(&conn, PoolKind::Notification).unwrap()[0].item;
    assert_eq!(stored.summary.as_deref(), Some("title"));
    assert_eq!(stored.text, "new body");
}

#[test]
fn a_setter_reports_no_change_for_the_value_already_stored() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_text_item(tx, PoolKind::Caption, &item("same")).unwrap();
        assert!(!set_text_item_text(tx, id, "same").unwrap());
        assert!(set_text_item_text(tx, id, "different").unwrap());
        assert!(!set_text_item_timeout(tx, id, None).unwrap());
        assert!(set_text_item_timeout(tx, id, Some(4.0)).unwrap());
    });
}

/// A stale editor writing into an entry that has since been removed must be told, not quietly given
/// the entry back — re-creating the row would undo the removal.
#[test]
fn a_setter_refuses_a_row_that_is_gone() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_text_item(tx, PoolKind::Caption, &item("gone")).unwrap();
        remove_text_item(tx, id).unwrap();
    });
    let tx = conn.transaction().unwrap();
    assert!(set_text_item_text(&tx, id, "back").is_err());
    assert!(add_text_item_tag(&tx, id, "imp").is_err());
}

/// Ownership is recorded on the association row, so a stage cannot own a tag it does not select by.
/// That invariant used to live only in the front end's habits; it is checked here now.
#[test]
fn a_stage_cannot_own_a_tag_it_does_not_select() {
    let mut conn = timeline_pack(&["a", "b"]);
    let tx = conn.transaction().unwrap();
    assert!(
        set_stage_content_tags(&tx, "a", Some(&["kept".to_string()]), Some("elsewhere")).is_err()
    );
    // And an unrestricted stage owns nothing, because there is no selection to own a tag in.
    assert!(set_stage_content_tags(&tx, "a", None, Some("kept")).is_err());
}

#[test]
fn a_stages_selection_and_its_owned_tag_move_together() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        assert!(
            set_stage_content_tags(tx, "a", Some(&["stage-a".to_string()]), Some("stage-a"))
                .unwrap()
        );
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.content.tags.as_deref(), Some(&["stage-a".to_string()][..]));
    assert_eq!(stage.content.owned_tag.as_deref(), Some("stage-a"));

    // Dropping the restriction drops the ownership with it.
    tx_on(&mut conn, |tx| {
        assert!(set_stage_content_tags(tx, "a", None, None).unwrap());
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.content.tags, None);
    assert_eq!(stage.content.owned_tag, None);
}

/// Naming a track and picking one at random are alternatives. The exclusivity used to be enforced
/// only on the slot side, so the whole-entity write could store both.
#[test]
fn switching_on_random_audio_clears_the_named_track() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        set_media_slot(
            tx,
            &MediaSlot::StageAudio {
                stage: "a".to_string(),
            },
            Some(3),
        )
        .unwrap();
        assert!(set_stage_audio_random(tx, "a", true).unwrap());
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.content.audio, None);
    assert!(stage.content.audio_random);
}

/// The last stage must have no end condition, and whether a stage has one is decided by the
/// timeline's shape rather than by the author — so a leaf setter refuses rather than creating one.
#[test]
fn an_end_condition_leaf_refuses_the_final_stage() {
    let mut conn = timeline_pack(&["a", "b"]);
    let tx = conn.transaction().unwrap();
    assert!(set_stage_end_duration(&tx, "b", Some(60.0)).is_err());
    // The stage before it has one, so the same call lands.
    assert!(set_stage_end_duration(&tx, "a", Some(60.0)).unwrap());
}

/// Movement and mitosis are optional sub-structures: one click creates the whole thing, and a speed
/// for a stage that does not move is a stale editor rather than a request to start moving.
#[test]
fn a_speed_needs_movement_to_be_switched_on_first() {
    let mut conn = timeline_pack(&["a", "b"]);
    {
        let tx = conn.transaction().unwrap();
        assert!(set_stage_movement_speed(&tx, "a", Some(50.0), None).is_err());
    }
    tx_on(&mut conn, |tx| {
        assert!(set_stage_movement(
            tx,
            "a",
            Some(&Movement {
                minimum_speed: Some(50.0),
                maximum_speed: Some(150.0)
            })
        )
        .unwrap());
        assert!(set_stage_movement_speed(tx, "a", Some(80.0), None).unwrap());
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    let movement = stage.movement.unwrap();
    assert_eq!(movement.minimum_speed, Some(80.0));
    assert_eq!(movement.maximum_speed, Some(150.0), "the other speed is untouched");
}

#[test]
fn a_stages_prompt_and_entry_rows_are_created_on_demand() {
    let mut conn = timeline_pack(&["a", "b"]);
    tx_on(&mut conn, |tx| {
        assert!(set_stage_prompt_timeout_multiplier(tx, "a", 2.0).unwrap());
        assert!(set_stage_entry_notification(tx, "a", Some("Begin.")).unwrap());
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.prompt.timeout_multiplier, 2.0);
    assert_eq!(stage.on_enter.notification.as_deref(), Some("Begin."));
}

#[test]
fn setting_one_event_kind_leaves_the_others_alone() {
    let mut conn = timeline_pack(&["a", "b"]);
    let schedule = EventSchedule {
        interval: Interval::Fixed { seconds: 20.0 },
        initial_delay_seconds: None,
        max_concurrent: None,
    };
    tx_on(&mut conn, |tx| {
        set_stage_event(tx, "a", EventKind::Popup, Some(&schedule)).unwrap();
        set_stage_event(tx, "a", EventKind::Sound, Some(&schedule)).unwrap();
        assert!(set_stage_event(tx, "a", EventKind::Popup, None).unwrap());
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    assert_eq!(stage.events.popup, None);
    assert_eq!(stage.events.sound, Some(schedule));
}

#[test]
fn a_transition_field_moves_on_its_own() {
    let mut conn = timeline_pack(&["a", "b"]);
    let id = transitions(&conn).unwrap()[0].id.clone();
    tx_on(&mut conn, |tx| {
        assert!(set_transition_duration(tx, &id, 12.5).unwrap());
        assert!(set_transition_easing(tx, &id, Easing::EaseInOut).unwrap());
        assert!(!set_transition_easing(tx, &id, Easing::EaseInOut).unwrap());
    });
    let transition = &transitions(&conn).unwrap()[0];
    assert_eq!(transition.duration_seconds, 12.5);
    assert_eq!(transition.easing, Easing::EaseInOut);
}

#[test]
fn a_popup_field_moves_without_disturbing_its_neighbours() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        set_popup_scale(tx, &[1], Some(2.0)).unwrap();
        set_popup_weight(tx, &[1], Some(5.0)).unwrap();
    });
    let entry = popups_of(&conn, 1).unwrap();
    assert_eq!(entry.scale, Some(2.0));
    assert_eq!(entry.weight, Some(5.0));
}

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
    assert_eq!(text_pool(&conn, PoolKind::Caption).unwrap()[0].item.tags, ["c"]);
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
        positions = web_links(tx).unwrap()[0].args.iter().map(|arg| arg.id).collect();
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
    assert!(!affected.contains(&TransitionCategory::Events), "expanded away");
    assert!(!affected.contains(&TransitionCategory::PopupInterval), "the one switched off");
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
    assert_eq!(args.iter().map(|arg| &arg.value).collect::<Vec<_>>(), ["a", "c"]);
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

// ── Naming a stage's tag, and membership ─────────────────────────────────────

/// A timeline whose stages carry the given labels and selections, plus a file to move around.
fn membership_pack(stages: &[(&str, &str, Option<&[&str]>)]) -> Connection {
    let ids: Vec<&str> = stages.iter().map(|(id, _, _)| *id).collect();
    let mut conn = timeline_pack(&ids);
    tx_on(&mut conn, |tx| {
        for (id, label, tags) in stages {
            set_stage_label(tx, id, label).unwrap();
            if let Some(tags) = tags {
                let tags: Vec<String> = tags.iter().map(|tag| tag.to_string()).collect();
                set_stage_content_tags(tx, id, Some(&tags), None).unwrap();
            }
        }
    });
    conn
}

fn tag_file(conn: &Connection, media: u64, tags: &[&str]) {
    for tag in tags {
        conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", params![tag])
            .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO media_tags (media_id, tag_id)
             SELECT ?1, id FROM tags WHERE name = ?2",
            params![media, tag],
        )
        .unwrap();
    }
}

#[test]
fn a_stages_tag_is_named_after_it() {
    let conn = pack();
    let name = |label: &str| stage_tag_name(&conn, label, None).unwrap();
    assert_eq!(name("Peak"), "stage-peak");
    assert_eq!(name("The very end!"), "stage-the-very-end");
    // A label with nothing tag-shaped in it still has to produce a usable tag.
    assert_eq!(name(""), "stage");
    assert_eq!(name("!?!"), "stage");
    // A label that already says "stage" keeps the prefix it has -- `stage-stage-3` reads like a bug.
    assert_eq!(name("Stage 3"), "stage-3");
    assert_eq!(name("Stage"), "stage");
    // But only the word on its own counts: these are *about* stages, they are not one.
    assert_eq!(name("Stages of grief"), "stage-stages-of-grief");
    assert_eq!(name("Staged"), "stage-staged");
    // Not by `[a-z0-9]`: a label in another script must produce a readable tag, not an empty one.
    assert_eq!(name("Восход"), "stage-восход");
}

#[test]
fn a_new_tag_never_lands_on_a_name_the_pack_already_uses() {
    let conn = pack();
    tag_file(&conn, 1, &["stage-peak", "stage-peak-2"]);
    // Adopting `stage-peak` would silently classify every file already carrying it.
    assert_eq!(stage_tag_name(&conn, "Peak", None).unwrap(), "stage-peak-3");
    // Except the one the caller already holds: a rename must not dedupe against itself.
    assert_eq!(
        stage_tag_name(&conn, "Peak", Some("stage-peak")).unwrap(),
        "stage-peak"
    );
}

#[test]
fn a_long_label_is_cut_between_words() {
    let conn = pack();
    let name = stage_tag_name(&conn, "the part where everything happens all at once forever", None)
        .unwrap();
    assert!(name.len() <= "stage-".len() + MAX_SLUG, "{name}");
    assert!(!name.ends_with('-'), "{name}");
    assert_eq!(name, "stage-the-part-where-everything-happens-all");
}

#[test]
fn renaming_a_stage_only_renames_a_tag_the_editor_owns() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    // Selected but not owned: the author's own word for this content, which is not ours to rename.
    tx_on(&mut conn, |tx| {
        assert_eq!(stage_owned_tags(tx, "peak").unwrap(), []);
    });
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(tx, "peak", Some(&["stage-peak".to_string()]), Some("stage-peak"))
            .unwrap();
    });
    assert_eq!(
        stage_owned_tags(&conn, "peak").unwrap(),
        [OwnedStageTag {
            tag: "stage-peak".to_string(),
            excluded: false
        }]
    );
}

#[test]
fn a_tag_another_stage_selects_by_is_not_ours_to_rename() {
    let mut conn = membership_pack(&[
        ("peak", "Peak", Some(&["stage-peak"])),
        ("climax", "Climax", Some(&["stage-peak"])),
    ]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(tx, "peak", Some(&["stage-peak".to_string()]), Some("stage-peak"))
            .unwrap();
    });
    // Another stage reading this name is another author decision; ours is bookkeeping.
    assert!(stage_tag_shared(&conn, "stage-peak", "peak").unwrap());
    // Once nothing else selects by it, it is the editor's to keep in step with the stage's label.
    tx_on(&mut conn, |tx| {
        remove_stage_tag(tx, "climax", "stage-peak").unwrap();
    });
    assert!(!stage_tag_shared(&conn, "stage-peak", "peak").unwrap());
}

/// A stage that restricts nothing used to be untoggleable: it shows every file, so there was no
/// tag whose absence would exclude one. Exclusion is that tag, so the control now works everywhere.
#[test]
fn a_file_can_leave_a_stage_that_restricts_nothing() {
    let conn = membership_pack(&[("all", "All", None)]);
    let plan = plan_stage_membership(&conn, 1, "all", false).unwrap();
    assert_eq!(
        plan.creation,
        Some(StageTagCreation {
            stage: "all".to_string(),
            tag: "not-stage-all".to_string(),
            excluded: true,
        })
    );
    assert_eq!(plan.apply, ["not-stage-all"]);
    assert!(plan.remove.is_empty(), "nothing of the author's comes off");
}

#[test]
fn joining_reuses_the_stages_own_tag_and_nothing_else() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(tx, "peak", Some(&["stage-peak".to_string()]), Some("stage-peak"))
            .unwrap();
    });
    let plan = plan_stage_membership(&conn, 1, "peak", true).unwrap();
    assert_eq!(plan.apply, ["stage-peak"]);
    assert_eq!(plan.creation, None, "{plan:?}");
}

#[test]
fn joining_by_an_author_tag_would_say_more_than_appears_here_so_it_makes_its_own() {
    // `intense` is the author's, and may also drive a content group or a text pool.
    let conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    let plan = plan_stage_membership(&conn, 1, "peak", true).unwrap();
    assert_eq!(
        plan.creation,
        Some(StageTagCreation {
            stage: "peak".to_string(),
            tag: "stage-peak".to_string(),
            excluded: false,
        })
    );
    assert_eq!(plan.apply, ["stage-peak"]);
    assert!(plan.remove.is_empty());
}

/// The heart of it. A stage selecting by the author's own word can only lose a file by that file
/// losing the word -- which would also drop it from every content group and text pool naming it.
/// So the file is excluded instead, and its own tags are left exactly as the author wrote them.
#[test]
fn leaving_never_takes_off_a_tag_the_author_chose() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tag_file(&conn, 1, &["intense"]);
    tx_on(&mut conn, |tx| {
        add_content_group(
            tx,
            &ContentGroup {
                id: "intense".to_string(),
                label: "Intense".to_string(),
                description: None,
                tags: vec!["intense".to_string()],
                enabled_by_default: true,
            },
        )
        .unwrap();
    });

    let plan = plan_stage_membership(&conn, 1, "peak", false).unwrap();
    assert!(
        plan.remove.is_empty(),
        "the content group would have gone with it: {plan:?}"
    );
    assert_eq!(plan.apply, ["not-stage-peak"]);
}

/// The exception, and why it is one: a membership that exists *only* through the stage's own tag
/// is undone exactly by taking that tag off, and doing it any other way would leave `stage-peak`
/// and `not-stage-peak` sitting next to each other on the same file.
#[test]
fn leaving_by_the_stages_own_tag_takes_that_tag_off_instead_of_excluding() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(tx, "peak", Some(&["stage-peak".to_string()]), Some("stage-peak"))
            .unwrap();
    });
    tag_file(&conn, 1, &["stage-peak"]);
    let plan = plan_stage_membership(&conn, 1, "peak", false).unwrap();
    assert_eq!(plan.remove, ["stage-peak"]);
    assert!(plan.apply.is_empty(), "{plan:?}");
    assert_eq!(plan.creation, None);
}

/// ...but only when it is the *only* reason. A file also carrying an author tag the stage selects
/// by would stay in after the owned tag came off, so taking it off would destroy work for nothing.
#[test]
fn a_file_held_by_two_tags_is_excluded_rather_than_half_untagged() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(
            tx,
            "peak",
            Some(&["intense".to_string(), "stage-peak".to_string()]),
            Some("stage-peak"),
        )
        .unwrap();
    });
    tag_file(&conn, 1, &["intense", "stage-peak"]);
    let plan = plan_stage_membership(&conn, 1, "peak", false).unwrap();
    assert!(plan.remove.is_empty(), "{plan:?}");
    assert_eq!(plan.apply, ["not-stage-peak"]);
}

/// Leaving one stage leaves only that stage -- now for free, because nothing shared is removed.
#[test]
fn leaving_one_stage_leaves_only_that_stage() {
    let mut conn = membership_pack(&[
        ("peak", "Peak", Some(&["intense"])),
        ("climax", "Climax", Some(&["intense"])),
    ]);
    tag_file(&conn, 1, &["intense"]);

    let mut plan = MembershipPlan::default();
    tx_on(&mut conn, |tx| {
        plan = plan_stage_membership(tx, 1, "peak", false).unwrap();
        apply_membership_creation(tx, &plan).unwrap();
    });
    settle(&mut conn, 1, &plan);

    let stages = read_stages(&conn).unwrap();
    let tags = media_tags(&conn, 1).unwrap();
    assert!(!selects(&stages[0], &tags), "left Peak");
    assert!(selects(&stages[1], &tags), "but stayed in Climax");
    assert!(tags.contains(&"intense".to_string()), "and kept its own tag");
}

/// Off and on again, with only the stage clicked having moved -- and no tag left over.
#[test]
fn rejoining_a_stage_undoes_the_exclusion_rather_than_layering_over_it() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tag_file(&conn, 1, &["intense"]);

    let mut plan = MembershipPlan::default();
    tx_on(&mut conn, |tx| {
        plan = plan_stage_membership(tx, 1, "peak", false).unwrap();
        apply_membership_creation(tx, &plan).unwrap();
    });
    settle(&mut conn, 1, &plan);
    assert!(!selects(&read_stages(&conn).unwrap()[0], &media_tags(&conn, 1).unwrap()));

    tx_on(&mut conn, |tx| {
        plan = plan_stage_membership(tx, 1, "peak", true).unwrap();
        apply_membership_creation(tx, &plan).unwrap();
    });
    // Taking the exclusion off is the whole of it: the file was always tagged `intense`.
    assert_eq!(plan.remove, ["not-stage-peak"]);
    assert!(plan.apply.is_empty(), "{plan:?}");
    settle(&mut conn, 1, &plan);
    assert!(selects(&read_stages(&conn).unwrap()[0], &media_tags(&conn, 1).unwrap()));
}

/// An exclusion tag left on the file would silently override the tag the join just added.
#[test]
fn joining_lifts_the_exclusion_even_when_a_tag_also_has_to_be_added() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_exclude_tags(
            tx,
            "peak",
            &["not-stage-peak".to_string()],
            Some("not-stage-peak"),
        )
        .unwrap();
    });
    tag_file(&conn, 1, &["not-stage-peak"]);

    let plan = plan_stage_membership(&conn, 1, "peak", true).unwrap();
    assert_eq!(plan.remove, ["not-stage-peak"]);
    assert_eq!(plan.apply, ["stage-peak"], "and it still needs a way in");
}

/// A toggle that asks for the state the file is already in does nothing at all.
#[test]
fn a_membership_toggle_that_changes_nothing_plans_nothing() {
    let conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tag_file(&conn, 1, &["intense"]);
    assert_eq!(
        plan_stage_membership(&conn, 1, "peak", true).unwrap(),
        MembershipPlan::default()
    );
    let conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    assert_eq!(
        plan_stage_membership(&conn, 1, "peak", false).unwrap(),
        MembershipPlan::default()
    );
}

#[test]
fn a_tag_cannot_be_on_both_sides_of_one_stages_selection() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_exclude_tags(tx, "peak", &["intense".to_string()], None).unwrap();
    });
    let stage = &read_stages(&conn).unwrap()[0];
    // Otherwise the stage would claim to select by a tag that also guarantees exclusion, and show
    // nothing carrying it.
    assert_eq!(stage.content.tags.as_deref(), Some(&[][..]));
    assert_eq!(stage.content.exclude, ["intense"]);
}

#[test]
fn an_excluded_file_is_out_however_it_got_in() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["intense"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_exclude_tags(tx, "peak", &["not-stage-peak".to_string()], None).unwrap();
    });
    let stage = &read_stages(&conn).unwrap()[0];
    assert!(selects(stage, &["intense".to_string()]));
    assert!(!selects(
        stage,
        &["intense".to_string(), "not-stage-peak".to_string()]
    ));
}

/// Applies a plan's tag changes to the file, the way the tag-action phase does.
fn settle(conn: &mut Connection, media: u64, plan: &MembershipPlan) {
    for tag in &plan.apply {
        tag_file(conn, media, &[tag.as_str()]);
    }
    for tag in &plan.remove {
        conn.execute(
            "DELETE FROM media_tags WHERE media_id = ?1
             AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
            params![media, tag],
        )
        .unwrap();
    }
}

/// A stage owns up to two tags — one to put files in, one to keep them out — and everything that
/// follows a stage around has to move both. Handling one is how the other gets orphaned: a tag on
/// media that no stage names any more, which nothing in the editor can reach to clean up.
#[test]
fn a_stage_owns_both_sides_of_its_own_selection() {
    let mut conn = membership_pack(&[("peak", "Peak", Some(&["stage-peak"]))]);
    tx_on(&mut conn, |tx| {
        set_stage_content_tags(tx, "peak", Some(&["stage-peak".to_string()]), Some("stage-peak"))
            .unwrap();
        set_stage_exclude_tags(
            tx,
            "peak",
            &["not-stage-peak".to_string()],
            Some("not-stage-peak"),
        )
        .unwrap();
    });
    assert_eq!(
        stage_owned_tags(&conn, "peak").unwrap(),
        [
            OwnedStageTag {
                tag: "stage-peak".to_string(),
                excluded: false
            },
            OwnedStageTag {
                tag: "not-stage-peak".to_string(),
                excluded: true
            },
        ]
    );
}

/// Renaming the stage has to give the exclusion tag the matching new name, not the inclusion one's.
#[test]
fn the_exclusion_tag_is_renamed_with_its_own_prefix() {
    let conn = pack();
    assert_eq!(stage_tag_name(&conn, "Climax", None).unwrap(), "stage-climax");
    assert_eq!(
        stage_exclude_tag_name(&conn, "Climax", None).unwrap(),
        "not-stage-climax"
    );
    // The same rules as the inclusion side, including the "already says stage" one.
    assert_eq!(stage_exclude_tag_name(&conn, "Stage 3", None).unwrap(), "not-stage-3");
    assert_eq!(stage_exclude_tag_name(&conn, "", None).unwrap(), "not-stage");
}
