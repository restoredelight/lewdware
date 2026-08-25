use super::*;

/// The shipped Lua the default modes are built from. Named here so the tests reference the
/// real files by name rather than repeating fragile relative paths.
pub(super) const LIB_MEDIA: &str = include_str!("../../../../default-modes/shared/lib/media.lua");
pub(super) const LIB_CONTENT: &str =
    include_str!("../../../../default-modes/shared/lib/content.lua");
pub(super) const LIB_WEB: &str = include_str!("../../../../default-modes/shared/lib/web.lua");
pub(super) const LIB_WALLPAPER: &str =
    include_str!("../../../../default-modes/shared/lib/wallpaper.lua");
pub(super) const LIB_SPAWN: &str = include_str!("../../../../default-modes/shared/lib/spawn.lua");
pub(super) const LIB_PROMPTS: &str =
    include_str!("../../../../default-modes/shared/lib/prompts.lua");
pub(super) const LIB_NOTIFICATIONS: &str =
    include_str!("../../../../default-modes/shared/lib/notifications.lua");
pub(super) const EXPERIENCE_TIMELINE: &str =
    include_str!("../../../../default-modes/experience/src/timeline.lua");
pub(super) const EXPERIENCE_MAIN: &str =
    include_str!("../../../../default-modes/experience/src/main.lua");
pub(super) const SANDBOX_MAIN: &str =
    include_str!("../../../../default-modes/sandbox/src/main.lua");

/// Builds the `(sources, Content)` pair shared by `lib/content.lua`'s own tests: no media
/// pack fixture needed beyond the media-less-by-default default (the picker never touches
/// `lewdware.media.*`), so every test below runs `main.lua` against `pack_fixture(false)`.
pub(super) fn content_lib_sources(main: &str) -> Vec<(&str, &str)> {
    vec![
        ("main.lua", main),
        ("lib/content.lua", LIB_CONTENT),
        ("lib/media.lua", LIB_MEDIA),
    ]
}

/// The real `default-modes` mode's own source files (not test-only stand-ins) -- for tests
/// that exercise `main.lua`'s actual process wiring end-to-end rather than a re-implementation
/// of its logic. Extended with additional `lib/*.lua` entries as later milestone phases add
/// new process modules.
pub(super) fn real_default_mode_sources() -> Vec<(&'static str, &'static str)> {
    vec![
        ("main.lua", SANDBOX_MAIN),
        ("lib/media.lua", LIB_MEDIA),
        ("lib/content.lua", LIB_CONTENT),
        ("lib/notifications.lua", LIB_NOTIFICATIONS),
        ("lib/web.lua", LIB_WEB),
        ("lib/prompts.lua", LIB_PROMPTS),
        ("lib/wallpaper.lua", LIB_WALLPAPER),
        ("lib/spawn.lua", LIB_SPAWN),
    ]
}

/// Like `real_default_mode_sources`, but the real `main.lua` is aliased to `real_main.lua` and
/// `setup` runs first as `main.lua` -- lets a test wrap an API function (e.g.
/// `lewdware.show_notification`, `lewdware.popup.dialog`) before the real mode's processes
/// start. This works because the library modules look up `lewdware.*` fields fresh each time
/// their scheduled callback fires, not once at `require()`/`start()` time, so the override
/// only needs to land before the first `advance()`, not before `require("real_main")` itself.
pub(super) fn wrapped_default_mode_sources(setup: &str) -> Vec<(String, String)> {
    let mut sources = vec![(
        "main.lua".to_string(),
        format!("{setup}\nrequire(\"real_main\")\n"),
    )];
    for (path, source) in real_default_mode_sources() {
        let path = if path == "main.lua" {
            "real_main.lua"
        } else {
            path
        };
        sources.push((path.to_string(), source.to_string()));
    }
    sources
}

