use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

/// Records, at the moment each spawn decision is made, whether it violated the timeline's
/// *currently active* tag restriction -- rather than comparing spawn counts across a
/// before/after time window (which races against exactly which spawn decisions happen to fall
/// in the gap around the level's boundary instant, per rule 2). Checking the invariant at each
/// decision point directly is deterministic regardless of that timing.
const SPAWN_NAME_CAPTURE_WRAP: &str = r#"
    KINKY_COUNT = 0
    VANILLA_WHILE_RESTRICTED = 0
    local real = lewdware.popup.image
    lewdware.popup.image = function(item, opts)
        -- `tags()` is a filter (`{ any = ..., none = ... }`), never the plain-array shorthand:
        -- a stage both includes and excludes. Reading `tags[1]` here would be nil forever, and
        -- this assertion would pass without ever testing anything.
        local tags = require("timeline").tags()
        local any = tags and tags.any
        if item.name == "kinky.avif" then KINKY_COUNT = KINKY_COUNT + 1 end
        if item.name == "vanilla.avif" and any and any[1] == "kinky" then
            VANILLA_WHILE_RESTRICTED = VANILLA_WHILE_RESTRICTED + 1
        end
        return real(item, opts)
    end
"#;

/// A level's `tags` restricts which media can spawn (the active tag set, an `any`-style
/// restriction -- see `Modifiers::tags`'s doc comment): once the timeline's active tag set is
/// `{"kinky"}`, `vanilla.avif` (tagged only `"vanilla"`) must never spawn -- checked
/// deterministically at every spawn decision (the engine's tag filter is a SQL
/// `EXISTS`/`NOT EXISTS` exclusion, not a random sample), not via a racy spawn-count window.
#[tokio::test(start_paused = true)]
async fn experience_timeline_tags_narrow_which_media_can_spawn() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] =
                &[("kinky.avif", &["kinky"]), ("vanilla.avif", &["vanilla"])];

            let experience = levels_to_experience(vec![
                Level {
                    anchors: FrequencyAnchors {
                        popup: Some(0.05),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Level {
                    at_seconds: 0.5,
                    anchors: FrequencyAnchors {
                        popup: Some(0.05),
                        ..Default::default()
                    },
                    tags: Some(vec!["kinky".to_string()]),
                    ..Default::default()
                },
            ]);

            let mut config = base_experience_mode_config();
            config.insert("max_popups".to_string(), OptionValue::Integer(1_000));
            config.insert("captions_enabled".to_string(), OptionValue::Boolean(false));

            let owned = wrapped_experience_mode_sources(SPAWN_NAME_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut harness = Harness::with_pack_and_experience(
                &sources,
                pack_fixture_with_data(MEDIA, None),
                Content::default(),
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2900)).await;

            assert_eq!(
                harness.eval_number("VANILLA_WHILE_RESTRICTED"),
                0.0,
                "vanilla.avif must never spawn while the timeline's active tag set is \
                 restricted to kinky"
            );
            assert!(
                harness.eval_number("KINKY_COUNT") > 10.0,
                "kinky spawns should still be happening at the full baseline rate"
            );
        })
        .await;
}

/// `ContentSelection::tags` has three states, and the third one used to be lost in transit.
/// `None` is "no restriction"; `Some([])` is "deliberately no content" -- the distinction the
/// pack database keeps a `restricts_content` column for. The engine documents empty tag lists
/// as imposing no constraint (right for a query API), so an empty set arrived as `{ any = {} }`
/// and the clause was skipped: a stage selecting *no* content spawned from the *whole* pack.
/// `lib/media.lua` now absorbs the mismatch, which is what that layer exists for.
#[tokio::test(start_paused = true)]
async fn experience_stage_selecting_no_content_spawns_nothing() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[("kinky.avif", &["kinky"]), ("untagged.avif", &[])];

            let experience = levels_to_experience(vec![Level {
                anchors: FrequencyAnchors {
                    popup: Some(0.05),
                    ..Default::default()
                },
                // Deliberately no content -- not "no restriction", which is `None`.
                tags: Some(vec![]),
                ..Default::default()
            }]);

            let mut config = base_experience_mode_config();
            config.insert("max_popups".to_string(), OptionValue::Integer(1_000));

            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture_with_data(MEDIA, None),
                Content::default(),
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2900)).await;

            let spawns = harness
                .recorded()
                .iter()
                .filter(|recorded| matches!(recorded, Recorded::SpawnImage { .. }))
                .count();
            assert_eq!(
                spawns, 0,
                "a stage that selects no content must spawn nothing -- untagged media \
                 included, since it is the pack-wide pool that was leaking through"
            );
        })
        .await;
}

/// The other half: `None` must keep meaning "no restriction". A fix that made an absent tag
/// set select nothing would be the same bug with the sign flipped.
#[tokio::test(start_paused = true)]
async fn experience_stage_with_no_tag_restriction_still_spawns_everything() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[("kinky.avif", &["kinky"]), ("untagged.avif", &[])];

            let experience = levels_to_experience(vec![Level {
                anchors: FrequencyAnchors {
                    popup: Some(0.05),
                    ..Default::default()
                },
                tags: None,
                ..Default::default()
            }]);

            let mut config = base_experience_mode_config();
            config.insert("max_popups".to_string(), OptionValue::Integer(1_000));

            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture_with_data(MEDIA, None),
                Content::default(),
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2900)).await;

            let spawns = harness
                .recorded()
                .iter()
                .filter(|recorded| matches!(recorded, Recorded::SpawnImage { .. }))
                .count();
            assert!(
                spawns > 10,
                "an unrestricted stage should spawn at the full rate, got {spawns}"
            );
        })
        .await;
}

