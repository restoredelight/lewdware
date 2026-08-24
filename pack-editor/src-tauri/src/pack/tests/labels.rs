//! Tags and artists, and how renaming or merging one carries through the document.

use shared::behaviour::{
    Behaviour, ContentGroup, ContentSelection, Events, Experience, Stage, TextItem, Timeline,
    WebLink,
};
use tempfile::tempdir;

use super::*;

#[tokio::test]
async fn add_tags_creates_new_tags_and_associates_them() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");

    let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
    let file_id = insert_staged_audio(&pack, b"audio").await;

    pack.add_tags(file_id, vec!["kinky".to_string(), "hypno".to_string()])
        .await
        .unwrap();

    let mut tags = pack.get_tags(file_id).await.unwrap();
    tags.sort();
    assert_eq!(tags, vec!["hypno".to_string(), "kinky".to_string()]);

    let mut all_tags = pack.get_all_tags().await.unwrap();
    all_tags.sort();
    assert_eq!(all_tags, vec!["hypno".to_string(), "kinky".to_string()]);

    // Re-adding an already-associated tag (e.g. a second imported file sharing a mood) is a
    // no-op, not a constraint-violation error.
    pack.add_tags(file_id, vec!["kinky".to_string()])
        .await
        .unwrap();
    let mut tags = pack.get_tags(file_id).await.unwrap();
    tags.sort();
    assert_eq!(tags, vec!["hypno".to_string(), "kinky".to_string()]);
}

#[tokio::test]
async fn bulk_tag_edit_updates_every_selected_file() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
    let first = insert_staged_audio(&pack, b"first").await;
    let second = insert_staged_audio(&pack, b"second").await;

    pack.add_tag_to_files(vec![first, second], "shared".to_string())
        .await
        .unwrap();
    assert_eq!(pack.get_tags(first).await.unwrap(), vec!["shared"]);
    assert_eq!(pack.get_tags(second).await.unwrap(), vec!["shared"]);

    pack.remove_tag_from_files(vec![first, second], "shared".to_string())
        .await
        .unwrap();
    assert!(pack.get_tags(first).await.unwrap().is_empty());
    assert!(pack.get_tags(second).await.unwrap().is_empty());
}

/// The reserved namespace is an invariant of the *writes*, not of the UI: every command an
/// author can reach refuses it, so a managed tag can only ever have been put there by the
/// editor itself. `add_tags` is deliberately absent -- that's the path the editor uses.
#[tokio::test]
async fn managed_tags_are_rejected_on_every_author_reachable_command() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(
        &tmp.path().join("reserved.lwpack"),
        data_dir.path(),
        "Reserved",
    )
    .await;
    let media = insert_staged_audio(&pack, b"scenery").await;
    pack.add_tags(media, vec![tags::NON_POPUP_TAG.to_string()])
        .await
        .unwrap();
    pack.add_tags(media, vec!["ordinary".to_string()])
        .await
        .unwrap();

    let reserved = tags::NON_POPUP_TAG.to_string();
    assert!(pack.add_tag(media, reserved.clone()).await.is_err());
    assert!(pack
        .create_and_add_tag(media, "__lewdware-mine".to_string())
        .await
        .is_err());
    assert!(pack
        .add_tag_to_files(vec![media], reserved.clone())
        .await
        .is_err());
    assert!(pack
        .rename_tag(reserved.clone(), "scenery".to_string())
        .await
        .is_err());
    assert!(pack
        .rename_tag("ordinary".to_string(), reserved.clone())
        .await
        .is_err());
    assert!(pack
        .merge_tag("ordinary".to_string(), reserved.clone())
        .await
        .is_err());

    // ...and it stays invisible to everything that offers or lists a tag.
    assert_eq!(pack.get_all_tags().await.unwrap(), vec!["ordinary"]);
    assert_eq!(
        pack.get_tag_summaries()
            .await
            .unwrap()
            .into_iter()
            .map(|summary| summary.name)
            .collect::<Vec<_>>(),
        vec!["ordinary"]
    );
    // The file itself still carries it -- hidden from the author, not from the engine.
    let mut media_tags = pack.get_tags(media).await.unwrap();
    media_tags.sort();
    assert_eq!(media_tags, vec![tags::NON_POPUP_TAG, "ordinary"]);
}

/// Merging a tag into one an entry already carries must leave one mention, not two. The old
/// document rewrite deduplicated by hand with a `HashSet`; the join table's primary key does
/// it now, and `UPDATE OR IGNORE` is what defers to it.
#[tokio::test]
async fn merging_a_tag_into_one_an_entry_already_has_leaves_a_single_mention() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("merge.lwpack"), data_dir.path(), "Merge").await;

    let behaviour = Behaviour {
        content: shared::behaviour::Content {
            captions: vec![TextItem {
                text: "Obey.".to_string(),
                tags: vec!["source".to_string(), "target".to_string()],
                timeout_seconds: None,
                summary: None,
            }],
            ..Default::default()
        },
        ..Behaviour::new()
    };
    pack.replace_behaviour(behaviour, "Seed behaviour".to_string())
        .await
        .unwrap();

    pack.merge_tag("source".to_string(), "target".to_string())
        .await
        .unwrap();
    let after = read_pack_behaviour(&pack).await;

    assert_eq!(after.content.captions[0].tags, vec!["target".to_string()]);
}

