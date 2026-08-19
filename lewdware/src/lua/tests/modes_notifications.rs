use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

#[tokio::test(start_paused = true)]
async fn notifications_fire_at_configured_frequency() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                notifications: vec![shared::behaviour::TextItem {
                    text: "hi".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(COUNTING_NOTIFICATION_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert(
                "notifications_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert(
                "notification_frequency".to_string(),
                OptionValue::Number(1.0),
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(3100)).await;

            assert_eq!(harness.eval_number("COUNT"), 3.0);
        })
        .await;
}

/// Records what the last notification was actually shown with, rather than only that one was:
/// `TextItem.summary` is the pack author's title, and a mode that dropped it on the floor
/// would look identical to `COUNTING_NOTIFICATION_WRAP`.
const RECORDING_NOTIFICATION_WRAP: &str = r#"
    SUMMARY = ""
    BODY = ""
    local real = lewdware.show_notification
    lewdware.show_notification = function(n)
        SUMMARY = n.summary or ""
        BODY = n.body
        return real(n)
    end
"#;

#[tokio::test(start_paused = true)]
async fn notifications_carry_the_authors_title() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                notifications: vec![shared::behaviour::TextItem {
                    text: "hi".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: Some("Attention".to_string()),
                }],
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(RECORDING_NOTIFICATION_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert(
                "notifications_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert(
                "notification_frequency".to_string(),
                OptionValue::Number(1.0),
            );

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

            assert_eq!(harness.eval_string("SUMMARY"), "Attention");
            assert_eq!(harness.eval_string("BODY"), "hi");
        })
        .await;
}

/// A pack that left the title blank -- every converted Edgeware pack -- must reach the
/// notification with no summary at all, not with an empty-string one.
#[tokio::test(start_paused = true)]
async fn notifications_without_a_title_pass_no_summary() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                notifications: vec![shared::behaviour::TextItem {
                    text: "hi".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(
                r#"
                    HAD_SUMMARY = 1
                    local real = lewdware.show_notification
                    lewdware.show_notification = function(n)
                        HAD_SUMMARY = n.summary == nil and 0 or 1
                        return real(n)
                    end
                "#,
            );
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert(
                "notifications_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert(
                "notification_frequency".to_string(),
                OptionValue::Number(1.0),
            );

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

            assert_eq!(harness.eval_number("HAD_SUMMARY"), 0.0);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn notifications_skip_when_pool_empty_but_enabled() {
    LocalSet::new()
        .run_until(async {
            let owned = wrapped_default_mode_sources(COUNTING_NOTIFICATION_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert(
                "notifications_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert(
                "notification_frequency".to_string(),
                OptionValue::Number(1.0),
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                Content::default(),
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2100)).await;

            assert_eq!(harness.eval_number("COUNT"), 0.0);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn notifications_disabled_never_fires() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                notifications: vec![shared::behaviour::TextItem {
                    text: "hi".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(COUNTING_NOTIFICATION_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert(
                "notifications_enabled".to_string(),
                OptionValue::Boolean(false),
            );
            config.insert(
                "notification_frequency".to_string(),
                OptionValue::Number(1.0),
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2100)).await;

            assert_eq!(harness.eval_number("COUNT"), 0.0);
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn notifications_pause_while_dormant() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                notifications: vec![shared::behaviour::TextItem {
                    text: "hi".to_string(),
                    tags: vec![],
                    timeout_seconds: None,
                    summary: None,
                }],
                ..Default::default()
            };

            let owned = wrapped_default_mode_sources(COUNTING_NOTIFICATION_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = isolated_process_config();
            config.insert(
                "notifications_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            // Ticks at 300ms/600ms/900ms/1200ms...; `active_min`/`active_max` (and
            // `dormant_min`/`dormant_max`) are whole seconds -- `schedule_dormancy` passes
            // them straight to `math.random(m, n)`, which requires integer-valued arguments.
            // Dormancy triggers at 1000ms and stays dormant for the rest of the test
            // (dormant_min/max = 10s): the 300/600/900ms ticks should fire, the 1200ms+ ticks
            // should all be suppressed.
            config.insert(
                "notification_frequency".to_string(),
                OptionValue::Number(0.3),
            );
            config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(true));
            config.insert("active_min".to_string(), OptionValue::Number(1.0));
            config.insert("active_max".to_string(), OptionValue::Number(1.0));
            config.insert("dormant_min".to_string(), OptionValue::Number(10.0));
            config.insert("dormant_max".to_string(), OptionValue::Number(10.0));

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(false),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1300)).await;

            assert_eq!(harness.eval_number("COUNT"), 3.0);
        })
        .await;
}
