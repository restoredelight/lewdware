use super::{item, pack, texts, tx_on};
use crate::behaviour::editor::{
    PoolKind, add_content_group, add_text_item, add_web_link, content_groups, remove_text_item,
    set_text_item_text, text_pool, web_links,
};
use crate::behaviour::schema::{Behaviour, Content, ContentGroup, TextItem, WebLink};

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
        set_text_item_text(tx, ids[1], "edited").unwrap();
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
        remove_text_item(tx, ids[0]).unwrap();
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
fn a_web_link_round_trips_with_its_tags() {
    let mut conn = pack();
    let link = WebLink {
        url: "https://example.invalid".to_string(),
        tags: vec!["imp".to_string()],
    };
    tx_on(&mut conn, |tx| {
        add_web_link(tx, &link).unwrap();
    });
    let rows = web_links(&conn).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].url, link.url);
    assert_eq!(rows[0].tags, link.tags);
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

    let document = Behaviour::read(&conn).unwrap();
    assert_eq!(
        document.content,
        Content {
            captions: vec![item("hello")],
            web_links: vec![WebLink {
                url: "https://example.invalid".to_string(),
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
