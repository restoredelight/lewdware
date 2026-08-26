use rusqlite::{Connection, Transaction};

use super::{
    PoolKind, add_stage, popup_attributes, remove_stage, stages, text_pool, transitions,
};
use crate::behaviour::schema::{
    Behaviour, ContentSelection, Events, Experience, PopupMedia, Stage, TextItem, Timeline,
};
use crate::behaviour::storage;

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

// Shared timeline fixtures used by several editor domains.
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

// Shared media-attribute fixture.
fn popups_of(conn: &Connection, id: u64) -> Option<PopupMedia> {
    popup_attributes(conn, &[id])
        .unwrap()
        .into_iter()
        .next()
        .map(|(_, entry)| entry)
}

mod collections;
mod content;
mod fields;
mod media;
mod membership;
mod timeline;
