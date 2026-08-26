use rusqlite::Connection;

use super::write;
use crate::behaviour::schema::{Behaviour, ContentSelection, Events, Stage};

/// A migrated pack database with a few media files, so the slot and per-item foreign keys
/// have something real to point at.
fn pack() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", true).unwrap();
    crate::db::migrate(&mut conn).unwrap();
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
mod experience;
mod facade;
mod integration;
mod media;
mod tags;
