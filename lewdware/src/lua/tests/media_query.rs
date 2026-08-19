use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

/// Integration-style: exercises the *shipped* `default-modes/src/lib/media.lua` file (not a
/// re-implementation of its logic), confirming a disabled content group's tagged media never
/// surfaces from `media.list()` -- the "no query path bypasses it" gate from the release
/// plan's M3 content-groups bullet.
#[tokio::test(start_paused = true)]
async fn shared_media_query_layer_honors_disabled_content_groups() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[
                ("kinky1.avif", &["kinky"]),
                ("kinky2.avif", &["kinky"]),
                ("vanilla1.avif", &[]),
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
                        local items = media.list({ type = { "image" } })

                        local seen = {}
                        for _, item in ipairs(items) do seen[item.name] = true end

                        assert(seen["vanilla1.avif"], "untagged media should always be present")
                        if lewdware.config["content_group.kinky"] then
                            assert(seen["kinky1.avif"], "kinky1 should be present when enabled")
                            assert(seen["kinky2.avif"], "kinky2 should be present when enabled")
                        else
                            assert(not seen["kinky1.avif"], "kinky1 should be excluded when disabled")
                            assert(not seen["kinky2.avif"], "kinky2 should be excluded when disabled")
                        end
                    "#,
                ),
                (
                    "lib/media.lua",
                    LIB_MEDIA,
                ),
            ];

            let key = format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX);

            // Group enabled: all three items present.
            let mut enabled_config = HashMap::new();
            enabled_config.insert(key.clone(), OptionValue::Boolean(true));
            let mut harness = Harness::with_pack(
                sources,
                pack_fixture_with_data(MEDIA, None),
                content.clone(),
                enabled_config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            // Group disabled: the kinky-tagged media never surfaces.
            let mut disabled_config = HashMap::new();
            disabled_config.insert(key, OptionValue::Boolean(false));
            let mut harness = Harness::with_pack(
                sources,
                pack_fixture_with_data(MEDIA, None),
                content,
                disabled_config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn shared_media_query_layer_separates_background_and_popup_audio() {
    LocalSet::new()
        .run_until(async {
            let sources: &[(&str, &str)] = &[
                (
                    "main.lua",
                    r#"
							local media = require("lib.media")
							local background = media.random_background_audio()
							assert(background and background.name == "music.ogg")
							-- An item, not a bare tag list: the picker reads `id` for explicit
							-- pairings and falls back to `tags`. Id 0 matches no file, so this
							-- exercises the tag path.
							local effect = media.random_popup_audio({ id = 0, tags = { "kinky" } })
							assert(effect and effect.name == "effect.ogg")
							assert(media.random_popup_audio({ id = 0, tags = { "vanilla" } }) == nil)
						"#,
                ),
                ("lib/media.lua", LIB_MEDIA),
            ];
            let media: &[(&str, &[&str])] = &[
                ("music.ogg", &[]),
                ("effect.ogg", &[shared::tags::POPUP_AUDIO_TAG, "kinky"]),
            ];
            let mut harness = Harness::with_pack(
                sources,
                pack_fixture_with_data(media, None),
                Content::default(),
                HashMap::new(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}

/// The popup-audio pool is asked for once and indexed, not re-listed and re-derived on every
/// spawn -- nothing it is built from can change while a session runs, and this is the spawn
/// path. The counting wrapper stands in for what the engine would otherwise be asked to do.
#[tokio::test(start_paused = true)]
async fn popup_audio_pool_is_listed_once_and_shared_by_every_spawn() {
    LocalSet::new()
        .run_until(async {
            let sources: &[(&str, &str)] = &[
                (
                    "main.lua",
                    r#"
							local media = require("lib.media")
							local queries = 0
							local list = lewdware.media.list
							lewdware.media.list = function(opts)
								queries = queries + 1
								return list(opts)
							end

							-- Audio with no tags of its own suits any popup, including one with no tags;
							-- audio that names tags suits only a popup sharing one.
							assert(media.random_popup_audio({ id = 0, tags = { "vanilla" } }).name == "ambient.ogg")
							assert(media.random_popup_audio({ id = 0, tags = {} }).name == "ambient.ogg")
							local shared_tag = media.random_popup_audio({ id = 0, tags = { "kinky" } }).name
							assert(shared_tag == "effect.ogg" or shared_tag == "ambient.ogg")

							-- Still one: the by-id index the explicit pairings need is only built
							-- when a popup actually has one, so a pack without pairings pays for
							-- nothing.
							assert(queries == 1, "expected one pool query, got " .. queries)
						"#,
                ),
                ("lib/media.lua", LIB_MEDIA),
            ];
            let media: &[(&str, &[&str])] = &[
                ("music.ogg", &[]),
                ("ambient.ogg", &[shared::tags::POPUP_AUDIO_TAG]),
                ("effect.ogg", &[shared::tags::POPUP_AUDIO_TAG, "kinky"]),
            ];
            let mut harness = Harness::with_pack(
                sources,
                pack_fixture_with_data(media, None),
                Content::default(),
                HashMap::new(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
        })
        .await;
}
