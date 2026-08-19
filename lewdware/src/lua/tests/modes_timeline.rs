use std::time::Duration;

use shared::user_config::Volume;

use super::{harness::*, mode_support::*, *};

/// Minimal sources for exercising `experience/src/timeline.lua` in isolation, uncoupled from
/// the spawn loop's own timing -- most of the timeline's own state-machine behaviour (trigger
/// evaluation, absolute-snapshot params) doesn't need a real pack or media at all.
fn timeline_only_sources() -> Vec<(&'static str, &'static str)> {
    vec![
        ("main.lua", "require('timeline')"),
        ("timeline.lua", EXPERIENCE_TIMELINE),
    ]
}

/// Same as `timeline_only_sources`, but also calls `M.init()` -- needed by any test relying on
/// a level's `at_seconds` timer actually firing (as opposed to only `on_popup_spawned()`-driven
/// advances).
fn timeline_only_sources_initialized() -> Vec<(&'static str, &'static str)> {
    vec![
        ("main.lua", "require('timeline').init()"),
        ("timeline.lua", EXPERIENCE_TIMELINE),
    ]
}

fn timeline_harness(sources: &[(&str, &str)], experience: Experience) -> Harness {
    Harness::with_pack_and_experience(
        sources,
        pack_fixture(false),
        Content::default(),
        experience,
        HashMap::new(),
        all_capabilities(),
        Volume::default(),
    )
}

/// A pack with no levels at all (the common case: most Experience packs won't design an arc,
/// and a purely static design is just a single baseline level with no timeline levels beyond
/// it) must leave every getter inert forever -- an empty `levels` is not a degraded case, it's
/// the common one -- see `Timeline`'s doc comment.
#[tokio::test(start_paused = true)]
async fn experience_timeline_with_no_levels_stays_inert_forever() {
    LocalSet::new()
        .run_until(async {
            let experience = Experience::default(); // no levels at all
            let mut harness = timeline_harness(&timeline_only_sources_initialized(), experience);
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_secs(3600)).await;

            assert_eq!(
                harness.eval_string("tostring(require('timeline').anchors().popup)"),
                "nil"
            );
            assert_eq!(
                harness.eval_string("table.concat(require('timeline').tags() or {}, '|')"),
                ""
            );
            assert_eq!(
                harness.eval_string("tostring(require('timeline').wallpaper())"),
                "nil"
            );
        })
        .await;
}

