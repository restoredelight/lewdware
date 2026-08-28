use rusqlite::Connection;

use super::{pack, stage, store};
use crate::behaviour::editor::{set_timeline_enabled, timeline as read_timeline};
use crate::behaviour::schema::{
    Behaviour, Easing, EventSchedule, Interval, Mitosis, Movement, Timeline, Transition,
    TransitionCategory,
};

/// The three-state tag field. "No restriction" and "restricted to nothing" are different
/// answers and an empty table cannot tell them apart on its own.
#[test]
fn an_empty_tag_selection_stays_distinct_from_no_selection() {
    let mut conn = pack();
    let mut unrestricted = stage("a");
    unrestricted.content.tags = None;
    let mut restricted = stage("b");
    restricted.content.tags = Some(vec![]);

    let behaviour = Behaviour {
        timeline: Some(Timeline {
            stages: vec![unrestricted, restricted],
            transitions: vec![],
            label: None,
        }),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    let stages = Behaviour::read(&conn)
        .unwrap()
        .timeline
        .unwrap()
        .stages;
    assert_eq!(stages[0].content.tags, None);
    assert_eq!(stages[1].content.tags, Some(vec![]));
}

/// Same shape for the optional sub-structures: `Some(Movement { .. all None })` is a stage
/// that has movement with no speeds set, not a stage with no movement.
#[test]
fn an_empty_sub_structure_stays_distinct_from_an_absent_one() {
    let mut conn = pack();
    let mut present = stage("a");
    present.movement = Some(Movement {
        minimum_speed: None,
        maximum_speed: None,
    });
    present.mitosis = Some(Mitosis {
        chance: None,
        count: None,
    });
    let absent = stage("b");

    let behaviour = Behaviour {
        timeline: Some(Timeline {
            stages: vec![present, absent],
            transitions: vec![],
            label: None,
        }),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    let stages = Behaviour::read(&conn)
        .unwrap()
        .timeline
        .unwrap()
        .stages;
    assert_eq!(
        stages[0].movement,
        Some(Movement {
            minimum_speed: None,
            maximum_speed: None
        })
    );
    assert_eq!(
        stages[0].mitosis,
        Some(Mitosis {
            chance: None,
            count: None
        })
    );
    assert_eq!(stages[1].movement, None);
    assert_eq!(stages[1].mitosis, None);
}

/// A pack with no timeline, versus one whose timeline simply doesn't rename the mode. Both
/// read back as themselves rather than collapsing into each other.
#[test]
fn a_suspended_timeline_is_distinct_from_an_unlabelled_one() {
    let mut conn = pack();
    let with_timeline = Behaviour {
        timeline: Some(Timeline::default()),
        ..Behaviour::new()
    };
    store(&mut conn, &with_timeline);
    assert_eq!(
        Behaviour::read(&conn).unwrap().timeline,
        Some(Timeline::default())
    );

    store(&mut conn, &Behaviour::new());
    assert_eq!(Behaviour::read(&conn).unwrap().timeline, None);
}

/// The reason writes diff instead of replacing: undo is a changeset, so a wholesale rewrite
/// would record every stage as changed on every keystroke. Editing one stage's label must
/// leave every other row exactly as it was.
#[test]
fn editing_one_stage_leaves_the_other_rows_untouched() {
    let mut conn = pack();
    let mut behaviour = Behaviour {
        timeline: Some(Timeline {
            stages: vec![stage("a"), stage("b"), stage("c")],
            transitions: vec![],
            label: None,
        }),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    // `data_version` ticks once per committed write to the database file, so it cannot show
    // which rows moved. Row-level `rowid`s can: an untouched row keeps its identity, while a
    // delete-and-reinsert gives it a new one.
    let rowids = |conn: &Connection| -> Vec<(String, i64)> {
        let mut statement = conn
            .prepare("SELECT id, rowid FROM behaviour_stage ORDER BY position")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    };
    let before = rowids(&conn);

    behaviour.timeline.as_mut().unwrap().stages[1].label = "Renamed".to_string();
    store(&mut conn, &behaviour);

    assert_eq!(before, rowids(&conn), "no stage row was replaced");
    let stages = Behaviour::read(&conn)
        .unwrap()
        .timeline
        .unwrap()
        .stages;
    assert_eq!(stages[1].label, "Renamed");
}

/// Removing a stage takes its child rows with it, and the transition that ran through it --
/// the cascades the schema declares, rather than a cleanup written out longhand.
#[test]
fn removing_a_stage_takes_its_children_and_transitions() {
    let mut conn = pack();
    let mut first = stage("a");
    first.content.tags = Some(vec!["kinky".to_string()]);
    first.movement = Some(Movement {
        minimum_speed: None,
        maximum_speed: None,
    });
    first.events.popup = Some(EventSchedule {
        interval: Interval::Fixed { seconds: 30.0 },
        initial_delay_seconds: None,
        max_concurrent: None,
    });
    let mut behaviour = Behaviour {
        timeline: Some(Timeline {
            stages: vec![first, stage("b")],
            transitions: vec![Transition {
                id: "t1".to_string(),
                from_stage: "a".to_string(),
                to_stage: "b".to_string(),
                duration_seconds: 0.0,
                easing: Easing::Linear,
                affected: vec![TransitionCategory::Events],
            }],
            label: None,
        }),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    let timeline = behaviour.timeline.as_mut().unwrap();
    timeline.stages.remove(0);
    timeline.transitions.clear();
    store(&mut conn, &behaviour);

    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |row| row.get(0)).unwrap() };
    assert_eq!(count("SELECT COUNT(*) FROM behaviour_stage"), 1);
    assert_eq!(count("SELECT COUNT(*) FROM behaviour_stage_tag"), 0);
    assert_eq!(count("SELECT COUNT(*) FROM behaviour_stage_movement"), 0);
    assert_eq!(count("SELECT COUNT(*) FROM behaviour_stage_event"), 0);
    assert_eq!(count("SELECT COUNT(*) FROM behaviour_transition"), 0);
    assert_eq!(
        count("SELECT COUNT(*) FROM behaviour_transition_category"),
        0
    );
}

/// Suspension is the whole reason `behaviour_timeline.enabled` exists: the engine must see a
/// switched-off timeline as no timeline at all, while the editor must still be able to show the
/// stages so switching it back on returns what was there.
#[test]
fn a_switched_off_timeline_reads_as_none_but_keeps_its_stages() {
    let mut conn = pack();
    let behaviour = Behaviour {
        timeline: Some(Timeline {
            stages: vec![stage("a"), stage("b")],
            transitions: vec![],
            label: Some("Corruption".to_string()),
        }),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    let tx = conn.transaction().unwrap();
    set_timeline_enabled(&tx, false).unwrap();
    tx.commit().unwrap();

    // What the engine, the converter and `lw` see.
    assert_eq!(Behaviour::read(&conn).unwrap().timeline, None);

    // What the editor sees.
    let view = read_timeline(&conn).unwrap().unwrap();
    assert!(!view.enabled);
    assert_eq!(view.timeline.label.as_deref(), Some("Corruption"));
    assert_eq!(view.timeline.stages.len(), 2);

    let tx = conn.transaction().unwrap();
    set_timeline_enabled(&tx, true).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        Behaviour::read(&conn).unwrap().timeline,
        behaviour.timeline
    );
}

/// A pack that has never had a timeline has no section to switch on or off, and reads as `None`
/// from both views -- as distinct from one whose timeline is merely switched off.
#[test]
fn a_pack_with_no_timeline_has_no_timeline_view_at_all() {
    let conn = pack();
    assert_eq!(Behaviour::read(&conn).unwrap().timeline, None);
    assert_eq!(read_timeline(&conn).unwrap(), None);
}

/// Switching a timeline on for a pack that never had one creates the section, so the author's
/// first stage has somewhere to live.
#[test]
fn switching_on_a_pack_with_no_timeline_creates_the_section() {
    let mut conn = pack();
    let tx = conn.transaction().unwrap();
    set_timeline_enabled(&tx, true).unwrap();
    tx.commit().unwrap();

    let view = read_timeline(&conn).unwrap().unwrap();
    assert!(view.enabled);
    assert!(view.timeline.stages.is_empty());
}

/// A toggle landing on the value already stored is still not an edit, and must not cost one of
/// the hundred entries undo keeps -- but that is now the changeset's guarantee rather than this
/// setter's, and the editor tests it (`an_edit_that_changes_nothing_records_no_history_entry`). What is left to check
/// here is that writing it twice is accepted and leaves the value where it was.
#[test]
fn switching_to_the_current_value_keeps_it() {
    let mut conn = pack();
    let tx = conn.transaction().unwrap();
    set_timeline_enabled(&tx, true).unwrap();
    set_timeline_enabled(&tx, true).unwrap();
    tx.commit().unwrap();
    assert!(read_timeline(&conn).unwrap().unwrap().enabled);
}

/// Writing a whole document that has a timeline clears any suspension: the document is saying the
/// pack has one, and a stale flag would keep it hidden.
#[test]
fn a_whole_document_write_unsuspends_the_timeline() {
    let mut conn = pack();
    let behaviour = Behaviour {
        timeline: Some(Timeline::default()),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);
    let tx = conn.transaction().unwrap();
    set_timeline_enabled(&tx, false).unwrap();
    tx.commit().unwrap();
    assert_eq!(Behaviour::read(&conn).unwrap().timeline, None);

    store(&mut conn, &behaviour);
    assert_eq!(
        Behaviour::read(&conn).unwrap().timeline,
        Some(Timeline::default())
    );
}
