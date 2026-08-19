use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

// ─── Experience mode ────────────────────────────────────────────────────────

/// The core Experience arithmetic: `effective_interval = anchor_seconds / pace`, exercised
/// against the real `default-modes/experience/src/main.lua`. Anchor 2s x pace 2.0 -> 1s
/// popups, so three should have spawned after 3.1s (mirrors the Sandbox constant-rate spawn
/// tests' shape).
#[tokio::test(start_paused = true)]
async fn experience_popup_spawns_at_anchor_over_pace_interval() {
    LocalSet::new()
        .run_until(async {
            let experience = experience_with_baseline(
                FrequencyAnchors {
                    popup: Some(2.0),
                    ..Default::default()
                },
                DesignValues::default(),
            );

            let mut config = base_experience_mode_config();
            config.insert("pace".to_string(), OptionValue::Number(2.0));

            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture(true),
                Content::default(),
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(3100)).await;

            let spawn_count = harness
                .recorded()
                .iter()
                .filter(|r| matches!(r, Recorded::SpawnImage { .. }))
                .count();
            assert_eq!(spawn_count, 3);
        })
        .await;
}

/// An absent `anchors.popup` means the spawn loop never starts at all in Experience -- not
/// "spawn at some default rate" -- see `FrequencyAnchors`'s doc comment and interaction rule
/// 5's "skip, don't error" spirit generalized to "doesn't exist here".
#[tokio::test(start_paused = true)]
async fn experience_missing_popup_anchor_never_spawns() {
    LocalSet::new()
        .run_until(async {
            let experience = Experience::default(); // no anchors at all

            let harness_config = base_experience_mode_config();

            let mut harness = Harness::with_pack_and_experience(
                &real_experience_mode_sources(),
                pack_fixture(true),
                Content::default(),
                experience,
                harness_config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_secs(60)).await;

            assert!(harness.recorded().is_empty());
        })
        .await;
}

/// Same anchor-over-pace arithmetic, for a standalone-scheduled feature (notifications)
/// rather than the spawn loop -- confirms `experience/src/main.lua` threads `anchors.* /
/// pace` through to `lib/notifications.lua` correctly.
#[tokio::test(start_paused = true)]
async fn experience_notification_anchor_scaled_by_pace() {
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
            let experience = experience_with_baseline(
                FrequencyAnchors {
                    notification: Some(2.0),
                    ..Default::default()
                },
                DesignValues::default(),
            );

            let mut config = isolated_experience_process_config();
            config.insert("pace".to_string(), OptionValue::Number(2.0));

            let owned = wrapped_experience_mode_sources(COUNTING_NOTIFICATION_WRAP);
            let sources = as_str_sources(&owned);

            let mut harness = Harness::with_pack_and_experience(
                &sources,
                pack_fixture(false),
                content,
                experience,
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

/// The user's off-switch still wins even when the pack's design anchors the feature --
/// class-1 ownership applies identically in Experience (`behaviour-design/default-mode.md`,
/// Ownership: "Available in *both* modes; the timeline can never touch them").
#[tokio::test(start_paused = true)]
async fn experience_notifications_off_switch_wins_over_present_anchor() {
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
            let experience = experience_with_baseline(
                FrequencyAnchors {
                    notification: Some(1.0),
                    ..Default::default()
                },
                DesignValues::default(),
            );

            let mut config = isolated_experience_process_config();
            config.insert(
                "notifications_enabled".to_string(),
                OptionValue::Boolean(false),
            );

            let owned = wrapped_experience_mode_sources(COUNTING_NOTIFICATION_WRAP);
            let sources = as_str_sources(&owned);

            let mut harness = Harness::with_pack_and_experience(
                &sources,
                pack_fixture(false),
                content,
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_secs(10)).await;

            assert_eq!(harness.eval_number("COUNT"), 0.0);
        })
        .await;
}

// ─── Transitions (timeline) ─────────────────────────────────────────────────