/// The core trigger + snapshot mechanics: before a level's `at_seconds`, every getter stays at
/// the baseline (level 0); once active-time elapses past it, `anchors()`/`tags()` jump to that
/// level's own absolute values in one step (interaction rules 1-3).
#[tokio::test(start_paused = true)]
async fn experience_timeline_advances_on_active_time_and_applies_level_values() {
    LocalSet::new()
        .run_until(async {
            let experience = levels_to_experience(vec![
                Level {
                    anchors: FrequencyAnchors {
                        popup: Some(1.0),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Level {
                    at_seconds: 5.0,
                    anchors: FrequencyAnchors {
                        popup: Some(4.0),
                        ..Default::default()
                    },
                    tags: Some(vec!["kinky".to_string()]),
                    ..Default::default()
                },
            ]);
            let mut harness = timeline_harness(&timeline_only_sources_initialized(), experience);
            harness.run_entrypoint("main.lua").unwrap();

            harness.advance(Duration::from_millis(4900)).await;
            assert_eq!(
                harness.eval_number("require('timeline').anchors().popup"),
                1.0,
                "still baseline just before at_seconds"
            );

            harness.advance(Duration::from_millis(300)).await; // crosses t=5.0s
            assert_eq!(
                harness.eval_number("require('timeline').anchors().popup"),
                4.0
            );
            assert_eq!(
                harness.eval_string("table.concat(require('timeline').tags() or {}, '|')"),
                "kinky"
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn experience_v3_holds_stage_then_interpolates_selected_transition_values() {
    use shared::behaviour::{
        ContentSelection, Easing, EndStrategy, EventSchedule, Events, Interval, Stage, StageEnd,
        Timeline, Transition, TransitionCategory,
    };

    LocalSet::new()
        .run_until(async {
            let event = |seconds| EventSchedule {
                interval: Interval::Fixed { seconds },
                initial_delay_seconds: None,
                max_concurrent: None,
            };
            let first = Stage {
                id: "first".into(),
                label: "First".into(),
                end: Some(StageEnd {
                    duration_seconds: Some(1.0),
                    event_count: None,
                    strategy: EndStrategy::Any,
                }),
                content: ContentSelection::default(),
                events: Events {
                    popup: Some(event(10.0)),
                    ..Default::default()
                },
                movement: None,
                mitosis: None,
                on_enter: Default::default(),
                prompt: Default::default(),
            };
            let second = Stage {
                id: "second".into(),
                label: "Second".into(),
                end: None,
                content: ContentSelection::default(),
                events: Events {
                    popup: Some(event(2.0)),
                    ..Default::default()
                },
                movement: None,
                mitosis: None,
                on_enter: Default::default(),
                prompt: Default::default(),
            };
            let experience = Experience {
                timeline: Timeline {
                    stages: vec![first, second],
                    transitions: vec![Transition {
                        id: "edge".into(),
                        from_stage: "first".into(),
                        to_stage: "second".into(),
                        duration_seconds: 1.0,
                        easing: Easing::Linear,
                        affected: vec![TransitionCategory::PopupInterval],
                    }],
                },
                label: None,
            };
            let mut harness = timeline_harness(&timeline_only_sources_initialized(), experience);
            harness.run_entrypoint("main.lua").unwrap();

            harness.advance(Duration::from_millis(900)).await;
            assert_eq!(
                harness.eval_number("require('timeline').anchors().popup"),
                10.0
            );
            harness.advance(Duration::from_millis(600)).await;
            let halfway = harness.eval_number("require('timeline').anchors().popup");
            assert!(
                (5.5..=6.5).contains(&halfway),
                "halfway value was {halfway}"
            );
            assert_eq!(
                harness.eval_string("require('timeline').phase()"),
                "transition"
            );
            harness.advance(Duration::from_millis(600)).await;
            assert_eq!(
                harness.eval_number("require('timeline').anchors().popup"),
                2.0
            );
            assert_eq!(
                harness.eval_number("require('timeline').stage_index()"),
                2.0
            );
        })
        .await;
}

/// A popup-count trigger reached well before its level's `at_seconds` advances immediately --
/// no waiting for the time timer, and no stepping through intermediate levels (there are none
/// here to step through, but see the order-independence test below for that half).
#[tokio::test(start_paused = true)]
async fn experience_timeline_popup_count_trigger_advances_before_time_elapses() {
    LocalSet::new()
        .run_until(async {
            let experience = levels_to_experience(vec![
                Level {
                    anchors: FrequencyAnchors {
                        popup: Some(1.0),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Level {
                    at_seconds: 1_000.0, // never reached within this test
                    at_popups: Some(3),
                    anchors: FrequencyAnchors {
                        popup: Some(2.0),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ]);
            // No `.init()` -- this test isolates popup-count-driven advancement from any
            // `at_seconds` timer.
            let mut harness = timeline_harness(&timeline_only_sources(), experience);
            harness.run_entrypoint("main.lua").unwrap();

            assert_eq!(
                harness.eval_number(
                    "(function() \
                        local t = require('timeline') \
                        t.on_popup_spawned() \
                        t.on_popup_spawned() \
                        return t.anchors().popup \
                    end)()"
                ),
                1.0,
                "two popups: threshold of three not yet reached"
            );

            assert_eq!(
                harness.eval_number(
                    "(function() \
                        local t = require('timeline') \
                        t.on_popup_spawned() \
                        return t.anchors().popup \
                    end)()"
                ),
                2.0,
                "third popup crosses the threshold"
            );
        })
        .await;
}

/// Rule 6, made structural: a level with an `at_popups` threshold still advances via
/// `at_seconds` even when nothing ever calls `on_popup_spawned()` (the real-world case: the
/// user disabled every popup-spawning media type). The design can't stall.
#[tokio::test(start_paused = true)]
async fn experience_timeline_rule6_time_fallback_when_popups_never_spawn() {
    LocalSet::new()
        .run_until(async {
            let experience = levels_to_experience(vec![
                Level::default(),
                Level {
                    at_seconds: 2.0,
                    at_popups: Some(1_000_000), // never reached -- popups never spawn
                    anchors: FrequencyAnchors {
                        popup: Some(9.0),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ]);
            let mut harness = timeline_harness(&timeline_only_sources_initialized(), experience);
            harness.run_entrypoint("main.lua").unwrap();

            harness.advance(Duration::from_millis(2100)).await;
            assert_eq!(
                harness.eval_number("require('timeline').anchors().popup"),
                9.0
            );
        })
        .await;
}

/// Levels are fully independent snapshots, not deltas relative to the previous level: jumping
/// straight from the baseline to level 3 (skipping level 2's own threshold) produces identical
/// params to passing through level 2 first -- see `Level`'s doc comment.
#[tokio::test(start_paused = true)]
async fn experience_timeline_order_independence_jumping_directly_matches_sequential() {
    LocalSet::new()
        .run_until(async {
            fn two_level_experience() -> Experience {
                levels_to_experience(vec![
                    Level::default(), // baseline
                    Level {
                        at_seconds: 1_000.0,
                        at_popups: Some(2),
                        anchors: FrequencyAnchors {
                            popup: Some(2.0),
                            ..Default::default()
                        },
                        tags: Some(vec!["a".to_string()]),
                        ..Default::default()
                    },
                    Level {
                        at_seconds: 1_000.0,
                        at_popups: Some(5),
                        anchors: FrequencyAnchors {
                            popup: Some(5.0),
                            ..Default::default()
                        },
                        tags: Some(vec!["b".to_string()]),
                        ..Default::default()
                    },
                ])
            }

            let probe = "(function() \
                local t = require('timeline') \
                return t.anchors().popup * 1000 + #(t.tags() or {}) \
            end)()";

            // Sequential: reach level 1 (2 popups), observe it, then continue to level 2.
            let mut sequential = timeline_harness(&timeline_only_sources(), two_level_experience());
            sequential.run_entrypoint("main.lua").unwrap();
            sequential.eval_number(
                "(function() local t = require('timeline') t.on_popup_spawned() t.on_popup_spawned() return 1 end)()",
            );
            assert_eq!(sequential.eval_number(probe), 2000.0 + 1.0, "level 1 reached");
            sequential.eval_number(
                "(function() local t = require('timeline') for i=1,3 do t.on_popup_spawned() end return 1 end)()",
            );
            let sequential_final = sequential.eval_number(probe);

            // Direct jump: five popups in one batch, never separately observing level 1.
            let mut direct = timeline_harness(&timeline_only_sources(), two_level_experience());
            direct.run_entrypoint("main.lua").unwrap();
            let direct_final = direct.eval_number(
                &format!(
                    "(function() \
                        local t = require('timeline') \
                        for i=1,5 do t.on_popup_spawned() end \
                        return {probe} \
                    end)()"
                ),
            );

            assert_eq!(sequential_final, 5000.0 + 1.0, "level 2's own absolute params");
            assert_eq!(
                direct_final, sequential_final,
                "jumping straight to level 2 must match passing through level 1 first"
            );
        })
        .await;
}

/// Wallpaper is the one mode parameter that needs *push* semantics (rule 3): it must reapply
/// when a level's effective override actually changes, but never redundantly for an unchanged
/// value -- otherwise every level transition would re-randomize/flicker the wallpaper even when
/// that level doesn't touch it.
#[tokio::test(start_paused = true)]
async fn experience_timeline_wallpaper_override_reapplies_only_when_changed() {
    LocalSet::new()
        .run_until(async {
            fn level_at(at_seconds: f64, wallpaper: Option<u64>) -> Level {
                Level {
                    at_seconds,
                    wallpaper,
                    ..Default::default()
                }
            }

            let experience = levels_to_experience(vec![
                Level::default(),                   // baseline: no wallpaper override
                level_at(1.0, Some(FIXTURE_MEDIA)), // baseline -> pic.avif: applies
                level_at(2.0, None), // pic.avif -> baseline (unset): no-ops internally
                level_at(3.0, Some(FIXTURE_MEDIA)), // baseline -> pic.avif again: applies
                level_at(4.0, Some(FIXTURE_MEDIA)), // unchanged: must not reapply
            ]);

            let owned = wrapped_experience_mode_sources(WALLPAPER_CAPTURE_WRAP);
            let sources = as_str_sources(&owned);

            let mut config = base_experience_mode_config();
            config.insert("images_enabled".to_string(), OptionValue::Boolean(false));

            let mut harness = Harness::with_pack_and_experience(
                &sources,
                pack_fixture(true),
                Content::default(),
                experience,
                config,
                all_capabilities(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(4100)).await;

            assert_eq!(
                harness.eval_number("SET_COUNT"),
                2.0,
                "two genuine wallpaper changes; the no-op-to-unset and repeat-same-value \
                 transitions must not add to this"
            );
        })
        .await;
}

/// `max_popups` (a class-1 user cap) stays authoritative even when a later level's own anchor
/// is far faster than the baseline's -- caps are "clamps applied after everything else"
/// (`behaviour-design/default-mode.md`, Ownership), never something a design can spawn its way
/// past.
#[tokio::test(start_paused = true)]
async fn experience_timeline_max_popups_still_clamps_when_level_raises_rate() {
    LocalSet::new()
        .run_until(async {
            let experience = levels_to_experience(vec![
                Level {
                    anchors: FrequencyAnchors {
                        popup: Some(0.1),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Level {
                    at_seconds: 0.05,
                    anchors: FrequencyAnchors {
                        popup: Some(0.001),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ]);

            let mut config = base_experience_mode_config();
            config.insert("max_popups".to_string(), OptionValue::Integer(2));

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
            harness.advance(Duration::from_secs(2)).await;

            let spawn_count = harness
                .recorded()
                .iter()
                .filter(|r| matches!(r, Recorded::SpawnImage { .. }))
                .count();
            assert_eq!(
                spawn_count, 2,
                "the cap, not the (much faster) modulated rate, wins"
            );
        })
        .await;
}
