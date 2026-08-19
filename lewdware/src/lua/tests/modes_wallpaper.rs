use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

#[tokio::test(start_paused = true)]
async fn wallpaper_applied_once_at_mode_start() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                wallpaper: Some(FIXTURE_MEDIA),
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(true),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(harness.eval_number("SET_COUNT"), 1.0);
            assert_eq!(harness.eval_string("SET_IMAGE_NAME"), "pic.avif");
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn wallpaper_skips_silently_when_the_reference_does_not_resolve() {
    // Rule 5: a slot naming a file the pack no longer has is a skip, not an error.
    LocalSet::new()
        .run_until(async {
            let content = Content {
                wallpaper: Some(FIXTURE_MEDIA),
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(harness.eval_number("SET_COUNT"), 0.0);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn wallpaper_not_applied_when_no_slot_is_set() {
    // Opt-in only: an unset `wallpaper` (the pack never filled the slot) means no wallpaper
    // feature at all, even if `wallpaper_enabled` is somehow true (defensive against a custom
    // mode reusing this library, or a stale stored option) and the fixture has real media.
    LocalSet::new()
        .run_until(async {
            let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(true),
                Content::default(),
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(harness.eval_number("SET_COUNT"), 0.0);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn wallpaper_skips_silently_when_the_reference_is_not_an_image() {
    // `lewdware.wallpaper.set` takes an image. A slot pointing at a video takes rule 5's skip
    // branch rather than erroring -- the same outcome `pack_has_wallpaper` reports up front,
    // so the toggle is never offered for this pack in the first place.
    LocalSet::new()
        .run_until(async {
            let content = Content {
                wallpaper: Some(FIXTURE_MEDIA),
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture_with_tagged_video(&[]),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(harness.eval_number("SET_COUNT"), 0.0);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn wallpaper_resets_on_dormancy_sleep_and_reapplies_on_wake() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                wallpaper: Some(FIXTURE_MEDIA),
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));
            // Whole-second bounds: schedule_dormancy passes these straight to
            // math.random(m, n), which requires integer-valued arguments (see
            // notifications_pause_while_dormant). 1s active, 1s dormant: reset should happen
            // ~1000ms in, reapply ~2000ms in.
            config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(true));
            config.insert("active_min".to_string(), OptionValue::Number(1.0));
            config.insert("active_max".to_string(), OptionValue::Number(1.0));
            config.insert("dormant_min".to_string(), OptionValue::Number(1.0));
            config.insert("dormant_max".to_string(), OptionValue::Number(1.0));

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(true),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            assert_eq!(harness.eval_number("SET_COUNT"), 1.0);

            harness.advance(Duration::from_millis(1100)).await;
            assert_eq!(harness.eval_number("RESET_COUNT"), 1.0);
            assert_eq!(
                harness.eval_number("SET_COUNT"),
                1.0,
                "shouldn't reapply until wake"
            );

            harness.advance(Duration::from_millis(1100)).await;
            assert_eq!(harness.eval_number("SET_COUNT"), 2.0);
        })
        .await;
}
