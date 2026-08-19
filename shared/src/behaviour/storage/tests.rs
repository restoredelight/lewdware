use super::*;
use crate::behaviour::{
    ContentGroup, CountScope, Easing, EndStrategy, EventKind, MonitorPreference, TextItem,
    TransitionCategory, WebLink,
};

/// A migrated pack database with a few media files, so the slot and per-item foreign keys
/// have something real to point at.
fn pack() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    crate::db::migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO media (id, file_name, file_type, \"offset\", length, hash)
         VALUES (1, 'bg.png', 'image', 0, 0, x'00'), (2, 'intro.gif', 'video', 0, 0, x'01'),
                (3, 'sting.ogg', 'audio', 0, 0, x'02')",
        [],
    )
    .unwrap();
    conn
}

fn store(conn: &mut Connection, behaviour: &Behaviour) {
    let tx = conn.transaction().unwrap();
    write(&tx, behaviour).unwrap();
    tx.commit().unwrap();
}

fn stage(id: &str) -> Stage {
    Stage {
        id: id.to_string(),
        label: format!("Stage {id}"),
        end: None,
        content: ContentSelection::default(),
        events: Events::default(),
        movement: None,
        mitosis: None,
        on_enter: Default::default(),
        prompt: Default::default(),
    }
}

/// Everything the structural half can express, in one document, through a full round trip.
#[test]
fn a_full_document_survives_the_round_trip() {
    let mut conn = pack();
    let mut first = stage("a");
    first.content.tags = Some(vec!["kinky".to_string(), "soft".to_string()]);
    first.content.wallpaper = Some(1);
    first.content.audio = Some(3);
    first.on_enter = StageEntry {
        splash: Some(1),
        sound: Some(3),
        popup_burst: Some(4),
        notification: Some("Enter".to_string()),
    };
    first.end = Some(StageEnd {
        duration_seconds: Some(300.0),
        event_count: Some(EventCountCondition {
            event: EventKind::Sound,
            count: 10,
            scope: CountScope::Session,
        }),
        strategy: EndStrategy::All,
    });
    first.movement = Some(Movement {
        minimum_speed: Some(50.0),
        maximum_speed: None,
    });
    first.mitosis = Some(Mitosis {
        chance: Some(0.5),
        count: Some(2),
    });
    first.prompt.timeout_multiplier = 1.5;
    first.prompt.popup_burst = Some(5);
    first.events.popup = Some(EventSchedule {
        interval: Interval::Fixed { seconds: 30.0 },
        initial_delay_seconds: Some(5.0),
        max_concurrent: Some(3),
    });
    first.events.web = Some(EventSchedule {
        interval: Interval::Random {
            minimum_seconds: 10.0,
            maximum_seconds: 20.0,
        },
        initial_delay_seconds: None,
        max_concurrent: None,
    });
    first.events.sound = Some(EventSchedule {
        interval: Interval::Fixed { seconds: 7.5 },
        initial_delay_seconds: Some(1.0),
        max_concurrent: None,
    });

    let behaviour = Behaviour {
        content: crate::behaviour::Content {
            popups: BTreeMap::from([
                (
                    1,
                    PopupMedia {
                        weight: Some(2.5),
                        scale: Some(1.5),
                        region: Some(SpawnRegion {
                            x: 0.5,
                            y: 0.25,
                            width: 0.5,
                            height: 0.5,
                        }),
                        monitor: Some(MonitorPreference::Primary),
                        caption: Some("Look.".to_string()),
                        video_loop: None,
                        video_audio: None,
                        audio: vec![3],
                    },
                ),
                (
                    2,
                    PopupMedia {
                        video_loop: Some(false),
                        video_audio: Some(true),
                        ..PopupMedia::default()
                    },
                ),
            ]),
            audio: BTreeMap::from([(3, AudioMedia { volume: Some(0.4) })]),
            content_groups: vec![ContentGroup {
                id: "kinky".to_string(),
                label: "Kinky".to_string(),
                description: Some("Kinky-tagged content".to_string()),
                tags: vec!["kinky".to_string(), "soft".to_string()],
                enabled_by_default: false,
            }],
            captions: vec![
                TextItem {
                    text: "Obey.".to_string(),
                    tags: vec!["kinky".to_string()],
                    timeout_seconds: None,
                    summary: None,
                },
                TextItem {
                    text: "Good.".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                },
            ],
            prompts: vec![TextItem {
                text: "Type this.".to_string(),
                tags: vec![],
                timeout_seconds: Some(30.0),
                summary: None,
            }],
            notifications: vec![TextItem {
                text: "Ping.".to_string(),
                tags: vec!["soft".to_string()],
                timeout_seconds: None,
                summary: Some("Attention".to_string()),
            }],
            web_links: vec![WebLink {
                url: "https://duckduckgo.com/?q=".to_string(),
                args: vec!["one".to_string(), "two".to_string()],
                tags: vec!["kinky".to_string()],
            }],
            wallpaper: Some(1),
            splash: Some(2),
        },
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![first, stage("b")],
                transitions: vec![Transition {
                    id: "t1".to_string(),
                    from_stage: "a".to_string(),
                    to_stage: "b".to_string(),
                    duration_seconds: 12.5,
                    easing: Easing::EaseInOut,
                    affected: vec![
                        TransitionCategory::MitosisChance,
                        TransitionCategory::PopupInterval,
                        TransitionCategory::SoundInterval,
                        TransitionCategory::Crossfade,
                    ],
                }],
            },
            label: Some("Corruption".to_string()),
        }),
    };

    store(&mut conn, &behaviour);
    assert_eq!(read(&conn).unwrap(), behaviour);
}

