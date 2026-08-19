use std::collections::HashSet;

use shared::behaviour::{
    CountScope, EventCountCondition, EventKind, EventSchedule, Interval, MediaSlot,
};
use shared::read_pack::RecommendedMode;
use shared::tags::NON_POPUP_TAG;

use crate::model::WarningKind;
use crate::source::DirSource;

use super::*;

/// The source file the converter wants in `slot`.
///
/// Slots hold media ids, which only exist once a file has been imported, so the converter
/// reports what each one is waiting on rather than filling it -- see
/// `ConversionOutput::media_references`.
fn slot_source(output: &ConversionOutput, slot: MediaSlot) -> Option<&str> {
    output
        .media_references
        .iter()
        .find(|(candidate, _)| *candidate == slot)
        .map(|(_, name)| name.as_str())
}

fn stage_slot_source(output: &ConversionOutput, index: usize) -> Option<&str> {
    let stages = &output.behaviour.experience.as_ref()?.timeline.stages;
    slot_source(
        output,
        MediaSlot::StageWallpaper {
            stage: stages.get(index)?.id.clone(),
        },
    )
}

/// Every anchor `EventSchedule` the converter produces is a `Interval::Fixed` (see
/// `schedule`) -- unwraps straight to the seconds value, mirroring the old schema's plain
/// `Option<f64>` anchor fields for test readability.
fn anchor_seconds(schedule: &Option<EventSchedule>) -> Option<f64> {
    schedule.as_ref().map(|s| match s.interval {
        Interval::Fixed { seconds } => seconds,
        Interval::Random { .. } => panic!("expected a fixed interval"),
    })
}

fn source_with(files: &[(&str, &str)]) -> (tempfile::TempDir, DirSource) {
    let dir = tempfile::tempdir().unwrap();
    for (name, contents) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }
    let source = DirSource::new(dir.path());
    (dir, source)
}

#[test]
fn metadata_falls_back_to_unnamed_pack() {
    let (_dir, source) = source_with(&[]);
    let output = convert(&source);
    assert_eq!(output.metadata.name, "Unnamed Pack");
}

#[test]
fn one_content_group_per_mood_with_unique_ids() {
    let (_dir, source) = source_with(&[(
        "index.json",
        r#"{"moods": [{"mood": "Guitar"}, {"mood": "guitar!"}]}"#,
    )]);
    let output = convert(&source);
    let ids: Vec<&str> = output
        .behaviour
        .content
        .content_groups
        .iter()
        .map(|g| g.id.as_str())
        .collect();
    assert_eq!(ids, vec!["guitar", "guitar-2"]);
    assert!(output.behaviour.content.content_groups[0].enabled_by_default);
}

#[test]
fn discovers_and_tags_media_by_directory_and_mood() {
    let (_dir, source) = source_with(&[
        (
            "index.json",
            r#"{"moods": [{"mood": "vanilla", "media": ["a.png"]}]}"#,
        ),
        ("img/a.png", "a"),
        ("img/untagged.png", "u"),
    ]);
    let output = convert(&source);
    let a = output
        .media
        .iter()
        .find(|m| m.suggested_name == "a.png")
        .unwrap();
    assert_eq!(a.tags, vec!["vanilla".to_string()]);
    assert_eq!(a.source_path, "img/a.png");
    let untagged = output
        .media
        .iter()
        .find(|m| m.suggested_name == "untagged.png")
        .unwrap();
    assert!(untagged.tags.is_empty());
}

#[test]
fn missing_referenced_media_file_warns() {
    let (_dir, source) = source_with(&[(
        "index.json",
        r#"{"moods": [{"mood": "vanilla", "media": ["missing.png"]}]}"#,
    )]);
    let output = convert(&source);
    assert!(
        output
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnreadableMediaFile)
    );
}

/// Packs drive Edgeware's wallpaper rotation by giving the wallpapers a mood of their own, so
/// the files a `media_moods` entry names are routinely in the pack root rather than under
/// `img/vid/aud`. They convert -- as the wallpaper slot and as the timeline's stage
/// wallpapers -- so reporting them as missing named every wallpaper in the pack as lost.
#[test]
fn a_mood_naming_media_converted_as_scenery_does_not_warn() {
    let (_dir, source) = source_with(&[
        ("img/a.png", "a"),
        ("wallpaper.png", "w1"),
        ("wallpaper2.png", "w2"),
        ("loading_splash.png", "s"),
        (
            "media.json",
            r#"{"wallpapers": ["wallpaper.png", "wallpaper2.png", "loading_splash.png"],
                "vanilla": ["a.png"]}"#,
        ),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {"1": "wallpaper.png", "2": "wallpaper2.png"}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    assert!(
        !output
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnreadableMediaFile),
        "{:#?}",
        output.warnings
    );
}

