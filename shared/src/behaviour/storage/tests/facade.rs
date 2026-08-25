use super::*;

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