/// An entry that says nothing is not a row. The editor clears an attribute by nulling it, and
/// when the last one goes the entry has to stop existing rather than linger as a row of
/// NULLs -- which `read` would then hand back as a meaningless `PopupMedia::default()`.
#[test]
fn an_entry_with_nothing_set_is_not_stored() {
    let mut conn = pack();
    let mut behaviour = Behaviour::new();
    behaviour.content.popups.insert(
        1,
        PopupMedia {
            scale: Some(2.0),
            ..PopupMedia::default()
        },
    );
    behaviour
        .content
        .audio
        .insert(3, AudioMedia { volume: Some(0.5) });
    store(&mut conn, &behaviour);
    assert_eq!(read(&conn).unwrap(), behaviour);

    // Clearing the last attribute of each removes the entry entirely.
    behaviour.content.popups.insert(1, PopupMedia::default());
    behaviour.content.audio.insert(3, AudioMedia::default());
    store(&mut conn, &behaviour);

    let stored = read(&conn).unwrap();
    assert!(stored.content.popups.is_empty());
    assert!(stored.content.audio.is_empty());
    assert!(stored.content.is_empty(), "and the document is empty again");
}

/// The spawn region is one field across four columns, so it has two failure modes the other
/// attributes do not: a row where only some of them are set, and values that are not a
/// rectangle at all. Both have to be answered on read, because the engine's placement
/// arithmetic panics on a NaN range and a pack is data, not code.
#[test]
fn a_region_is_all_four_columns_or_none_of_them() {
    let mut conn = pack();
    let mut behaviour = Behaviour::new();
    behaviour.content.popups.insert(
        1,
        PopupMedia {
            // Off the screen and inside out on the way in; a rectangle on the way out.
            region: Some(SpawnRegion {
                x: 0.8,
                y: -1.0,
                width: 0.5,
                height: f64::NAN,
            }),
            ..PopupMedia::default()
        },
    );
    store(&mut conn, &behaviour);

    assert_eq!(
        read(&conn).unwrap().content.popups[&1].region,
        Some(SpawnRegion {
            x: 0.8,
            y: 0.0,
            // Shrunk against the far edge rather than slid back, so the corner the author
            // dragged to stays where they put it.
            width: 0.2,
            height: 0.0,
        })
    );

    // A half-written row -- a hand-edited pack, or one from a newer editor -- is no region
    // rather than a rectangle with invented edges. The rest of the entry survives.
    conn.execute(
        "UPDATE behaviour_popup_media SET region_height = NULL, weight = 2.0",
        [],
    )
    .unwrap();

    let stored = read(&conn).unwrap();
    assert_eq!(stored.content.popups[&1].region, None);
    assert_eq!(stored.content.popups[&1].weight, Some(2.0));
}

