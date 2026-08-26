use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

#[tokio::test(start_paused = true)]
async fn web_link_opens_its_url() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                web_links: vec![shared::behaviour::WebLink {
                    url: "https://example.com/".to_string(),
                    tags: vec![],
                }],
                ..Default::default()
            };

            let mut config = isolated_process_config();
            config.insert(
                "web_opening_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert("web_frequency".to_string(), OptionValue::Number(1.0));

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

            assert_eq!(
                harness.recorded(),
                vec![Recorded::OpenLink {
                    url: "https://example.com/".to_string()
                }]
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn web_opening_skips_when_pool_empty() {
    LocalSet::new()
        .run_until(async {
            let mut config = isolated_process_config();
            config.insert(
                "web_opening_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert("web_frequency".to_string(), OptionValue::Number(1.0));

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
async fn web_opening_pauses_while_dormant() {
    LocalSet::new()
        .run_until(async {
            let content = Content {
                web_links: vec![shared::behaviour::WebLink {
                    url: "https://example.com/".to_string(),
                    tags: vec![],
                }],
                ..Default::default()
            };

            let mut config = isolated_process_config();
            config.insert(
                "web_opening_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            // Same timing scheme as `notifications_pause_while_dormant`: ticks at
            // 300/600/900/1200ms, dormancy triggers at 1000ms and stays dormant for the rest
            // of the test.
            config.insert("web_frequency".to_string(), OptionValue::Number(0.3));
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