/// Windows again: a `media.json` naming `Image1.PNG` for a file called `image1.png` is a
/// disagreement its author cannot see. Read case-sensitively the file converts untagged, into
/// no content group at all, and nothing says so.
#[test]
fn a_mood_finds_its_media_across_a_casing_mismatch() {
    let (_dir, source) = source_with(&[
        ("img/image1.png", "a"),
        ("media.json", r#"{"vanilla": ["Image1.PNG"]}"#),
    ]);
    let output = convert(&source);

    let tagged = output
        .media
        .iter()
        .find(|m| m.suggested_name == "image1.png")
        .expect("the file converts");
    assert_eq!(tagged.tags, vec!["vanilla".to_string()]);
    assert!(
        !output
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnreadableMediaFile),
        "{:#?}",
        output.warnings
    );
}

/// ...but a mood naming a file that really is absent still does.
#[test]
fn a_mood_naming_media_that_is_nowhere_still_warns() {
    let (_dir, source) = source_with(&[
        ("img/a.png", "a"),
        ("media.json", r#"{"vanilla": ["a.png", "gone.png"]}"#),
    ]);
    let output = convert(&source);

    let messages: Vec<&str> = output
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::UnreadableMediaFile)
        .map(|w| w.message.as_str())
        .collect();
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(messages[0].contains("gone.png"), "{messages:?}");
}

#[test]
fn wallpaper_and_splash_fill_their_slots_and_are_kept_out_of_popups() {
    let (_dir, source) = source_with(&[("wallpaper.png", "w"), ("loading_splash.gif", "s")]);
    let output = convert(&source);
    assert_eq!(
        slot_source(&output, MediaSlot::Wallpaper),
        Some("wallpaper.png")
    );
    assert_eq!(
        slot_source(&output, MediaSlot::Splash),
        Some("loading_splash.gif")
    );
    assert!(
        output
            .media
            .iter()
            .any(|m| m.source_path == "wallpaper.png" && m.tags == vec![NON_POPUP_TAG.to_string()])
    );
    assert!(output.media.iter().any(
        |m| m.source_path == "loading_splash.gif" && m.tags == vec![NON_POPUP_TAG.to_string()]
    ));
}

#[test]
fn wallpaper_slot_tolerates_casing_and_extension() {
    // A real pack (FeetWare V2) ships exactly this: capital W, `.jpg`, plus numbered spares.
    let (_dir, source) = source_with(&[
        ("Wallpaper.jpg", "w"),
        ("Wallpaper (2).jpg", "spare"),
        ("Wallpaper (3).png", "spare"),
    ]);
    let output = convert(&source);
    assert_eq!(
        slot_source(&output, MediaSlot::Wallpaper),
        Some("Wallpaper.jpg")
    );
    // The spares aren't the pick and aren't imported -- there is one slot.
    assert!(
        !output
            .media
            .iter()
            .any(|m| m.source_path.starts_with("Wallpaper ("))
    );
}

#[test]
fn wallpaper_slot_prefers_edgewares_own_filename() {
    let (_dir, source) = source_with(&[("Wallpaper.jpg", "w"), ("wallpaper.png", "w")]);
    let output = convert(&source);
    assert_eq!(
        slot_source(&output, MediaSlot::Wallpaper),
        Some("wallpaper.png")
    );
}

#[test]
fn wallpaper_slot_ignores_non_image_extensions() {
    let (_dir, source) = source_with(&[("wallpaper.mp4", "w"), ("wallpaper.gif", "w")]);
    let output = convert(&source);
    assert_eq!(output.behaviour.content.wallpaper, None);
}

