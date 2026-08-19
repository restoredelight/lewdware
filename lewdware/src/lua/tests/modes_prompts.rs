use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

#[tokio::test(start_paused = true)]
async fn prompt_dialog_spawns_with_pool_text_and_submit_label() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                prompts: vec![shared::behaviour::TextItem {
                    text: "Well?".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(DIALOG_ELEMENT_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
            config.insert("prompt_frequency".to_string(), OptionValue::Number(1.0));

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;

            assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);
            assert_eq!(harness.eval_string("DIALOG_TEXT"), "Well?");
            assert_eq!(
                harness.eval_string("DIALOG_COUNTDOWN"),
                "Time remaining: 15 seconds"
            );
            assert_eq!(harness.eval_string("DIALOG_LABEL"), "Submit");
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn experience_prompt_timeout_closes_the_dialog() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                prompts: vec![shared::behaviour::TextItem {
                    text: "Well?".into(),
                    tags: vec![],
                    timeout_seconds: Some(0.2),
                    summary: None,
                }],
                ..Default::default()
            };
            let experience = experience_with_baseline(
                FrequencyAnchors {
                    prompt: Some(10.0),
                    ..Default::default()
                },
                DesignValues::default(),
            );
            let mut config = isolated_experience_process_config();
            config.insert("prompts_enabled".into(), OptionValue::Boolean(true));
            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture(false),
                content,
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(10_100)).await;
            assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);
            harness.advance(Duration::from_millis(200)).await;
            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnDialog,
                    Recorded::CloseWindow { id: ItemId(0) },
                ]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn experience_scales_an_automatic_prompt_timeout() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                prompts: vec![shared::behaviour::TextItem {
                    text: "Well?".into(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };
            let mut experience = experience_with_baseline(
                FrequencyAnchors {
                    prompt: Some(10.0),
                    ..Default::default()
                },
                DesignValues::default(),
            );
            // A short prompt's automatic 15-second floor becomes 1.5 seconds at 0.1×.
            experience.timeline.stages[0].prompt.timeout_multiplier = 0.1;
            let mut config = isolated_experience_process_config();
            config.insert("prompts_enabled".into(), OptionValue::Boolean(true));
            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture(false),
                content,
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(10_100)).await;
            assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);
            harness.advance(Duration::from_millis(1_500)).await;

            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnDialog,
                    Recorded::CloseWindow { id: ItemId(0) },
                ]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn experience_can_disable_per_prompt_timeouts_for_a_stage() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                prompts: vec![shared::behaviour::TextItem {
                    text: "Well?".into(),
                    tags: vec![],
                    timeout_seconds: Some(0.2),
                    summary: None,
                }],
                ..Default::default()
            };
            let mut experience = experience_with_baseline(
                FrequencyAnchors {
                    prompt: Some(10.0),
                    ..Default::default()
                },
                DesignValues::default(),
            );
            experience.timeline.stages[0].prompt.timeouts_enabled = false;
            let mut config = isolated_experience_process_config();
            config.insert("prompts_enabled".into(), OptionValue::Boolean(true));
            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture(false),
                content,
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            assert_eq!(
                harness.eval_number("require('timeline').prompt().timeouts_enabled and 1 or 0"),
                0.0
            );
            harness.advance(Duration::from_millis(10_500)).await;

            assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn experience_prompt_timeout_combines_popup_and_sound_consequences() {
    LocalSet::new()
        .run_until(async {
            const MEDIA: &[(&str, &[&str])] = &[("popup.avif", &[]), ("wrong.ogg", &[])];
            let content = Content {
                prompts: vec![shared::behaviour::TextItem {
                    text: "Well?".into(),
                    tags: vec![],
                    timeout_seconds: Some(0.2),
                    summary: None,
                }],
                ..Default::default()
            };
            let mut experience = experience_with_baseline(
                FrequencyAnchors {
                    prompt: Some(10.0),
                    ..Default::default()
                },
                DesignValues::default(),
            );
            experience.timeline.stages[0].prompt.popup_burst = Some(2);
            experience.timeline.stages[0].prompt.sound = Some(2);
            let mut config = isolated_experience_process_config();
            config.insert("prompts_enabled".into(), OptionValue::Boolean(true));
            config.insert("popup_audio_enabled".into(), OptionValue::Boolean(true));
            config.insert("images_enabled".into(), OptionValue::Boolean(true));
            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture_with_data(MEDIA, None),
                content,
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(10_300)).await;

            assert_eq!(
                harness
                    .recorded()
                    .iter()
                    .filter(|entry| matches!(entry, Recorded::SpawnImage { media_id: 1 }))
                    .count(),
                2
            );
            assert!(
                harness
                    .recorded()
                    .iter()
                    .any(|entry| matches!(entry, Recorded::SpawnAudio { media_id: 2, .. }))
            );
            assert!(
                harness
                    .recorded()
                    .iter()
                    .any(|entry| matches!(entry, Recorded::CloseWindow { id: ItemId(0) }))
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn wrong_answer_popup_effect_is_a_noop_when_popups_are_disabled() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                prompts: vec![shared::behaviour::TextItem {
                    text: "Well?".into(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };
            let mut config = isolated_process_config();
            config.insert("prompts_enabled".into(), OptionValue::Boolean(true));
            config.insert("prompt_frequency".into(), OptionValue::Number(1.0));
            config.insert(
                "prompt_wrong_answer".into(),
                OptionValue::Enum("popup_burst".into()),
            );
            config.insert("prompt_wrong_answer_value".into(), OptionValue::Integer(5));
            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(false),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1_100)).await;
            harness.send_event(Event::DialogSelect {
                id: ItemId(0),
                button_id: "submit".into(),
                values: HashMap::from([("response".into(), "wrong".into())]),
            });
            harness.pump_events();
            assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);
        })
        .await;
}

