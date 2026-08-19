use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

/// The hole this closes: a stage's tag set already reached popups, captions, notifications,
/// web links and prompts, but not the background music -- so "this stage is
/// about X" meant everything about it except what you could hear.
#[tokio::test(start_paused = true)]
async fn experience_background_audio_follows_the_stages_tag_set() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[("calm.ogg", &["calm"]), ("harsh.ogg", &["harsh"])];

            let experience = levels_to_experience(vec![Level {
                tags: Some(vec!["harsh".to_string()]),
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
                vec![Recorded::SpawnAudio {
                    media_id: 2,
                    volume: 1.0,
                }],
                "the stage restricts content to `harsh`, so harsh.ogg is the only background \
                 track eligible -- calm.ogg must not be picked"
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn experience_stage_audio_switches_once_and_none_retains_it() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[("first.ogg", &[]), ("second.ogg", &[])];
            let mut experience = levels_to_experience(vec![
                Level::default(),
                Level {
                    at_seconds: 0.1,
                    ..Default::default()
                },
                Level {
                    at_seconds: 0.2,
                    ..Default::default()
                },
            ]);
            experience.timeline.stages[0].content.audio = Some(1);
            experience.timeline.stages[1].content.audio = Some(2);
            // Third remains None: it must retain second. Repeating the second id would likewise
            // compare equal and avoid a restart.
            let mut config = isolated_experience_process_config();
            config.insert(
                "background_audio_enabled".into(),
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
            harness.advance(Duration::from_millis(300)).await;
            let audio: Vec<_> = harness
                .recorded()
                .iter()
                .filter_map(|entry| match entry {
                    Recorded::SpawnAudio { media_id, .. } => Some(*media_id),
                    _ => None,
                })
                .collect();
            assert_eq!(audio, vec![1, 2]);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn experience_scheduled_sounds_count_towards_stage_end() {
    LocalSet::new()
        .run_until(async {
            use shared::behaviour::{
                CountScope, EndStrategy, EventCountCondition, EventKind, EventSchedule, Interval,
            };

            const MEDIA: &[(&str, &[&str])] = &[("sting.ogg", &[shared::tags::POPUP_AUDIO_TAG])];
            let mut experience = levels_to_experience(vec![
                Level::default(),
                Level {
                    at_seconds: 100.0,
                    ..Default::default()
                },
            ]);
            let first = &mut experience.timeline.stages[0];
            first.events.sound = Some(EventSchedule {
                interval: Interval::Fixed { seconds: 0.05 },
                initial_delay_seconds: Some(0.01),
                max_concurrent: None,
            });
            first.end = Some(shared::behaviour::StageEnd {
                duration_seconds: None,
                event_count: Some(EventCountCondition {
                    event: EventKind::Sound,
                    count: 2,
                    scope: CountScope::Stage,
                }),
                strategy: EndStrategy::Any,
            });

            let mut config = isolated_experience_process_config();
            config.insert("popup_audio_enabled".into(), OptionValue::Boolean(true));
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
            harness.advance(Duration::from_millis(150)).await;

            assert_eq!(
                harness
                    .recorded()
                    .iter()
                    .filter(|entry| matches!(entry, Recorded::SpawnAudio { media_id: 1, .. }))
                    .count(),
                2
            );
            assert_eq!(
                harness.eval_number("require('timeline').stage_index()"),
                2.0
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn experience_crossfade_starts_the_next_track_before_the_transition_ends() {
    LocalSet::new()
        .run_until(async {
            use shared::behaviour::TransitionCategory;

            const MEDIA: &[(&str, &[&str])] = &[("first.ogg", &[]), ("second.ogg", &[])];
            let mut experience = levels_to_experience(vec![
                Level::default(),
                Level {
                    at_seconds: 0.1,
                    ..Default::default()
                },
            ]);
            experience.timeline.stages[0].content.audio = Some(1);
            experience.timeline.stages[1].content.audio = Some(2);
            experience.timeline.transitions[0].duration_seconds = 1.0;
            experience.timeline.transitions[0].affected = vec![TransitionCategory::Crossfade];

            let mut config = isolated_experience_process_config();
            config.insert(
                "background_audio_enabled".into(),
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
            harness.advance(Duration::from_millis(200)).await;

            let audio: Vec<_> = harness
                .recorded()
                .iter()
                .filter_map(|entry| match entry {
                    Recorded::SpawnAudio { media_id, .. } => Some(*media_id),
                    _ => None,
                })
                .collect();
            assert_eq!(audio, vec![1, 2]);
            assert_eq!(
                harness
                    .recorded()
                    .iter()
                    .filter_map(|entry| match entry {
                        Recorded::FadeAudioVolume {
                            volume, duration, ..
                        } => {
                            Some((*volume, *duration))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                vec![(0.0, 1000), (1.0, 1000)]
            );
            assert_eq!(
                harness.eval_string("require('timeline').phase()"),
                "transition"
            );
        })
        .await;
}

/// The fallback that keeps converted packs audible. Edgeware moods tag the images, not the
/// soundtrack, so narrowing the background pool by the stage's tags would leave every
/// restricted stage silent. An empty narrowed pool falls back to the whole background pool --
/// interaction rule 5 (empty pools are skip-and-continue), applied to a loop that would
/// otherwise have no way to restart itself.
#[tokio::test(start_paused = true)]
async fn experience_background_audio_falls_back_when_no_track_carries_the_stages_tags() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[("art.avif", &["kinky"]), ("music.ogg", &[])];

            let experience = levels_to_experience(vec![Level {
                tags: Some(vec!["kinky".to_string()]),
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
                vec![Recorded::SpawnAudio {
                    media_id: 2,
                    volume: 1.0,
                }],
                "music.ogg carries none of the stage's tags, but silence is the wrong answer: \
                 the narrowed pool is empty, so the whole background pool applies"
            );
        })
        .await;
}