/// The whole reason these are rows rather than a map in the document: deleting a file takes
/// its attributes with it, with no equivalent of `clear_media_reference` to remember to call.
/// A pairing pointing at the deleted file goes too, without disturbing the popup it belonged
/// to.
#[test]
fn deleting_a_file_cascades_its_attributes_away() {
    let mut conn = pack();
    let mut behaviour = Behaviour::new();
    behaviour.content.popups.insert(
        1,
        PopupMedia {
            scale: Some(2.0),
            audio: vec![3],
            ..PopupMedia::default()
        },
    );
    behaviour
        .content
        .audio
        .insert(3, AudioMedia { volume: Some(0.5) });
    store(&mut conn, &behaviour);

    conn.execute("DELETE FROM media WHERE id = 3", []).unwrap();

    let stored = read(&conn).unwrap();
    assert!(stored.content.audio.is_empty(), "its own entry is gone");
    assert_eq!(
        stored.content.popups.get(&1).map(|popup| popup.audio.len()),
        Some(0),
        "and so is the pairing that named it",
    );
    assert_eq!(
        stored.content.popups.get(&1).and_then(|popup| popup.scale),
        Some(2.0),
        "while the popup entry itself is untouched",
    );

    conn.execute("DELETE FROM media WHERE id = 1", []).unwrap();
    assert!(read(&conn).unwrap().content.popups.is_empty());
}

/// Pairings are a set: the mode picks one at random, so order carries no meaning and the same
/// pair twice is one pair. Normalised on read so the document round-trips whatever the editor
/// sent.
#[test]
fn pairings_are_a_set_not_a_list() {
    let mut conn = pack();
    conn.execute(
        "INSERT INTO media (id, file_name, file_type, \"offset\", length, hash)
         VALUES (4, 'other.ogg', 'audio', 0, 0, x'03')",
        [],
    )
    .unwrap();

    let mut behaviour = Behaviour::new();
    behaviour.content.popups.insert(
        1,
        PopupMedia {
            audio: vec![4, 3, 4],
            ..PopupMedia::default()
        },
    );
    store(&mut conn, &behaviour);

    assert_eq!(
        read(&conn).unwrap().content.popups[&1].audio,
        vec![3, 4],
        "deduplicated and ordered, so a round-trip is stable",
    );
}

/// The diff-not-replace discipline, which is what keeps the editor's undo changesets the size
/// of the edit. Rewriting an unrelated entry must not touch this one's row.
#[test]
fn writing_one_entry_leaves_the_others_row_alone() {
    let mut conn = pack();
    let mut behaviour = Behaviour::new();
    behaviour.content.popups.insert(
        1,
        PopupMedia {
            scale: Some(2.0),
            ..PopupMedia::default()
        },
    );
    behaviour.content.popups.insert(
        2,
        PopupMedia {
            weight: Some(3.0),
            ..PopupMedia::default()
        },
    );
    store(&mut conn, &behaviour);

    let rowid_before: i64 = conn
        .query_row(
            "SELECT rowid FROM behaviour_popup_media WHERE media_id = 2",
            [],
            |row| row.get(0),
        )
        .unwrap();

    behaviour.content.popups.get_mut(&1).unwrap().scale = Some(3.0);
    store(&mut conn, &behaviour);

    let rowid_after: i64 = conn
        .query_row(
            "SELECT rowid FROM behaviour_popup_media WHERE media_id = 2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rowid_before, rowid_after);
}

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
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![unrestricted, restricted],
                transitions: vec![],
            },
            label: None,
        }),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    let stages = read(&conn).unwrap().experience.unwrap().timeline.stages;
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
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![present, absent],
                transitions: vec![],
            },
            label: None,
        }),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    let stages = read(&conn).unwrap().experience.unwrap().timeline.stages;
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
        experience: Some(Experience::default()),
        ..Behaviour::new()
    };
    store(&mut conn, &with_timeline);
    assert_eq!(read(&conn).unwrap().experience, Some(Experience::default()));

    store(&mut conn, &Behaviour::new());
    assert_eq!(read(&conn).unwrap().experience, None);
}

