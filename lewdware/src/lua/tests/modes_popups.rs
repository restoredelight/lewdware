use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

#[tokio::test(start_paused = true)]
async fn open_popup_sets_title_from_matching_caption() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                captions: vec![shared::behaviour::TextItem {
                    text: "Obey.".to_string(),
                    tags: vec!["red".to_string()],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(true),
                content,
                base_default_mode_config(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            // Slack beyond the nominal 1000ms popup_frequency: the fake handler's reply is a
            // real OS-thread round trip, which eats into the paused clock's margin (see
            // `interval_keeps_firing_after_a_callback_errors`'s comment for the same reasoning).
            harness.advance(Duration::from_millis(1100)).await;

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::SetTitle {
                        id: ItemId(0),
                        title: Some("Obey.".to_string()),
                    },
                ]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn popup_audio_plays_when_a_popup_spawns() {
    LocalSet::new()
        .run_until(async {
            let mut config = base_default_mode_config();
            config.insert(
                "popup_audio_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            let media: &[(&str, &[&str])] = &[
                ("popup.avif", &[]),
                ("effect.ogg", &[shared::tags::POPUP_AUDIO_TAG]),
            ];
            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture_with_data(media, None),
                Content::default(),
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;
            harness.send_event(Event::WindowSpawned { id: ItemId(0) });
            harness.pump_events();

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::SpawnAudio {
                        media_id: 2,
                        volume: 1.0,
                    },
                ]
            );
        })
        .await;
}

/// One popup sound at a time: popups arrive faster than their sounds finish, and layering
/// one-shots stops them reading as individual sounds. The one playing keeps the slot, and the
/// popups spawned meanwhile go without rather than cutting it off.
#[tokio::test(start_paused = true)]
async fn a_popup_sound_still_playing_is_not_stacked_on_by_the_next_popup() {
    LocalSet::new()
        .run_until(async {
            let mut config = base_default_mode_config();
            config.insert(
                "popup_audio_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            let media: &[(&str, &[&str])] = &[
                ("popup.avif", &[]),
                ("effect.ogg", &[shared::tags::POPUP_AUDIO_TAG]),
            ];
            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture_with_data(media, None),
                Content::default(),
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            // Windows and audio draw on one id sequence, so the first popup is 0 and the sound
            // it starts is 1.
            harness.advance(Duration::from_millis(1100)).await;
            harness.send_event(Event::WindowSpawned { id: ItemId(0) });
            harness.pump_events();

            // A second popup while that sound is still going gets none of its own.
            harness.advance(Duration::from_millis(1000)).await;
            harness.send_event(Event::WindowSpawned { id: ItemId(2) });
            harness.pump_events();

            // Once it ends, the next popup sounds again. This half is what keeps the assertion
            // above honest: were these ids wrong, no sound would play here either.
            harness.send_event(Event::AudioFinish { id: ItemId(1) });
            harness.pump_events();
            harness.advance(Duration::from_millis(1000)).await;
            harness.send_event(Event::WindowSpawned { id: ItemId(3) });
            harness.pump_events();

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::SpawnAudio {
                        media_id: 2,
                        volume: 1.0,
                    },
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::SpawnAudio {
                        media_id: 2,
                        volume: 1.0,
                    },
                ]
            );
        })
        .await;
}

/// The other half of the switch above: a pack of short stings under a slow spawner is a
/// legitimate thing to want layered, and the user is the one who can hear which it is. With
/// layering on, the sound still playing does not block the next popup's -- and no slot is held,
/// so no `AudioFinish` is needed to release one.
#[tokio::test(start_paused = true)]
async fn layered_popup_sounds_stack_instead_of_holding_one_slot() {
    LocalSet::new()
        .run_until(async {
            let mut config = base_default_mode_config();
            config.insert(
                "popup_audio_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert(
                "popup_audio_layered".to_string(),
                OptionValue::Boolean(true),
            );
            let media: &[(&str, &[&str])] = &[
                ("popup.avif", &[]),
                ("effect.ogg", &[shared::tags::POPUP_AUDIO_TAG]),
            ];
            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture_with_data(media, None),
                Content::default(),
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();

            harness.advance(Duration::from_millis(1100)).await;
            harness.send_event(Event::WindowSpawned { id: ItemId(0) });
            harness.pump_events();
            harness.advance(Duration::from_millis(1000)).await;
            harness.send_event(Event::WindowSpawned { id: ItemId(2) });
            harness.pump_events();

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::SpawnAudio {
                        media_id: 2,
                        volume: 1.0,
                    },
                    Recorded::SpawnImage { media_id: 1 },
                    Recorded::SpawnAudio {
                        media_id: 2,
                        volume: 1.0,
                    },
                ]
            );
        })
        .await;
}

/// Popup sounds carry the user's popup volume, not the engine default -- the one magnitude
/// the user owns here (comfort, not pacing; see `behaviour-design/default-mode-v2.md`).
#[tokio::test(start_paused = true)]
async fn popup_sounds_play_at_the_users_popup_volume() {
    LocalSet::new()
        .run_until(async {
            let mut config = base_default_mode_config();
            config.insert(
                "popup_audio_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert("popup_volume".to_string(), OptionValue::Number(0.25));
            let media: &[(&str, &[&str])] = &[
                ("popup.avif", &[]),
                ("effect.ogg", &[shared::tags::POPUP_AUDIO_TAG]),
            ];
            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture_with_data(media, None),
                Content::default(),
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;
            harness.send_event(Event::WindowSpawned { id: ItemId(0) });
            harness.pump_events();

            assert!(
                harness.recorded().iter().any(|recorded| matches!(
                    recorded,
                    Recorded::SpawnAudio {
                        media_id: 2,
                        volume
                    } if (*volume - 0.25).abs() < f32::EPSILON
                )),
                "expected the popup sound at the user's 0.25, got {:?}",
                harness.recorded()
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn open_popup_skips_caption_silently_when_pool_empty_or_unmatched() {
    LocalSet::new()
        .run_until(async {
            for content in [
                Content::default(),
                Content {
                    captions: vec![shared::behaviour::TextItem {
                        text: "Obey.".to_string(),
                        tags: vec!["nonexistent".to_string()],
                        timeout_seconds: None,
                        summary: None,
                    }],
                    ..Default::default()
                },
            ] {
                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(true),
                    content,
                    base_default_mode_config(),
                    all_capabilities(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1100)).await;

                assert_eq!(
                    harness.recorded(),
                    vec![Recorded::SpawnImage { media_id: 1 }],
                    "expected no SetTitle when the caption pool is empty or unmatched"
                );
            }
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn captions_disabled_via_config_leaves_title_unset() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                captions: vec![shared::behaviour::TextItem {
                    text: "Obey.".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let mut config = base_default_mode_config();
            config.insert("captions_enabled".to_string(), OptionValue::Boolean(false));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(true),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;

            assert_eq!(
                harness.recorded(),
                vec![Recorded::SpawnImage { media_id: 1 }],
                "expected no SetTitle when captions are disabled via config"
            );
        })
        .await;
}
