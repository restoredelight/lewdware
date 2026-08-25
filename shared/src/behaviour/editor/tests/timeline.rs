use super::*;

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
        assert!(set_transition_affected(tx, &ab.id, &[TransitionCategory::PopupInterval]).unwrap());
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
    assert_eq!(
        copy.events.popup.unwrap().interval,
        Interval::Fixed { seconds: 20.0 }
    );
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
