use super::{pack, store};
use crate::behaviour::schema::{Behaviour, Caption};

/// What the tag joins are for. A tag list in the document is a foreign key into `tags`, so a
/// tag the author typed into a caption is a real tag the pack has -- which is what lets
/// renaming one be `UPDATE tags SET name` instead of a pass over the whole document.
#[test]
fn a_tag_named_by_the_document_becomes_a_real_tag() {
    let mut conn = pack();
    let behaviour = Behaviour {
        content: crate::behaviour::Content {
            captions: vec![Caption {
                text: "Obey.".to_string(),
                tags: vec!["fresh".to_string()],
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
        Behaviour::read(&conn).unwrap().content.captions[0].tags,
        vec!["renamed".to_string()]
    );

    // And deleting it cascades, which is what "delete this tag" used to mean longhand.
    conn.execute("DELETE FROM tags WHERE id = ?", [id]).unwrap();
    assert!(
        Behaviour::read(&conn).unwrap().content.captions[0]
            .tags
            .is_empty()
    );
}
