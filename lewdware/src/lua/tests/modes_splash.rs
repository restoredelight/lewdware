use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

#[tokio::test(start_paused = true)]
async fn splash_shown_once_and_fades_out() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                splash: Some(FIXTURE_MEDIA),
                ..Default::default()
            };

            let mut config = isolated_process_config();
            config.insert("splash_enabled".to_string(), OptionValue::Boolean(true));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(true),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            assert_eq!(
                harness.recorded(),
                vec![Recorded::SpawnImage { media_id: 1 }]
            );

            // Drive the fade-in -> hold -> fade-out -> close chain: the fake handler doesn't
            // simulate FadeFinish itself (a real animation-completion event in the real app),
            // so the test supplies it directly, same as DialogSelect/DialogSubmit elsewhere.
            harness.send_event(Event::FadeFinish {
                id: ItemId(0),
                fade_id: 0,
            });
            harness.pump_events();
            harness.advance(Duration::from_millis(1600)).await; // SPLASH_HOLD_MS = 1500
            harness.send_event(Event::FadeFinish {
                id: ItemId(0),
                fade_id: 1,
            });
            harness.pump_events();

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::CloseWindow { id: ItemId(0) },
                ]
            );
        })
        .await;
}

/// The pack's wallpaper and its splash are scenery, not popup content: they were picked to sit
/// behind everything and to open the session. Letting the ordinary spawn loop draw from them
/// puts the desktop background in a popup and gives away that the splash was never special.
/// The marker the editor applies to slot media is what says so -- and it generalizes, so a
/// file an author simply doesn't want in popups is excluded by the same rule. Media rows here
/// are ids 1..=3 in fixture order, so the assertion names exactly which ones were allowed
/// through.
#[tokio::test(start_paused = true)]
async fn ordinary_popups_never_draw_non_popup_media() {
    LocalSet::new()
        .run_until(async {
            // Ids 2 and 3 in the fixture order below: `bg.avif` and `intro.avif`.
            let content = Content {
                wallpaper: Some(2),
                splash: Some(3),
                ..Default::default()
            };

            let mut config = base_default_mode_config();
            // Leave the two features themselves off: this is about the *popup* pool, and their
            // own spawns would otherwise be mixed into what we assert on.
            config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(false));
            config.insert("splash_enabled".to_string(), OptionValue::Boolean(false));
            config.insert("max_popups".to_string(), OptionValue::Null);

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture_with_data(
                    &[
                        ("ordinary.avif", &[]),
                        ("bg.avif", &[shared::tags::NON_POPUP_TAG]),
                        ("intro.avif", &[shared::tags::NON_POPUP_TAG]),
                    ],
                    None,
                ),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            // Long enough that a pool of three would almost certainly have offered up both of
            // the excluded rows at least once.
            harness.advance(Duration::from_millis(40_000)).await;

            let spawned: Vec<u64> = harness
                .recorded()
                .into_iter()
                .filter_map(|record| match record {
                    Recorded::SpawnImage { media_id } => Some(media_id),
                    _ => None,
                })
                .collect();

            assert!(
                spawned.len() > 5,
                "expected a run of popups, got {spawned:?}"
            );
            for media_id in spawned {
                assert_eq!(
                    media_id, 1,
                    "only `ordinary.avif` (id 1) should ever be spawned as a popup"
                );
            }
        })
        .await;
}

/// The splash a pack actually ships is usually animated, and animation means the pack stores a
/// *video* (`shared/src/encode.rs` probes animated gif/apng/webp as `FileInfo::Video`). Querying
/// images alone sent every one of those down rule 5's skip branch, so `splash_enabled` was shown
/// and honoured for still splashes while doing nothing at all for the common case.
#[tokio::test(start_paused = true)]
async fn an_animated_splash_is_shown_like_any_other() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                splash: Some(FIXTURE_MEDIA),
                ..Default::default()
            };

            let mut config = isolated_process_config();
            config.insert("splash_enabled".to_string(), OptionValue::Boolean(true));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture_with_tagged_video(&[]),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            assert_eq!(
                harness.recorded(),
                vec![Recorded::SpawnVideo {
                    media_id: 1,
                    volume: 1.0
                }],
                "an animated splash should spawn as a video popup"
            );

            // Same fade-in -> hold -> fade-out -> close chain as the still case: the window's
            // lifetime is the schedule's, not the video's, which is why it loops.
            harness.send_event(Event::FadeFinish {
                id: ItemId(0),
                fade_id: 0,
            });
            harness.pump_events();
            harness.advance(Duration::from_millis(1600)).await;
            harness.send_event(Event::FadeFinish {
                id: ItemId(0),
                fade_id: 1,
            });
            harness.pump_events();

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnVideo {
                        media_id: 1,
                        volume: 1.0
                    },
                    Recorded::CloseWindow { id: ItemId(0) },
                ]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn splash_disabled_never_spawns() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                splash: Some(FIXTURE_MEDIA),
                ..Default::default()
            };

            let mut config = isolated_process_config();
            config.insert("splash_enabled".to_string(), OptionValue::Boolean(false));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(true),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(harness.recorded(), vec![]);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn splash_skips_silently_when_the_reference_does_not_resolve() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                splash: Some(FIXTURE_MEDIA),
                ..Default::default()
            };

            let mut config = isolated_process_config();
            config.insert("splash_enabled".to_string(), OptionValue::Boolean(true));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(false),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(harness.recorded(), vec![]);
        })
        .await;
}