/// The same distinction on the audio path, which reaches it by a different route (the
/// stage-tag narrowing has a fallback to the whole background pool, and "no content" must not
/// be mistaken for "nothing matched").
#[tokio::test(start_paused = true)]
async fn experience_stage_selecting_no_content_is_silent() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[("music.ogg", &[])];

            let experience = levels_to_experience(vec![Level {
                tags: Some(vec![]),
                ..Default::default()
            }]);

            let mut config = isolated_experience_process_config();
            config.insert(
                "background_audio_enabled".to_string(),
                OptionValue::Boolean(true),
            );

            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture_with_data(MEDIA, None),
                Content::default(),
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(
                harness.recorded(),
                vec![],
                "the stage selects no content, so the fallback to the whole background pool \
                 must not fire"
            );
        })
        .await;
}

/// A stage's exclusion list keeps media out however else it qualified.
///
/// The point of the feature, and the reason it is a *tag* rather than a list of ids: the editor's
/// "Appears in" toggle takes one file out of one stage by tagging it, without touching the author's
/// own vocabulary. So `kinky.avif` still carries `kinky`, the stage still selects by `kinky`, and
/// the file still must not appear — checked at every spawn decision rather than over a time window,
/// since a single leaked spawn is the whole bug.
#[tokio::test(start_paused = true)]
async fn experience_stage_exclusions_keep_media_out_of_a_stage_that_includes_it() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[
                ("kept.avif", &["kinky"]),
                ("removed.avif", &["kinky", "not-stage-1"]),
            ];

            let experience = levels_to_experience(vec![Level {
                anchors: FrequencyAnchors {
                    popup: Some(0.05),
                    ..Default::default()
                },
                tags: Some(vec!["kinky".to_string()]),
                exclude: vec!["not-stage-1".to_string()],
                ..Default::default()
            }]);

            let mut config = base_experience_mode_config();
            config.insert("max_popups".to_string(), OptionValue::Integer(1_000));
            config.insert("captions_enabled".to_string(), OptionValue::Boolean(false));

            let owned = wrapped_experience_mode_sources(
                r#"
                KEPT = 0
                REMOVED = 0
                local real = lewdware.popup.image
                lewdware.popup.image = function(item, opts)
                    if item.name == "kept.avif" then KEPT = KEPT + 1 end
                    if item.name == "removed.avif" then REMOVED = REMOVED + 1 end
                    return real(item, opts)
                end
            "#,
            );
            let sources = as_str_sources(&owned);

            let mut harness = Harness::with_pack_and_experience(
                &sources,
                pack_fixture_with_data(MEDIA, None),
                Content::default(),
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2900)).await;

            assert_eq!(
                harness.eval_number("REMOVED"),
                0.0,
                "removed.avif carries the stage's exclusion tag and must never spawn in it, \
                 even though it also carries the tag the stage selects by"
            );
            assert!(
                harness.eval_number("KEPT") > 10.0,
                "the rest of the stage's media should spawn at the full baseline rate"
            );
        })
        .await;
}

/// Exclusion is the only thing that can narrow a stage which restricts nothing — there is no
/// inclusion list for a file to fall out of. This is what makes "Appears in" work on every stage.
#[tokio::test(start_paused = true)]
async fn experience_stage_exclusions_work_without_any_inclusion_list() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] =
                &[("kept.avif", &[]), ("removed.avif", &["not-stage-1"])];

            let experience = levels_to_experience(vec![Level {
                anchors: FrequencyAnchors {
                    popup: Some(0.05),
                    ..Default::default()
                },
                // No `tags` at all: the stage shows the whole pack apart from what it excludes.
                exclude: vec!["not-stage-1".to_string()],
                ..Default::default()
            }]);

            let mut config = base_experience_mode_config();
            config.insert("max_popups".to_string(), OptionValue::Integer(1_000));
            config.insert("captions_enabled".to_string(), OptionValue::Boolean(false));

            let owned = wrapped_experience_mode_sources(
                r#"
                KEPT = 0
                REMOVED = 0
                local real = lewdware.popup.image
                lewdware.popup.image = function(item, opts)
                    if item.name == "kept.avif" then KEPT = KEPT + 1 end
                    if item.name == "removed.avif" then REMOVED = REMOVED + 1 end
                    return real(item, opts)
                end
            "#,
            );
            let sources = as_str_sources(&owned);

            let mut harness = Harness::with_pack_and_experience(
                &sources,
                pack_fixture_with_data(MEDIA, None),
                Content::default(),
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2900)).await;

            assert_eq!(harness.eval_number("REMOVED"), 0.0);
            assert!(harness.eval_number("KEPT") > 10.0);
        })
        .await;
}