/// The reason writes diff instead of replacing: undo is a changeset, so a wholesale rewrite
/// would record every stage as changed on every keystroke. Editing one stage's label must
/// leave every other row exactly as it was.
#[test]
fn editing_one_stage_leaves_the_other_rows_untouched() {
    let mut conn = pack();
    let mut behaviour = Behaviour {
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![stage("a"), stage("b"), stage("c")],
                transitions: vec![],
            },
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

    behaviour.experience.as_mut().unwrap().timeline.stages[1].label = "Renamed".to_string();
    store(&mut conn, &behaviour);

    assert_eq!(before, rowids(&conn), "no stage row was replaced");
    let stages = read(&conn).unwrap().experience.unwrap().timeline.stages;
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
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![first, stage("b")],
                transitions: vec![Transition {
                    id: "t1".to_string(),
                    from_stage: "a".to_string(),
                    to_stage: "b".to_string(),
                    duration_seconds: 0.0,
                    easing: Easing::Linear,
                    affected: vec![TransitionCategory::Events],
                }],
            },
            label: None,
        }),
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    let timeline = &mut behaviour.experience.as_mut().unwrap().timeline;
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

/// What the tag joins are for. A tag list in the document is a foreign key into `tags`, so a
/// tag the author typed into a caption is a real tag the pack has -- which is what lets
/// renaming one be `UPDATE tags SET name` instead of a pass over the whole document.
#[test]
fn a_tag_named_by_the_document_becomes_a_real_tag() {
    let mut conn = pack();
    let behaviour = Behaviour {
        content: crate::behaviour::Content {
            captions: vec![TextItem {
                text: "Obey.".to_string(),
                tags: vec!["fresh".to_string()],
                timeout_seconds: None,
                summary: None,
            }],
            ..Default::default()
        },
        ..Behaviour::new()
    };
    store(&mut conn, &behaviour);

    let id: i64 = conn
        .query_row("SELECT id FROM tags WHERE name = 'fresh'", [], |row| {
            row.get(0)
        })
        .unwrap();

    // Renaming the tag row is the whole operation: the caption follows because it holds the id.
    conn.execute("UPDATE tags SET name = 'renamed' WHERE id = ?", [id])
        .unwrap();
    assert_eq!(
        read(&conn).unwrap().content.captions[0].tags,
        vec!["renamed".to_string()]
    );

    // And deleting it cascades, which is what "delete this tag" used to mean longhand.
    conn.execute("DELETE FROM tags WHERE id = ?", [id]).unwrap();
    assert!(read(&conn).unwrap().content.captions[0].tags.is_empty());
}

/// The point of the whole step: a slot is a foreign key now, so pointing one at media the pack
/// does not have is refused by the database rather than discovered later.
#[test]
fn a_slot_cannot_point_at_media_the_pack_does_not_have() {
    let mut conn = pack();
    let behaviour = Behaviour {
        content: crate::behaviour::Content {
            wallpaper: Some(404),
            ..Default::default()
        },
        ..Behaviour::new()
    };
    let tx = conn.transaction().unwrap();
    let error = write(&tx, &behaviour).unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("foreign key"),
        "unexpected error: {error}"
    );
}

/// Nothing is left in `pack_data`: the document is rows, end to end.
#[test]
fn the_document_no_longer_touches_pack_data() {
    let mut conn = pack();
    let behaviour = Behaviour {
        content: crate::behaviour::Content {
            captions: vec![TextItem {
                text: "Obey.".to_string(),
                tags: vec!["kinky".to_string()],
                timeout_seconds: None,
                summary: None,
            }],
            wallpaper: Some(1),
            ..Default::default()
        },
        experience: Some(Experience::default()),
    };
    store(&mut conn, &behaviour);

    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM pack_data", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0, "the behaviour blob is gone, not merely smaller");
    assert_eq!(read(&conn).unwrap(), behaviour);
}

/// A pack still carrying the old blob is one the tables cannot describe, so it is refused
/// rather than read as an empty document.
#[test]
fn a_pack_from_before_the_content_tables_is_refused() {
    let conn = pack();
    conn.execute(
        "INSERT INTO pack_data (name, blob) VALUES ('behaviour', ?)",
        params![br#"{"version":5,"content":{}}"#],
    )
    .unwrap();

    let error = read(&conn).unwrap_err();
    assert!(error.to_string().contains("re-import"), "{error}");
}
