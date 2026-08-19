use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

/// A tagged-only item (no empty-tag fallback in the pool) is eligible exactly when the
/// spawned media's tags intersect its own -- proving tag-matching actually gates eligibility,
/// independent of the empty-tag-always-applies case covered by the next test.
#[tokio::test(start_paused = true)]
async fn content_picker_matches_media_tags_over_empty_bucket() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                captions: vec![shared::behaviour::TextItem {
                    text: "kinky".to_string(),
                    tags: vec!["kinky".to_string()],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let sources = content_lib_sources(
                r#"
                    local content = require("lib.content")
                    for _ = 1, 20 do
                        local caption = content.pick_caption({ "kinky" })
                        assert(caption ~= nil and caption.text == "kinky",
                            "expected the tagged caption to be eligible on a matching tag")
                    end
                    assert(content.pick_caption({ "other" }) == nil,
                        "a non-matching tag should exclude the only pool entry")
                "#,
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                HashMap::new(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn content_picker_falls_back_to_empty_tag_item_when_no_match() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                captions: vec![
                    shared::behaviour::TextItem {
                        text: "untagged".to_string(),
                        tags: vec![],
                        timeout_seconds: None,
                        summary: None,
                    },
                    shared::behaviour::TextItem {
                        text: "kinky".to_string(),
                        tags: vec!["kinky".to_string()],
                        timeout_seconds: None,
                        summary: None,
                    },
                ],
                ..Default::default()
            };

            let sources = content_lib_sources(
                r#"
                    local content = require("lib.content")
                    for _ = 1, 20 do
                        local caption = content.pick_caption({ "other" })
                        assert(caption.text == "untagged", "expected the empty-tag caption to win")
                    end
                "#,
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                HashMap::new(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn content_picker_returns_nil_for_empty_pool() {
    LocalSet::new()
        .run_until(async {
            let sources = content_lib_sources(
                r#"
                    local content = require("lib.content")
                    assert(content.pick_caption({ "anything" }) == nil)
                    assert(content.pick_prompt() == nil)
                    assert(content.pick_notification() == nil)
                    assert(content.pick_web_link() == nil)
                "#,
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                Content::default(),
                HashMap::new(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn content_picker_excludes_disabled_content_group_tags() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                content_groups: vec![shared::behaviour::ContentGroup {
                    id: "kinky".to_string(),
                    label: "Kinky".to_string(),
                    description: None,
                    tags: vec!["kinky".to_string()],
                    enabled_by_default: true,
                }],
                notifications: vec![
                    shared::behaviour::TextItem {
                        text: "vanilla".to_string(),
                        tags: vec![],
                        timeout_seconds: None,
                        summary: None,
                    },
                    shared::behaviour::TextItem {
                        text: "kinky".to_string(),
                        tags: vec!["kinky".to_string()],
                        timeout_seconds: None,
                        summary: None,
                    },
                ],
                ..Default::default()
            };

            let sources = content_lib_sources(
                r#"
                    local content = require("lib.content")
                    for _ = 1, 20 do
                        local notification = content.pick_notification()
                        assert(notification.text == "vanilla",
                            "expected the kinky-tagged notification to be excluded")
                    end
                "#,
            );

            let mut mode_config = HashMap::new();
            mode_config.insert(
                format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX),
                OptionValue::Boolean(false),
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                mode_config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn content_picker_standalone_mode_ignores_own_tags() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                notifications: vec![shared::behaviour::TextItem {
                    text: "obey".to_string(),
                    tags: vec!["hypno".to_string()],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let sources = content_lib_sources(
                r#"
                    local content = require("lib.content")
                    for _ = 1, 20 do
                        local item = content.pick_notification()
                        assert(item ~= nil and item.text == "obey",
                            "a standalone pool's own tags shouldn't gate eligibility")
                    end
                "#,
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                HashMap::new(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

/// Regression test for the `merge_tags` shorthand-array bug: querying with the shorthand
/// `tags = { "wallpaper" }` form must still narrow the results even once some *other* content
/// group is disabled -- before the fix, `merge_tags` silently dropped the shorthand filter in
/// that case (`tags.any` read off a plain array is always nil), matching all media instead.
#[tokio::test(start_paused = true)]
async fn media_query_layer_honors_shorthand_tags_when_a_group_is_disabled() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[
                ("wallpaper1.avif", &["wallpaper"]),
                ("kinky1.avif", &["kinky"]),
            ];

            let content = Content {
                content_groups: vec![shared::behaviour::ContentGroup {
                    id: "kinky".to_string(),
                    label: "Kinky".to_string(),
                    description: None,
                    tags: vec!["kinky".to_string()],
                    enabled_by_default: true,
                }],
                ..Default::default()
            };

            let sources: &[(&str, &str)] = &[
                (
                    "main.lua",
                    r#"
                        local media = require("lib.media")
                        local items = media.list({ type = { "image" }, tags = { "wallpaper" } })
                        assert(#items == 1, "shorthand tag filter should still narrow the results")
                        assert(items[1].name == "wallpaper1.avif")
                    "#,
                ),
                ("lib/media.lua", LIB_MEDIA),
            ];

            let mut mode_config = HashMap::new();
            mode_config.insert(
                format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX),
                OptionValue::Boolean(false),
            );

            let mut harness = Harness::with_pack(
                sources,
                pack_fixture_with_data(MEDIA, None),
                content,
                mode_config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}