/// A prompt is text to copy out, not a question: it closes only once the user has typed it
/// back, whether they submit with the button or with Enter. Anything else clears the box and
/// leaves the dialog up.
#[tokio::test(start_paused = true)]
async fn a_prompt_closes_only_once_its_text_has_been_typed_back() {
    async fn spawn_one_prompt_dialog() -> Harness {
        let content = Content {
            prompts: vec![shared::behaviour::TextItem {
                text: "Well?".to_string(),
                tags: vec![],
                timeout_seconds: None,
                summary: None,
            }],
            ..Default::default()
        };

        let mut config = isolated_process_config();
        config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
        config.insert("prompt_frequency".to_string(), OptionValue::Number(1.0));

        let mut harness = Harness::with_pack(
            &real_default_mode_sources(),
            pack_fixture(false),
            content,
            config,
            all_capabilities(),
            Volume::default(),
        );
        harness.run_entrypoint("main.lua").unwrap();
        harness.advance(Duration::from_millis(1100)).await;
        assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);
        harness
    }

    fn typed(response: &str) -> HashMap<String, String> {
        HashMap::from([("response".to_string(), response.to_string())])
    }

    LocalSet::new()
        .run_until(async {
            // The submit button, with every answer that is not the prompt's own text.
            for wrong in ["", "well?", "Well", "Something else"] {
                let mut harness = spawn_one_prompt_dialog().await;
                harness.send_event(Event::DialogSelect {
                    id: ItemId(0),
                    button_id: "submit".to_string(),
                    values: typed(wrong),
                });
                harness.pump_events();
                assert_eq!(
                    harness.recorded(),
                    vec![Recorded::SpawnDialog],
                    "{wrong:?} closed the prompt"
                );
            }

            // ...and with the text typed back. Surrounding whitespace is invisible, so it is
            // forgiven; nothing else is.
            for right in ["Well?", "  Well? "] {
                let mut harness = spawn_one_prompt_dialog().await;
                harness.send_event(Event::DialogSelect {
                    id: ItemId(0),
                    button_id: "submit".to_string(),
                    values: typed(right),
                });
                harness.pump_events();
                assert_eq!(
                    harness.recorded(),
                    vec![
                        Recorded::SpawnDialog,
                        Recorded::CloseWindow { id: ItemId(0) },
                    ],
                    "{right:?} did not close the prompt"
                );
            }

            // Enter in the input reaches the same check.
            let mut harness = spawn_one_prompt_dialog().await;
            harness.send_event(Event::DialogSubmit {
                id: ItemId(0),
                element_id: "response".to_string(),
                values: typed("nope"),
            });
            harness.pump_events();
            assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);

            harness.send_event(Event::DialogSubmit {
                id: ItemId(0),
                element_id: "response".to_string(),
                values: typed("Well?"),
            });
            harness.pump_events();
            assert_eq!(
                harness.recorded(),
                vec![
                    Recorded::SpawnDialog,
                    Recorded::CloseWindow { id: ItemId(0) },
                ]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn prompts_skip_when_pool_empty() {
    LocalSet::new()
        .run_until(async {
            let mut config = isolated_process_config();
            config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
            config.insert("prompt_frequency".to_string(), OptionValue::Number(1.0));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(false),
                Content::default(),
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2100)).await;

            assert_eq!(harness.recorded(), vec![]);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn prompts_pause_while_dormant() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                prompts: vec![shared::behaviour::TextItem {
                    text: "Well?".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let mut config = isolated_process_config();
            config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
            // Same timing scheme as `notifications_pause_while_dormant`: ticks at
            // 300/600/900/1200ms, dormancy triggers at 1000ms and stays dormant for the rest
            // of the test -- the 300/600/900ms ticks should spawn a dialog, the 1200ms tick
            // should be suppressed.
            config.insert("prompt_frequency".to_string(), OptionValue::Number(0.3));
            config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(true));
            config.insert("active_min".to_string(), OptionValue::Number(1.0));
            config.insert("active_max".to_string(), OptionValue::Number(1.0));
            config.insert("dormant_min".to_string(), OptionValue::Number(10.0));
            config.insert("dormant_max".to_string(), OptionValue::Number(10.0));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(false),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1300)).await;

            assert_eq!(harness.recorded().len(), 3);
        })
        .await;
}
