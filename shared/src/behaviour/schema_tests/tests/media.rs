use super::{pack, store};
use crate::behaviour::schema::{AudioMedia, Behaviour, Content, PopupMedia, SpawnRegion};

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
    assert_eq!(Behaviour::read(&conn).unwrap(), behaviour);

    // Clearing the last attribute of each removes the entry entirely.
    behaviour.content.popups.insert(1, PopupMedia::default());
    behaviour.content.audio.insert(3, AudioMedia::default());
    store(&mut conn, &behaviour);

    let stored = Behaviour::read(&conn).unwrap();
    assert!(stored.content.popups.is_empty());
    assert!(stored.content.audio.is_empty());
    assert_eq!(
        stored.content,
        Content::default(),
        "and the document is empty again"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM behaviour_popup_media_fields",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0,
        "the SQL representation is sparse too",
    );

    assert!(
        conn.execute(
            "INSERT INTO behaviour_popup_media_fields (media_id) VALUES (1)",
            [],
        )
        .is_err(),
        "the backing table itself rejects an empty row",
    );
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
        Behaviour::read(&conn).unwrap().content.popups[&1].region,
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

    let stored = Behaviour::read(&conn).unwrap();
    assert_eq!(stored.content.popups[&1].region, None);
    assert_eq!(stored.content.popups[&1].weight, Some(2.0));
}

/// The whole reason these are rows rather than a map in the document: deleting a file takes
/// its attributes with it, with no explicit cleanup query to remember to call.
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

    let stored = Behaviour::read(&conn).unwrap();
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
    assert!(Behaviour::read(&conn).unwrap().content.popups.is_empty());
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
        Behaviour::read(&conn).unwrap().content.popups[&1].audio,
        vec![3, 4],
        "deduplicated and ordered, so a round-trip is stable",
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM behaviour_popup_media_fields",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0,
        "pairings do not need an all-NULL scalar row",
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
            "SELECT rowid FROM behaviour_popup_media_fields WHERE media_id = 2",
            [],
            |row| row.get(0),
        )
        .unwrap();

    behaviour.content.popups.get_mut(&1).unwrap().scale = Some(3.0);
    store(&mut conn, &behaviour);

    let rowid_after: i64 = conn
        .query_row(
            "SELECT rowid FROM behaviour_popup_media_fields WHERE media_id = 2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rowid_before, rowid_after);
}
