use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

/// Captures the theme and palette every spawn actually asked for, whichever popup type it was.
const THEME_CAPTURE_WRAP: &str = r#"
    SPAWNED_THEMES = {}
    SPAWNED_APPEARANCES = {}
    local function record(opts)
        table.insert(SPAWNED_THEMES, (opts or {}).theme or "<unset>")
        table.insert(SPAWNED_APPEARANCES, (opts or {}).appearance or "<unset>")
    end
    local real_image = lewdware.popup.image
    lewdware.popup.image = function(image, opts)
        record(opts)
        return real_image(image, opts)
    end
    local real_dialog = lewdware.popup.dialog
    lewdware.popup.dialog = function(opts)
        record(opts)
        return real_dialog(opts)
    end
"#;

/// Every window-behaviour option Sandbox owns has to survive the trip from `lewdware.config`
/// through `lib/spawn.lua` into the actual spawn call. They are plain pass-throughs, which is
/// exactly why nothing else would catch them going missing: a dropped field is not an error,
/// just a window that quietly ignores the user's setting.
const WINDOW_OPTS_CAPTURE_WRAP: &str = r#"
    SPAWNED_OPTS = {}
    local function fmt(opts)
        opts = opts or {}
        local function show(v)
            if v == nil then return "<unset>" end
            return tostring(v)
        end
        return "decorations=" .. show(opts.decorations)
            .. " draggable=" .. show(opts.draggable)
            .. " opacity=" .. show(opts.opacity)
            .. " click_through=" .. show(opts.click_through)
    end
    local real_image = lewdware.popup.image
    lewdware.popup.image = function(image, opts)
        table.insert(SPAWNED_OPTS, fmt(opts))
        return real_image(image, opts)
    end
"#;

#[tokio::test(start_paused = true)]
async fn sandbox_passes_its_window_behaviour_options_to_every_popup() {
    LocalSet::new()
        .run_until(async {
            let owned = wrapped_default_mode_sources(WINDOW_OPTS_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = base_default_mode_config();
            config.insert(
                "decorations_enabled".to_string(),
                OptionValue::Boolean(true),
            );
            config.insert("draggable".to_string(), OptionValue::Boolean(true));
            config.insert("window_opacity".to_string(), OptionValue::Number(0.5));
            config.insert(
                "click_action".to_string(),
                OptionValue::Enum("nothing".to_string()),
            );

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(true),
                Content::default(),
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;

            let seen = harness.eval_string("table.concat(SPAWNED_OPTS, '|')");
            assert!(!seen.is_empty(), "nothing spawned");
            for opts in seen.split('|') {
                assert_eq!(
                    opts, "decorations=true draggable=true opacity=0.5 click_through=false",
                    "in {seen:?}"
                );
            }
        })
        .await;
}

/// The bundled modes name no theme at all, at any spawn site: chrome is the user's to choose
/// (`AppConfig::theme`), and a mode that hardcoded a look here would quietly take that choice
/// away for every pack it runs. Sandbox's popups and its prompt dialog alike.
#[tokio::test(start_paused = true)]
async fn sandbox_leaves_every_windows_look_to_the_user() {
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

            let owned = wrapped_default_mode_sources(THEME_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = base_default_mode_config();
            config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
            config.insert("prompt_frequency".to_string(), OptionValue::Number(1.0));

            let mut harness = Harness::with_pack(
                &sources,
                pack_fixture(true),
                content,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;

            let themes = harness.eval_string("table.concat(SPAWNED_THEMES, ',')");
            assert!(!themes.is_empty(), "nothing spawned");
            for theme in themes.split(',') {
                assert_eq!(theme, "<unset>", "in {themes:?}");
            }

            let appearances = harness.eval_string("table.concat(SPAWNED_APPEARANCES, ',')");
            for appearance in appearances.split(',') {
                assert_eq!(appearance, "<unset>", "in {appearances:?}");
            }
        })
        .await;
}

/// Sequence too — and this is the mode where the temptation was greatest, since the pack
/// author holds design authority over everything else it does.
#[tokio::test(start_paused = true)]
async fn sequence_leaves_every_windows_look_to_the_user() {
    LocalSet::new()
        .run_until(async {
            let owned = wrapped_experience_mode_sources(THEME_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut harness = Harness::with_pack_and_experience(
                &sources,
                pack_fixture(true),
                Content::default(),
                experience_with_baseline(
                    FrequencyAnchors {
                        popup: Some(1.0),
                        ..Default::default()
                    },
                    DesignValues::default(),
                ),
                base_experience_mode_config(),
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(2100)).await;

            let themes = harness.eval_string("table.concat(SPAWNED_THEMES, ',')");
            assert!(!themes.is_empty(), "nothing spawned");
            for theme in themes.split(',') {
                assert_eq!(theme, "<unset>", "in {themes:?}");
            }
        })
        .await;
}