/// Like `real_default_mode_sources`, but the shipped Experience mode
/// (`default-modes/experience/src/main.lua`) -- everything except `main.lua` is shared with
/// Sandbox (`lib/*.lua`), so both source lists point at the same files.
pub(super) fn real_experience_mode_sources() -> Vec<(&'static str, &'static str)> {
    vec![
        ("main.lua", EXPERIENCE_MAIN),
        ("timeline.lua", EXPERIENCE_TIMELINE),
        ("lib/media.lua", LIB_MEDIA),
        ("lib/content.lua", LIB_CONTENT),
        ("lib/notifications.lua", LIB_NOTIFICATIONS),
        ("lib/web.lua", LIB_WEB),
        ("lib/prompts.lua", LIB_PROMPTS),
        ("lib/wallpaper.lua", LIB_WALLPAPER),
        ("lib/spawn.lua", LIB_SPAWN),
    ]
}

/// Like `wrapped_default_mode_sources`, but for Experience (see `real_experience_mode_sources`).
pub(super) fn wrapped_experience_mode_sources(setup: &str) -> Vec<(String, String)> {
    let mut sources = vec![(
        "main.lua".to_string(),
        format!("{setup}\nrequire(\"real_main\")\n"),
    )];
    for (path, source) in real_experience_mode_sources() {
        let path = if path == "main.lua" {
            "real_main.lua"
        } else {
            path
        };
        sources.push((path.to_string(), source.to_string()));
    }
    sources
}

/// The audio group both modes share (`audio` in either `config.jsonc`): both roles off, so a
/// test that wants sound turns on the one role it is about. The volumes are still supplied --
/// the modes read them whether or not the toggle is on, and the harness materialises no
/// defaults.
pub(super) fn base_audio_config() -> HashMap<String, OptionValue> {
    HashMap::from([
        (
            "background_audio_enabled".to_string(),
            OptionValue::Boolean(false),
        ),
        ("background_volume".to_string(), OptionValue::Number(100.0)),
        (
            "popup_audio_enabled".to_string(),
            OptionValue::Boolean(false),
        ),
        ("popup_volume".to_string(), OptionValue::Number(100.0)),
        (
            "popup_audio_layered".to_string(),
            OptionValue::Boolean(false),
        ),
    ])
}

/// Baseline `mode_config` covering every key `default-modes/sandbox/src/main.lua` reads
/// unconditionally at startup -- image-only, constant-interval spawning at 1 popup/second,
/// every optional feature off. The harness hands Lua `lewdware.config` exactly as given
/// (unlike the real app, which resolves defaults first via `resolve_mode_config`), so a test
/// running the real `main.lua` must supply every key it reads; tests extend/override
/// individual keys via `.insert(...)` on the returned map.
pub(super) fn base_default_mode_config() -> HashMap<String, OptionValue> {
    let mut config = HashMap::new();
    config.insert(
        "spawn_mode".to_string(),
        OptionValue::Enum("constant".to_string()),
    );
    config.insert("popup_frequency".to_string(), OptionValue::Number(1.0));
    config.insert("max_popups".to_string(), OptionValue::Integer(5));
    config.insert("images_enabled".to_string(), OptionValue::Boolean(true));
    config.insert("videos_enabled".to_string(), OptionValue::Boolean(false));
    config.extend(base_audio_config());
    config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(false));
    config.insert(
        "close_trigger_enabled".to_string(),
        OptionValue::Boolean(false),
    );
    config.insert("movement_enabled".to_string(), OptionValue::Boolean(false));
    config.insert("captions_enabled".to_string(), OptionValue::Boolean(true));
    config
}

/// `base_default_mode_config()` with popup spawning turned off (`images_enabled = false`),
/// for tests exercising one of the frequency-scheduled processes (notifications/web/
/// prompts) in isolation, unpolluted by the ordinary spawn loop's own
/// `SpawnImage`/`SetTitle` traffic.
pub(super) fn isolated_process_config() -> HashMap<String, OptionValue> {
    let mut config = base_default_mode_config();
    config.insert("images_enabled".to_string(), OptionValue::Boolean(false));
    config
}

