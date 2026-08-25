use super::*;

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
    assert_eq!(
        stage.content.tags.as_deref(),
        Some(&["stage-a".to_string()][..])
    );
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
        assert!(
            set_stage_movement(
                tx,
                "a",
                Some(&Movement {
                    minimum_speed: Some(50.0),
                    maximum_speed: Some(150.0)
                })
            )
            .unwrap()
        );
        assert!(set_stage_movement_speed(tx, "a", Some(80.0), None).unwrap());
    });
    let stage = read_one_stage(&conn, "a").unwrap().unwrap();
    let movement = stage.movement.unwrap();
    assert_eq!(movement.minimum_speed, Some(80.0));
    assert_eq!(
        movement.maximum_speed,
        Some(150.0),
        "the other speed is untouched"
    );
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