/// Renaming a tag is now `UPDATE tags SET name` and nothing else: the document holds tag ids,
/// so every mention follows without being rewritten. Reading it back afterwards is what the test
/// is really pinning down -- nothing recomputed the document, and every mention still moved.
#[tokio::test]
async fn renaming_a_tag_carries_every_mention_in_the_document() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("rename.lwpack"), data_dir.path(), "Rename").await;

    let behaviour = Behaviour {
        content: shared::behaviour::Content {
            content_groups: vec![ContentGroup {
                id: "g".to_string(),
                label: "Group".to_string(),
                description: None,
                tags: vec!["old".to_string()],
                enabled_by_default: true,
            }],
            captions: vec![TextItem {
                text: "Obey.".to_string(),
                tags: vec!["old".to_string()],
                timeout_seconds: None,
                summary: None,
            }],
            web_links: vec![WebLink {
                url: "https://example.com".to_string(),
                args: vec![],
                tags: vec!["old".to_string()],
            }],
            ..Default::default()
        },
        experience: Some(Experience {
            timeline: Timeline {
                stages: vec![Stage {
                    id: "stage-1".to_string(),
                    label: "Stage 1".to_string(),
                    end: None,
                    content: ContentSelection {
                        tags: Some(vec!["old".to_string()]),
                        owned_tag: None,
                        wallpaper: None,
                        audio: None,
                        audio_random: false,
                    },
                    events: Events::default(),
                    movement: None,
                    mitosis: None,
                    on_enter: Default::default(),
                    prompt: Default::default(),
                }],
                transitions: vec![],
            },
            label: None,
        }),
    };
    pack.replace_behaviour(behaviour, "Seed behaviour".to_string())
        .await
        .unwrap();

    pack.rename_tag("old".to_string(), "new".to_string())
        .await
        .unwrap();
    let after = read_pack_behaviour(&pack).await;

    let expected = vec!["new".to_string()];
    assert_eq!(after.content.content_groups[0].tags, expected);
    assert_eq!(after.content.captions[0].tags, expected);
    assert_eq!(after.content.web_links[0].tags, expected);
    assert_eq!(
        after.experience.unwrap().timeline.stages[0].content.tags,
        Some(expected)
    );
}

#[tokio::test]
async fn tag_management_updates_associations_and_behaviour_together() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack_path = tmp.path().join("test.lwpack");
    let pack = new_test_pack(&pack_path, data_dir.path(), "Test").await;
    let media = insert_staged_audio(&pack, b"tagged").await;
    pack.add_tags(media, vec!["old".to_string()]).await.unwrap();

    let mut behaviour = Behaviour::default();
    behaviour.content.captions = vec![TextItem {
        text: "Obey.".to_string(),
        tags: vec!["old".to_string()],
        timeout_seconds: None,
        summary: None,
    }];
    pack.replace_behaviour(behaviour.clone(), "Seed behaviour".to_string())
        .await
        .unwrap();
    pack.rename_tag("old".to_string(), "new".to_string())
        .await
        .unwrap();
    let behaviour = read_pack_behaviour(&pack).await;

    assert_eq!(pack.get_tags(media).await.unwrap(), vec!["new"]);
    assert_eq!(behaviour.content.captions[0].tags, vec!["new"]);

    pack.delete_tag("new".to_string()).await.unwrap();
    let behaviour = read_pack_behaviour(&pack).await;
    assert!(behaviour.content.captions[0].tags.is_empty());
    assert!(pack.get_tags(media).await.unwrap().is_empty());
    assert!(pack.get_tag_summaries().await.unwrap().is_empty());
}

#[tokio::test]
async fn undoing_a_merge_recovers_source_target_and_dual_associations_atomically() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("tags.lwpack"), data_dir.path(), "Tags").await;
    let source_only = insert_staged_audio(&pack, b"source").await;
    let target_only = insert_staged_audio(&pack, b"target").await;
    let both = insert_staged_audio(&pack, b"both").await;
    pack.add_tags(source_only, vec!["source".into()])
        .await
        .unwrap();
    pack.add_tags(target_only, vec!["target".into()])
        .await
        .unwrap();
    pack.add_tags(both, vec!["source".into(), "target".into()])
        .await
        .unwrap();
    let mut before = Behaviour::default();
    before.content.captions = vec![TextItem {
        text: "Obey.".to_string(),
        tags: vec!["source".into()],
        timeout_seconds: None,
        summary: None,
    }];
    pack.replace_behaviour(before.clone(), "Seed behaviour".to_string())
        .await
        .unwrap();

    pack.merge_tag("source".into(), "target".into())
        .await
        .unwrap();
    let retained_media: u64 = pack
        .db_execute(move |connection| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM history_media_refs
                     WHERE history_id = (SELECT id FROM history_entries
                                         WHERE label = 'Merge tags'
                                         ORDER BY sequence DESC LIMIT 1)",
                    [],
                    |row| row.get(0),
                )
                .map_err(Into::into)
        })
        .await
        .unwrap();
    assert_eq!(retained_media, 2);
    pack.undo().await.unwrap();

    assert_eq!(pack.get_tags(source_only).await.unwrap(), vec!["source"]);
    assert_eq!(pack.get_tags(target_only).await.unwrap(), vec!["target"]);
    let mut both_tags = pack.get_tags(both).await.unwrap();
    both_tags.sort();
    assert_eq!(both_tags, vec!["source", "target"]);
    let stored = pack.get_behaviour().await.unwrap();
    assert_eq!(stored.content.captions[0].tags, vec!["source"]);
}

