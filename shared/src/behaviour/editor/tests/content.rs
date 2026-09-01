use super::{caption, caption_texts, notification, pack, prompt, tx_on};
use crate::behaviour::editor::{
    add_caption, add_content_group, add_notification, add_prompt, add_web_link, captions,
    content_groups, notifications, prompts, remove_caption, set_caption_text, web_links,
};
use crate::behaviour::schema::{
    Behaviour, Caption, Content, ContentGroup, Notification, Prompt, WebLink,
};

#[test]
fn a_content_entry_round_trips_with_its_id() {
    let mut conn = pack();
    let mut id = 0;
    tx_on(&mut conn, |tx| {
        id = add_caption(tx, &caption("first")).unwrap();
    });

    let rows = captions(&conn).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, id);
    assert_eq!(rows[0].text, "first");
}

/// The point of the module: an edit names the row it means, so it cannot land on a neighbour if
/// the collection has shifted under the editor since it last looked.
#[test]
fn editing_one_entry_leaves_its_neighbours_alone() {
    let mut conn = pack();
    let mut ids = vec![];
    tx_on(&mut conn, |tx| {
        for text in ["one", "two", "three"] {
            ids.push(add_caption(tx, &caption(text)).unwrap());
        }
    });

    tx_on(&mut conn, |tx| {
        set_caption_text(tx, ids[1], "edited").unwrap();
    });

    assert_eq!(caption_texts(&conn), ["one", "edited", "three"]);
}

/// Removing an entry leaves the remaining ids in their existing order, and a later append follows
/// them even though the id sequence now has a gap.
#[test]
fn removing_an_entry_preserves_order_before_the_next_append() {
    let mut conn = pack();
    let mut ids = vec![];
    tx_on(&mut conn, |tx| {
        for text in ["one", "two", "three"] {
            ids.push(add_caption(tx, &caption(text)).unwrap());
        }
        remove_caption(tx, ids[0]).unwrap();
        add_caption(tx, &caption("four")).unwrap();
    });
    assert_eq!(caption_texts(&conn), ["two", "three", "four"]);
}

#[test]
fn the_four_content_collections_do_not_see_each_other() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        add_caption(tx, &caption("a caption")).unwrap();
        add_prompt(
            tx,
            &Prompt {
                timeout_seconds: Some(12.0),
                ..prompt("a prompt")
            },
        )
        .unwrap();
        add_notification(
            tx,
            &Notification {
                summary: Some("A title".to_string()),
                ..notification("a notification")
            },
        )
        .unwrap();
        add_web_link(
            tx,
            &WebLink {
                url: "https://example.invalid".to_string(),
                tags: vec![],
            },
        )
        .unwrap();
    });
    assert_eq!(caption_texts(&conn), ["a caption"]);
    assert_eq!(prompts(&conn).unwrap()[0].text, "a prompt");
    assert_eq!(notifications(&conn).unwrap()[0].text, "a notification");
    assert_eq!(web_links(&conn).unwrap()[0].url, "https://example.invalid");
    assert_eq!(prompts(&conn).unwrap()[0].timeout_seconds, Some(12.0));
    assert_eq!(
        notifications(&conn).unwrap()[0].summary.as_deref(),
        Some("A title")
    );
}

#[test]
fn an_entrys_tags_round_trip_and_become_real_tags() {
    let mut conn = pack();
    tx_on(&mut conn, |tx| {
        add_caption(
            tx,
            &Caption {
                tags: vec!["imp".to_string(), "succubus".to_string()],
                ..caption("tagged")
            },
        )
        .unwrap();
    });
    let rows = captions(&conn).unwrap();
    assert_eq!(rows[0].tags, ["imp", "succubus"]);
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
        add_caption(tx, &caption("hello")).unwrap();
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
            captions: vec![Caption {
                text: "hello".to_string(),
                tags: vec![],
            }],
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