/// Like `base_default_mode_config`, but for Experience's meta-controls schema
/// (`default-modes/experience/config.jsonc`): `pace = 1.0` (no scaling), image-only spawning,
/// every off-switch on (rate/design values themselves come from the test's `Experience`
/// fixture, not this config map -- see `behaviour-design/default-mode.md`, Ownership).
pub(super) fn base_experience_mode_config() -> HashMap<String, OptionValue> {
    let mut config = HashMap::new();
    config.insert("pace".to_string(), OptionValue::Number(1.0));
    config.insert("max_popups".to_string(), OptionValue::Integer(5));
    config.insert("images_enabled".to_string(), OptionValue::Boolean(true));
    config.insert("videos_enabled".to_string(), OptionValue::Boolean(false));
    config.extend(base_audio_config());
    config.insert(
        "close_trigger_enabled".to_string(),
        OptionValue::Boolean(false),
    );
    config.insert("movement_enabled".to_string(), OptionValue::Boolean(false));
    config.insert("captions_enabled".to_string(), OptionValue::Boolean(true));
    config.insert(
        "notifications_enabled".to_string(),
        OptionValue::Boolean(true),
    );
    config.insert(
        "web_opening_enabled".to_string(),
        OptionValue::Boolean(true),
    );
    config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
    config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));
    config.insert("splash_enabled".to_string(), OptionValue::Boolean(true));
    config
}

/// `base_experience_mode_config()` with popup spawning turned off, mirroring
/// `isolated_process_config`'s reasoning.
pub(super) fn isolated_experience_process_config() -> HashMap<String, OptionValue> {
    let mut config = base_experience_mode_config();
    config.insert("images_enabled".to_string(), OptionValue::Boolean(false));
    config
}

/// Test-only, cumulative-timeline-friendly shape: far terser to hand-write than a
/// `Stage`/`Transition` graph directly, since a level is a flat, self-contained snapshot
/// rather than needing its trigger expressed as the *previous* level's own `end` condition.
/// `levels_to_experience` below converts a `Vec<Level>` into the real thing.
#[derive(Default)]
pub(super) struct Level {
    pub(super) at_seconds: f64,
    pub(super) at_popups: Option<u32>,
    pub(super) anchors: FrequencyAnchors,
    pub(super) design: DesignValues,
    pub(super) tags: Option<Vec<String>>,
    /// Tags that keep media out of the stage, whatever `tags` lets in.
    pub(super) exclude: Vec<String>,
    pub(super) wallpaper: Option<u64>,
}

#[derive(Default)]
pub(super) struct FrequencyAnchors {
    pub(super) popup: Option<f64>,
    pub(super) web: Option<f64>,
    pub(super) notification: Option<f64>,
    pub(super) prompt: Option<f64>,
}

#[derive(Default)]
pub(super) struct DesignValues {
    pub(super) movement_speed_min: Option<f64>,
    pub(super) movement_speed_max: Option<f64>,
    pub(super) mitosis_chance: Option<f64>,
    pub(super) mitosis_count: Option<u32>,
}

pub(super) fn schedule(seconds: Option<f64>) -> Option<shared::behaviour::EventSchedule> {
    seconds.map(|seconds| shared::behaviour::EventSchedule {
        interval: shared::behaviour::Interval::Fixed { seconds },
        initial_delay_seconds: None,
        max_concurrent: None,
    })
}

pub(super) fn movement(design: &DesignValues) -> Option<shared::behaviour::Movement> {
    match (design.movement_speed_min, design.movement_speed_max) {
        (None, None) => None,
        (minimum_speed, maximum_speed) => Some(shared::behaviour::Movement {
            minimum_speed,
            maximum_speed,
        }),
    }
}

pub(super) fn mitosis(design: &DesignValues) -> Option<shared::behaviour::Mitosis> {
    match (design.mitosis_chance, design.mitosis_count) {
        (None, None) => None,
        (chance, count) => Some(shared::behaviour::Mitosis { chance, count }),
    }
}