#[test]
fn corruption_level_reusing_the_primary_wallpaper_imports_it_once() {
    let (_dir, source) = source_with(&[
        ("Wallpaper.jpg", "w"),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {"1": "Wallpaper.jpg"}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);
    assert_eq!(
        output
            .media
            .iter()
            .filter(|m| m.source_path == "Wallpaper.jpg")
            .count(),
        1
    );
}

/// Packs are authored on Windows, where `corruption.json` naming `wallpaper.png` finds
/// `Wallpaper.png` and the disagreement never surfaces. Dropping those levels' wallpapers on a
/// case-sensitive read would lose a feature Lewdware does support, over spelling.
#[test]
fn corruption_wallpapers_resolve_across_a_casing_mismatch() {
    let (_dir, source) = source_with(&[
        ("img/a.png", "a"),
        ("Wallpaper.png", "w1"),
        ("Wallpaper2.png", "w2"),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {"1": "wallpaper.png", "2": "wallpaper2.png"}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    assert!(
        !output
            .warnings
            .iter()
            .any(|w| w.message.contains("wasn't found")),
        "{:#?}",
        output.warnings
    );
    assert_eq!(stage_slot_source(&output, 0), Some("Wallpaper.png"));
    assert_eq!(stage_slot_source(&output, 1), Some("Wallpaper2.png"));
    // The stage reference and `discover_media`'s primary agree, so it is imported once.
    assert_eq!(
        output
            .media
            .iter()
            .filter(|m| m.source_path == "Wallpaper.png")
            .count(),
        1
    );
}

/// An exact match still wins, so a root holding both spellings resolves each to itself rather
/// than collapsing them onto whichever the listing happens to reach first.
#[test]
fn an_exactly_matching_wallpaper_wins_over_a_casing_variant() {
    let (_dir, source) = source_with(&[
        ("Wallpaper2.png", "upper"),
        ("wallpaper2.png", "lower"),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {"1": "wallpaper2.png"}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    assert_eq!(stage_slot_source(&output, 0), Some("wallpaper2.png"));
}

/// Import order is what the author waits on: the few files the Content and Timeline tabs
/// name go first, ahead of a pack's thousands of popups.
#[test]
fn media_the_slots_are_waiting_on_is_imported_first() {
    let (_dir, source) = source_with(&[
        ("img/a.png", "a"),
        ("img/b.png", "b"),
        ("wallpaper.png", "w"),
        ("loading_splash.gif", "s"),
        ("level2.png", "l2"),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {"1": "wallpaper.png", "2": "level2.png"}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    // Deduplicated: one file may fill several slots, and the pack wallpaper reused by a stage
    // is Edgeware's ordinary case.
    let referenced: HashSet<&str> = output
        .media_references
        .iter()
        .map(|(_, name)| name.as_str())
        .collect();
    let ordered: Vec<&str> = output
        .media
        .iter()
        .map(|m| m.suggested_name.as_str())
        .collect();
    let first_popup = ordered
        .iter()
        .position(|name| !referenced.contains(name))
        .expect("the fixture has popup media");

    for name in &ordered[..first_popup] {
        assert!(referenced.contains(name), "{name} is not referenced");
    }
    assert_eq!(
        first_popup,
        referenced.len(),
        "every referenced file should come first, got {ordered:?}"
    );
    // Untouched within each group -- discovery order still reads as discovery order.
    assert_eq!(&ordered[first_popup..], &["a.png", "b.png"]);
}

/// Neither hypno directory is imported: there is no subliminals feature for a spiral to be
/// drawn by, and taking one in as an ordinary popup would spawn a window full of
/// half-transparent spiral. Ordinary popup media is untouched by any of this.
#[test]
fn hypno_and_legacy_subliminal_media_is_skipped_with_a_warning() {
    for dir in ["hypno", "subliminals"] {
        let (_dir, source) =
            source_with(&[("img/a.png", "a"), (&format!("{dir}/spiral.gif"), "spiral")]);
        let output = convert(&source);
        assert!(
            !output
                .media
                .iter()
                .any(|m| m.suggested_name == "spiral.gif"),
            "{dir}/ should not be imported at all"
        );
        assert!(
            output.warnings.iter().any(|warning| warning.kind
                == WarningKind::UnsupportedFeatureDropped
                && warning.message.contains("subliminal")),
            "{dir}/ should be reported as skipped"
        );
        let popup = output
            .media
            .iter()
            .find(|m| m.suggested_name == "a.png")
            .unwrap();
        assert!(popup.tags.is_empty());
    }
}

/// Scenery isn't popup content, so a pack made only of it has no popup pace to set --
/// otherwise the timeline promises popups the pack can't spawn.
#[test]
fn popup_anchor_ignores_non_popup_media() {
    let (_dir, source) = source_with(&[("wallpaper.png", "w")]);
    let output = convert(&source);
    let popup_anchor =
        output.behaviour.experience.as_ref().map(|experience| {
            anchor_seconds(&experience.timeline.stages[0].events.popup).is_some()
        });
    assert_ne!(popup_anchor, Some(true));

    let (_dir2, source2) = source_with(&[("wallpaper.png", "w"), ("img/a.png", "a")]);
    let output2 = convert(&source2);
    let experience = output2.behaviour.experience.unwrap();
    assert!(anchor_seconds(&experience.timeline.stages[0].events.popup).is_some());
}

#[test]
fn popup_close_folds_into_captions_only_when_authored() {
    let (_dir, source) = source_with(&[("index.json", r#"{"default": {}}"#)]);
    let output = convert(&source);
    assert!(output.behaviour.content.captions.is_empty());

    let (_dir2, source2) =
        source_with(&[("index.json", r#"{"default": {"popupClose": "Close me"}}"#)]);
    let output2 = convert(&source2);
    assert_eq!(output2.behaviour.content.captions.len(), 1);
    assert_eq!(output2.behaviour.content.captions[0].text, "Close me");
    assert!(output2.behaviour.content.captions[0].tags.is_empty());
}

#[test]
fn script_discord_corruption_config_presence_warn() {
    let (_dir, source) = source_with(&[
        ("script.lua", "-- edgeware script"),
        ("discord.dat", "text\nimage"),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {}, "config": {}}"#,
        ),
        ("config.json", r#"{"someKey": 1}"#),
    ]);
    let output = convert(&source);
    for kind in [
        WarningKind::ScriptSkipped,
        WarningKind::DiscordSkipped,
        WarningKind::CorruptionNotConverted,
        WarningKind::ConfigNotConverted,
    ] {
        assert!(
            output.warnings.iter().any(|w| w.kind == kind),
            "expected a warning of kind {kind:?}"
        );
    }
}

#[test]
fn empty_config_json_does_not_warn() {
    let (_dir, source) = source_with(&[("config.json", "{}")]);
    let output = convert(&source);
    assert!(
        !output
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::ConfigNotConverted)
    );
}

#[test]
fn icon_is_surfaced_as_a_path_hint_only() {
    let (_dir, source) = source_with(&[("icon.ico", "fake ico bytes")]);
    let output = convert(&source);
    assert_eq!(output.icon.as_deref(), Some("icon.ico"));
}

#[test]
fn no_config_or_corruption_recommends_sandbox_with_no_experience_section() {
    let (_dir, source) = source_with(&[]);
    let output = convert(&source);
    assert_eq!(
        output.metadata.recommended_mode,
        Some(RecommendedMode::Sandbox)
    );
    assert_eq!(output.behaviour.experience, None);
}

#[test]
fn config_json_pacing_alone_populates_anchors_but_still_recommends_sandbox() {
    let (_dir, source) = source_with(&[(
        "config.json",
        r#"{"delay": 5000, "popupMod": 100, "vidMod": 0, "webMod": 0}"#,
    )]);
    let output = convert(&source);
    let experience = output.behaviour.experience.as_ref().unwrap();
    // 100% chance every 5000ms tick -> one event every 5 seconds.
    assert_eq!(
        anchor_seconds(&experience.timeline.stages[0].events.popup),
        Some(5.0)
    );
    assert_eq!(
        output.metadata.recommended_mode,
        Some(RecommendedMode::Sandbox)
    );
    // No corruption.json -> just the one baseline stage, no escalation.
    assert_eq!(experience.timeline.stages.len(), 1);
}

#[test]
fn popup_anchor_sums_image_and_video_chance() {
    let (_dir, source) = source_with(&[(
        "config.json",
        r#"{"delay": 1000, "popupMod": 50, "vidMod": 50}"#,
    )]);
    let output = convert(&source);
    // 100% combined chance every 1000ms -> one event/sec.
    assert_eq!(
        anchor_seconds(
            &output
                .behaviour
                .experience
                .as_ref()
                .unwrap()
                .timeline
                .stages[0]
                .events
                .popup
        ),
        Some(1.0)
    );
}

#[test]
fn zero_chance_key_leaves_that_anchor_absent() {
    let (_dir, source) = source_with(&[("config.json", r#"{"webMod": 0}"#)]);
    let output = convert(&source);
    assert!(output.behaviour.experience.is_none());
}

#[test]
fn content_presence_defaults_an_anchor_on_when_no_config_signal_exists_anywhere() {
    // No config.json, no corruption.json -- webMod is never mentioned anywhere. A pack with
    // real web-link content should still convert with the web anchor assumed on at a sensible
    // default pace, rather than silently defaulting to Edgeware's literal (but here clearly
    // unintended) "off".
    let (_dir, source) = source_with(&[(
        "index.json",
        r#"{"default": {"web": ["https://example.com"], "webArgs": [[]]}}"#,
    )]);
    let output = convert(&source);
    assert!(!output.behaviour.content.web_links.is_empty());
    assert_eq!(
        anchor_seconds(
            &output.behaviour.experience.unwrap().timeline.stages[0]
                .events
                .web
        ),
        Some(DEFAULT_WEB_PERIOD_SECONDS)
    );
}

#[test]
fn no_content_and_no_config_signal_leaves_the_anchor_absent() {
    // Same as above but with no web-link content at all -- nothing implies the feature should
    // be on, so it correctly stays absent.
    let (_dir, source) = source_with(&[("index.json", r#"{"default": {"captions": ["hi"]}}"#)]);
    let output = convert(&source);
    assert!(output.behaviour.content.web_links.is_empty());
    assert!(output.behaviour.experience.is_none());
}

#[test]
fn missing_delay_falls_back_to_edgewares_own_default() {
    let (_dir, source) = source_with(&[("config.json", r#"{"promptMod": 100}"#)]);
    let output = convert(&source);
    // 100% chance every (default) 5000ms -> one event every 5 seconds.
    assert_eq!(
        anchor_seconds(
            &output
                .behaviour
                .experience
                .as_ref()
                .unwrap()
                .timeline
                .stages[0]
                .events
                .prompt
        ),
        Some(5.0)
    );
}

#[test]
fn corruption_timeline_recommends_experience_with_cumulative_tags() {
    let (_dir, source) = source_with(&[(
        "corruption.json",
        r#"{
            "moods": {
                "1": {"add": ["angel", "apple"], "remove": []},
                "2": {"add": ["globe"], "remove": ["apple"]}
            },
            "wallpapers": {},
            "config": {}
        }"#,
    )]);
    let output = convert(&source);
    assert_eq!(
        output.metadata.recommended_mode,
        Some(RecommendedMode::Experience)
    );
    let timeline = &output.behaviour.experience.as_ref().unwrap().timeline;
    assert_eq!(timeline.stages.len(), 2);
    // Stage 1 is applied immediately (Edgeware applies it at session start, not after one
    // corruption_time interval) -- see `build_timeline`. It also doubles as the new schema's
    // baseline stage, so it has no trigger of its own; its own `end` instead encodes when
    // stage 2 is reached (default corruption_time, config.json absent -> Edgeware's own 60s
    // default).
    assert_eq!(
        timeline.stages[0].end.as_ref().unwrap().duration_seconds,
        Some(60.0)
    );
    assert_eq!(
        timeline.stages[0].content.tags,
        Some(vec!["angel".to_string(), "apple".to_string()])
    );
    assert!(timeline.stages[1].end.is_none());
    // "apple" removed, "globe" added -- cumulative, not a replacement.
    assert_eq!(
        timeline.stages[1].content.tags,
        Some(vec!["angel".to_string(), "globe".to_string()])
    );
}

#[test]
fn popup_trigger_derives_at_popups_from_corruption_popups() {
    let (_dir, source) = source_with(&[
        (
            "corruption.json",
            r#"{"moods": {"1": {"add": [], "remove": []}, "2": {"add": [], "remove": []}}, "wallpapers": {}, "config": {}}"#,
        ),
        (
            "config.json",
            r#"{"corruptionTrigger": "Popup", "corruptionPopups": 10}"#,
        ),
    ]);
    let output = convert(&source);
    let timeline = output.behaviour.experience.unwrap().timeline;
    // The popup-count trigger for reaching stage 1 (from the baseline stage 0) is encoded
    // as stage 0's own `end` condition -- there's nothing before stage 0 to derive a trigger
    // for, matching the old schema's "ignored for the baseline level" rule.
    assert_eq!(
        timeline.stages[0].end.as_ref().unwrap().event_count,
        Some(EventCountCondition {
            event: EventKind::Popup,
            count: 10,
            scope: CountScope::Session,
        })
    );
}

#[test]
fn timed_trigger_never_sets_at_popups() {
    let (_dir, source) = source_with(&[(
        "corruption.json",
        r#"{"moods": {"1": {"add": [], "remove": []}, "2": {"add": [], "remove": []}}, "wallpapers": {}, "config": {}}"#,
    )]);
    let output = convert(&source);
    let timeline = output.behaviour.experience.unwrap().timeline;
    assert!(
        timeline
            .stages
            .iter()
            .all(|s| s.end.as_ref().is_none_or(|end| end.event_count.is_none()))
    );
}

#[test]
fn launch_trigger_warns_and_still_produces_a_time_based_timeline() {
    let (_dir, source) = source_with(&[
        (
            "corruption.json",
            r#"{"moods": {"1": {"add": ["a"], "remove": []}}, "wallpapers": {}, "config": {}}"#,
        ),
        ("config.json", r#"{"corruptionTrigger": "Launch"}"#),
    ]);
    let output = convert(&source);
    assert!(
        output
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnsupportedFeatureDropped)
    );
    assert!(
        !output
            .behaviour
            .experience
            .unwrap()
            .timeline
            .stages
            .is_empty()
    );
}

#[test]
fn per_level_config_override_of_a_recognized_anchor_key_converts_not_warns() {
    // promptMod is now a recognized anchor-affecting key (see LEVEL_ANCHOR_CONFIG_KEYS) --
    // it should be converted into a real per-level anchor change, not warned about and
    // dropped the way an unrecognized per-level key still is.
    let (_dir, source) = source_with(&[(
        "corruption.json",
        r#"{"moods": {}, "wallpapers": {}, "config": {"1": {"promptMod": 0}}}"#,
    )]);
    let output = convert(&source);
    assert!(!output.warnings.iter().any(
        |w| w.kind == WarningKind::UnsupportedFeatureDropped && w.message.contains("promptMod")
    ));
}

#[test]
fn per_level_config_override_of_an_unrecognized_key_still_warns_and_is_dropped() {
    let (_dir, source) = source_with(&[(
        "corruption.json",
        r#"{"moods": {}, "wallpapers": {}, "config": {"1": {"promptMistakes": 10}}}"#,
    )]);
    let output = convert(&source);
    assert!(
        output
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnsupportedFeatureDropped
                && w.message.contains("promptMistakes"))
    );
}

#[test]
fn per_level_promptmod_overrides_convert_into_real_per_level_prompt_anchors() {
    // Mirrors the real Edgeware++ Test Pack V2's own corruption.json: prompts explicitly off
    // at the baseline, explicitly on from level 4 onward -- a level's own promptMod override
    // must be reflected in that level's (and every subsequent level's, until changed again)
    // `anchors.prompt`, not silently dropped the way the old single-scalar-modifier schema
    // had to.
    let (_dir, source) = source_with(&[(
        "corruption.json",
        r#"{
            "moods": {"1": {"add": [], "remove": []}, "2": {"add": [], "remove": []},
                      "3": {"add": [], "remove": []}, "4": {"add": [], "remove": []}},
            "wallpapers": {},
            "config": {"1": {"promptMod": 0}, "4": {"promptMod": 10}}
        }"#,
    )]);
    let output = convert(&source);
    let timeline = &output.behaviour.experience.unwrap().timeline;
    assert_eq!(timeline.stages.len(), 4);
    assert_eq!(
        anchor_seconds(&timeline.stages[0].events.prompt),
        None,
        "off at the baseline"
    );
    assert_eq!(
        anchor_seconds(&timeline.stages[1].events.prompt),
        None,
        "still off (carried forward)"
    );
    assert_eq!(
        anchor_seconds(&timeline.stages[2].events.prompt),
        None,
        "still off (carried forward)"
    );
    assert!(
        timeline.stages[3].events.prompt.is_some(),
        "on from stage 4 (index 3) onward"
    );
}

#[test]
fn level_wallpaper_reuses_the_primary_wallpaper_without_duplicate_media() {
    let (_dir, source) = source_with(&[
        ("wallpaper.png", "w"),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {"1": "wallpaper.png"}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);
    assert_eq!(stage_slot_source(&output, 0), Some("wallpaper.png"));
    assert_eq!(
        output
            .media
            .iter()
            .filter(|m| m.source_path == "wallpaper.png")
            .count(),
        1
    );
}

#[test]
fn level_wallpaper_other_than_primary_gets_its_own_media_entry() {
    let (_dir, source) = source_with(&[
        ("wallpaper2.png", "w2"),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {"1": "wallpaper2.png", "2": "wallpaper2.png"}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);
    let stage_count = output
        .behaviour
        .experience
        .as_ref()
        .unwrap()
        .timeline
        .stages
        .len();
    for index in 0..stage_count {
        assert_eq!(stage_slot_source(&output, index), Some("wallpaper2.png"));
    }
    // Two levels naming the same file is one import, not two.
    let entries: Vec<_> = output
        .media
        .iter()
        .filter(|m| m.source_path == "wallpaper2.png")
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].tags, vec![NON_POPUP_TAG.to_string()]);
}

#[test]
fn no_synthetic_tags_are_minted_for_wallpapers() {
    // The whole point of references: a pack's wallpapers land as plain names, and nothing
    // resembling a mechanical `wallpaper`/`wallpaper-<n>` tag reaches the pack's namespace.
    let (_dir, source) = source_with(&[
        ("wallpaper.png", "w"),
        ("loading_splash.gif", "s"),
        ("wallpaper2.png", "w2"),
        (
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {"1": "wallpaper.png", "2": "wallpaper2.png"}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    for item in &output.media {
        for tag in &item.tags {
            assert!(
                tag == NON_POPUP_TAG || !tag.starts_with("wallpaper") && tag != "splash",
                "unexpected mechanical tag {tag:?} on {}",
                item.source_path
            );
        }
    }
    assert_eq!(stage_slot_source(&output, 0), Some("wallpaper.png"));
    assert_eq!(stage_slot_source(&output, 1), Some("wallpaper2.png"));
}

#[test]
fn missing_level_wallpaper_file_warns_and_leaves_no_override() {
    let (_dir, source) = source_with(&[(
        "corruption.json",
        r#"{"moods": {}, "wallpapers": {"1": "missing.png"}, "config": {}}"#,
    )]);
    let output = convert(&source);
    assert!(
        output
            .warnings
            .iter()
            .any(|w| w.kind == WarningKind::UnreadableMediaFile)
    );
    let timeline = output.behaviour.experience.unwrap().timeline;
    assert_eq!(timeline.stages[0].content.wallpaper, None);
}

#[test]
fn moodless_media_alongside_a_timeline_gets_tagged_into_every_level() {
    let (_dir, source) = source_with(&[
        ("img/untagged.png", "u"),
        (
            "corruption.json",
            r#"{"moods": {"1": {"add": ["a"], "remove": []}, "2": {"add": ["b"], "remove": ["a"]}}, "wallpapers": {}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    let untagged = output
        .media
        .iter()
        .find(|m| m.suggested_name == "untagged.png")
        .unwrap();
    assert_eq!(untagged.tags, vec![MOODLESS_TAG.to_string()]);

    let timeline = output.behaviour.experience.unwrap().timeline;
    for stage in &timeline.stages {
        assert!(
            stage
                .content
                .tags
                .as_ref()
                .unwrap()
                .contains(&MOODLESS_TAG.to_string()),
            "expected every stage to keep mood-less media eligible: {stage:?}"
        );
    }
    // Never removed by an unrelated mood change.
    assert!(
        timeline.stages[1]
            .content
            .tags
            .as_ref()
            .unwrap()
            .contains(&MOODLESS_TAG.to_string())
    );
}

#[test]
fn no_moodless_media_means_no_synthetic_tag_anywhere() {
    let (_dir, source) = source_with(&[
        (
            "index.json",
            r#"{"moods": [{"mood": "vanilla", "media": ["a.png"]}]}"#,
        ),
        ("img/a.png", "a"),
        (
            "corruption.json",
            r#"{"moods": {"1": {"add": ["vanilla"], "remove": []}}, "wallpapers": {}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    assert!(
        !output
            .media
            .iter()
            .any(|m| m.tags.contains(&MOODLESS_TAG.to_string()))
    );
    let timeline = output.behaviour.experience.unwrap().timeline;
    assert!(
        !timeline.stages[0]
            .content
            .tags
            .as_ref()
            .unwrap()
            .contains(&MOODLESS_TAG.to_string())
    );
}

#[test]
fn free_tag_returns_base_when_unused_and_suffixes_on_collision() {
    let mut reserved = HashSet::new();
    assert_eq!(free_tag("wallpaper-1", &reserved), "wallpaper-1");

    reserved.insert("wallpaper-1".to_string());
    assert_eq!(free_tag("wallpaper-1", &reserved), "wallpaper-1-2");

    reserved.insert("wallpaper-1-2".to_string());
    assert_eq!(free_tag("wallpaper-1", &reserved), "wallpaper-1-3");
}

#[test]
fn a_mood_inside_the_reserved_namespace_is_escaped_everywhere_it_lands() {
    // A pack whose own mood is literally named `__lewdware-non-popup`. Carried verbatim it
    // would pull that mood's whole media set out of the popup pool -- mechanical meaning the
    // author never asked for. Every place the name lands has to agree on the escaped form,
    // or the level's mood delta stops matching its own media.
    let (_dir, source) = source_with(&[
        (
            "index.json",
            r#"{"moods": [{"mood": "__lewdware-non-popup", "media": ["a.png"]}]}"#,
        ),
        ("img/a.png", "a"),
        (
            "corruption.json",
            r#"{"moods": {"1": {"add": ["__lewdware-non-popup"], "remove": []}}, "wallpapers": {}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    let escaped = "___lewdware-non-popup".to_string();
    let a = output
        .media
        .iter()
        .find(|m| m.suggested_name == "a.png")
        .unwrap();
    assert_eq!(a.tags, vec![escaped.clone()]);
    assert_eq!(
        output.behaviour.content.content_groups[0].tags,
        vec![escaped.clone()]
    );
    let timeline = output.behaviour.experience.unwrap().timeline;
    assert!(
        timeline.stages[0]
            .content
            .tags
            .as_ref()
            .unwrap()
            .contains(&escaped),
        "the level's mood delta must name the escaped tag its media actually carries"
    );
}

#[test]
fn moodless_tag_dodges_a_real_mood_of_the_same_name() {
    // A pack with a real mood literally named `corruption-moodless`, added then removed across
    // the timeline. The synthetic mood-less tag must stay distinct, so removing the real mood
    // doesn't drop the untagged media the synthetic tag exists to protect.
    let (_dir, source) = source_with(&[
        (
            "index.json",
            r#"{"moods": [{"mood": "corruption-moodless", "media": ["a.png"]}]}"#,
        ),
        ("img/a.png", "a"),
        ("img/untagged.png", "u"),
        (
            "corruption.json",
            r#"{"moods": {"1": {"add": ["corruption-moodless"], "remove": []}, "2": {"add": [], "remove": ["corruption-moodless"]}}, "wallpapers": {}, "config": {}}"#,
        ),
    ]);
    let output = convert(&source);

    // Untagged media gets the suffixed synthetic tag; the real mood's media keeps the base name.
    let untagged = output
        .media
        .iter()
        .find(|m| m.suggested_name == "untagged.png")
        .unwrap();
    assert_eq!(untagged.tags, vec!["corruption-moodless-2".to_string()]);
    let a = output
        .media
        .iter()
        .find(|m| m.suggested_name == "a.png")
        .unwrap();
    assert_eq!(a.tags, vec!["corruption-moodless".to_string()]);

    let timeline = output.behaviour.experience.unwrap().timeline;
    let stage_tags = |i: usize| timeline.stages[i].content.tags.clone().unwrap();
    // Stage 1 (index 0) adds the real mood; both tags are present and independent.
    assert!(stage_tags(0).contains(&"corruption-moodless".to_string()));
    assert!(stage_tags(0).contains(&"corruption-moodless-2".to_string()));
    // Stage 2 (index 1) removes the real mood -- but the synthetic tag survives, so the
    // mood-less media stays eligible (the exact failure the guard prevents).
    assert!(!stage_tags(1).contains(&"corruption-moodless".to_string()));
    assert!(stage_tags(1).contains(&"corruption-moodless-2".to_string()));
}