#[tokio::test]
async fn undoing_a_deleted_tag_recovers_associations_and_behaviour() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(
        &tmp.path().join("delete-tag.lwpack"),
        data_dir.path(),
        "Tags",
    )
    .await;
    let media = insert_staged_audio(&pack, b"tagged").await;
    pack.add_tags(media, vec!["restore-me".into()])
        .await
        .unwrap();
    let mut before = Behaviour::default();
    before.content.captions = vec![TextItem {
        text: "Obey.".to_string(),
        tags: vec!["restore-me".into()],
        timeout_seconds: None,
        summary: None,
    }];
    pack.replace_behaviour(before.clone(), "Seed behaviour".to_string())
        .await
        .unwrap();
    pack.delete_tag("restore-me".into()).await.unwrap();
    pack.undo().await.unwrap();
    assert_eq!(pack.get_tags(media).await.unwrap(), vec!["restore-me"]);
    let stored = pack.get_behaviour().await.unwrap();
    assert_eq!(stored.content.captions[0].tags, vec!["restore-me"]);
}

/// The Tags tab reads this, and it is the only thing it reads: the table used to be a three-way
/// merge in the browser over the media list, a summary, and the behaviour document. A tag named
/// only by a caption is a real row in the pack with no media on it, and it has to be listed.
#[tokio::test]
async fn tag_rows_count_media_and_both_halves_of_the_document() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("rows.lwpack"), data_dir.path(), "Rows").await;
    let media = insert_staged_audio(&pack, b"tagged").await;
    pack.create_and_add_tag(media, "shared".to_string())
        .await
        .unwrap();

    let behaviour = Behaviour {
        content: shared::behaviour::Content {
            captions: vec![TextItem {
                text: "Obey.".to_string(),
                tags: vec!["shared".to_string(), "caption-only".to_string()],
                timeout_seconds: None,
                summary: None,
            }],
            ..Default::default()
        },
        experience: Some(shared::behaviour::Experience {
            timeline: Timeline {
                stages: vec![Stage {
                    id: "s".to_string(),
                    label: "Stage".to_string(),
                    end: None,
                    content: shared::behaviour::ContentSelection {
                        tags: Some(vec!["shared".to_string()]),
                        ..Default::default()
                    },
                    events: Default::default(),
                    movement: None,
                    mitosis: None,
                    on_enter: Default::default(),
                    prompt: Default::default(),
                }],
                transitions: vec![],
            },
            label: None,
        }),
    };
    pack.replace_behaviour(behaviour, "Seed behaviour".to_string())
        .await
        .unwrap();

    let rows = pack.get_tag_rows().await.unwrap();
    let shared = rows.iter().find(|row| row.name == "shared").unwrap();
    assert_eq!(shared.media_count, 1);
    assert_eq!(shared.content_uses, 1, "the caption");
    assert_eq!(shared.experience_uses, 1, "the stage's selection");

    // Named by a caption and nothing else -- the row the old three-way merge existed to include.
    let caption_only = rows.iter().find(|row| row.name == "caption-only").unwrap();
    assert_eq!(caption_only.media_count, 0);
    assert_eq!(caption_only.content_uses, 1);
}

/// What the Tags tab shows after a rename. The document holds tag *ids*, so the row moves without
/// anything rewriting it -- and the tab has to see the new name when it looks again.
#[tokio::test]
async fn tag_rows_show_the_new_name_after_a_rename() {
    let tmp = tempdir().unwrap();
    let data_dir = tempdir().unwrap();
    let pack = new_test_pack(&tmp.path().join("rowrename.lwpack"), data_dir.path(), "Rows").await;
    let media = insert_staged_audio(&pack, b"tagged").await;
    pack.create_and_add_tag(media, "old".to_string())
        .await
        .unwrap();

    pack.rename_tag("old".to_string(), "new".to_string())
        .await
        .unwrap();

    let rows = pack.get_tag_rows().await.unwrap();
    assert!(
        rows.iter().all(|row| row.name != "old"),
        "the old name is gone: {:?}",
        rows.iter().map(|row| &row.name).collect::<Vec<_>>()
    );
    let renamed = rows.iter().find(|row| row.name == "new").unwrap();
    assert_eq!(renamed.media_count, 1);
}