/// Converts a cumulative `Vec<Level>` into a real `Experience`: each level becomes a
/// `Stage`, and a level's `at_seconds`/`at_popups` (the trigger for *reaching* it) becomes
/// the *previous* stage's `StageEnd` (the condition for *leaving* it) -- the last level never
/// gets an `end`. One zero-duration, linear, unaffected `Transition` is synthesized per
/// adjacent stage pair, since a cumulative level's values always apply instantly.
pub(super) fn levels_to_experience(levels: Vec<Level>) -> Experience {
    use shared::behaviour::{
        ContentSelection, CountScope, Easing, EndStrategy, EventCountCondition, EventKind, Events,
        Stage, StageEnd, Timeline, Transition,
    };

    let stages = levels
        .iter()
        .enumerate()
        .map(|(index, level)| {
            let next = levels.get(index + 1);
            let end = next.map(|next| StageEnd {
                duration_seconds: Some((next.at_seconds - level.at_seconds).max(0.0)),
                event_count: next.at_popups.map(|count| EventCountCondition {
                    event: EventKind::Popup,
                    count,
                    scope: CountScope::Session,
                }),
                strategy: EndStrategy::Any,
            });
            Stage {
                id: format!("stage-{}", index + 1),
                label: format!("Stage {}", index + 1),
                end,
                content: ContentSelection {
                    tags: level.tags.clone(),
                    exclude: level.exclude.clone(),
                    wallpaper: level.wallpaper,
                    ..ContentSelection::default()
                },
                events: Events {
                    popup: schedule(level.anchors.popup),
                    web: schedule(level.anchors.web),
                    notification: schedule(level.anchors.notification),
                    prompt: schedule(level.anchors.prompt),
                    sound: None,
                },
                movement: movement(&level.design),
                mitosis: mitosis(&level.design),
                on_enter: Default::default(),
                prompt: Default::default(),
            }
        })
        .collect::<Vec<_>>();
    let transitions = stages
        .windows(2)
        .enumerate()
        .map(|(index, pair)| Transition {
            id: format!("transition-{}", index + 1),
            from_stage: pair[0].id.clone(),
            to_stage: pair[1].id.clone(),
            duration_seconds: 0.0,
            easing: Easing::Linear,
            affected: vec![],
        })
        .collect();
    Experience {
        timeline: Timeline {
            stages,
            transitions,
        },
        label: None,
    }
}

/// Builds a single-level `Experience` (just the baseline, no escalation at all) from a
/// baseline `FrequencyAnchors`/`DesignValues` pair -- the common case for tests exercising
/// anchor/design arithmetic in isolation from the timeline's own trigger mechanics.
pub(super) fn experience_with_baseline(
    anchors: FrequencyAnchors,
    design: DesignValues,
) -> Experience {
    levels_to_experience(vec![Level {
        at_seconds: 0.0,
        at_popups: None,
        anchors,
        design,
        tags: None,
        exclude: Vec::new(),
        wallpaper: None,
    }])
}

pub(super) fn as_str_sources(owned: &[(String, String)]) -> Vec<(&str, &str)> {
    owned
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect()
}

pub(super) const COUNTING_NOTIFICATION_WRAP: &str = r#"
    COUNT = 0
    local real = lewdware.show_notification
    lewdware.show_notification = function(n)
        COUNT = COUNT + 1
        return real(n)
    end
"#;

pub(super) const DIALOG_ELEMENT_CAPTURE_WRAP: &str = r#"
    DIALOG_TEXT = nil
    DIALOG_COUNTDOWN = nil
    DIALOG_LABEL = nil
    local real = lewdware.popup.dialog
    lewdware.popup.dialog = function(opts)
        for _, el in ipairs(opts.elements) do
            if el.type == "text" and el.bold then DIALOG_TEXT = el.text end
            if el.id == "countdown" then DIALOG_COUNTDOWN = el.text end
            if el.type == "buttons" then DIALOG_LABEL = el.options[1].label end
        end
        return real(opts)
    end
"#;

pub(super) const WALLPAPER_CAPTURE_WRAP: &str = r#"
    SET_COUNT = 0
    RESET_COUNT = 0
    SET_IMAGE_NAME = nil
    local real_set = lewdware.wallpaper.set
    lewdware.wallpaper.set = function(image)
        SET_COUNT = SET_COUNT + 1
        SET_IMAGE_NAME = image.name
        return real_set(image)
    end
    local real_reset = lewdware.wallpaper.reset
    lewdware.wallpaper.reset = function()
        RESET_COUNT = RESET_COUNT + 1
        return real_reset()
    end
"#;
