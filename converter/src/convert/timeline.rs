//! The Corruption timeline: reading Edgeware's per-level config keys, folding them into the
//! cumulative anchor series a `Timeline` wants, and warning about what could not be mapped.

use std::collections::{BTreeSet, HashSet};

use super::*;
use crate::model::{CorruptionLevel, EdgewareIndex, Warning, WarningKind};
use crate::source::PackSource;
use serde_json::{Map, Value};
use shared::behaviour::Content;
use shared::behaviour::{
    ContentSelection, CountScope, Easing, EndStrategy, EventCountCondition, EventKind,
    EventSchedule, Events, Experience, Interval, MediaSlot, Mitosis, Movement, Stage, StageEnd,
    Timeline, Transition,
};
use shared::tags::NON_POPUP_TAG;

/// An intermediate, cumulative-timeline-friendly shape the converter builds internally --
/// mirrors the pre-Transitions "one flat, -contained snapshot per level" schema, which is a
/// far simpler target for `build_timeline`'s cumulative tag/anchor folding than the stage graph
/// (transitions between stages, `end` conditions referencing the *next* stage) is. Converted to
/// real `Stage`/`Transition`s by `levels_to_experience` once every level is resolved.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Level {
    pub(super) at_seconds: f64,
    pub(super) at_popups: Option<u32>,
    pub(super) anchors: FrequencyAnchors,
    pub(super) design: DesignValues,
    pub(super) tags: Option<Vec<String>>,
    pub(super) wallpaper: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct FrequencyAnchors {
    pub(super) popup: Option<f64>,
    pub(super) web: Option<f64>,
    pub(super) notification: Option<f64>,
    pub(super) prompt: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct DesignValues {
    pub(super) movement_speed_min: Option<f64>,
    pub(super) movement_speed_max: Option<f64>,
    pub(super) mitosis_chance: Option<f64>,
    pub(super) mitosis_count: Option<u32>,
}

pub(super) fn schedule(seconds: Option<f64>) -> Option<EventSchedule> {
    seconds.map(|seconds| EventSchedule {
        interval: Interval::Fixed { seconds },
        initial_delay_seconds: None,
        max_concurrent: None,
    })
}

pub(super) fn movement(design: &DesignValues) -> Option<Movement> {
    match (design.movement_speed_min, design.movement_speed_max) {
        (None, None) => None,
        (minimum_speed, maximum_speed) => Some(Movement {
            minimum_speed,
            maximum_speed,
        }),
    }
}

pub(super) fn mitosis(design: &DesignValues) -> Option<Mitosis> {
    match (design.mitosis_chance, design.mitosis_count) {
        (None, None) => None,
        (chance, count) => Some(Mitosis { chance, count }),
    }
}

/// Converts a cumulative `Vec<Level>` (see `Level`'s doc comment) into a real `Experience`:
/// each level becomes a `Stage`, and a level's `at_seconds`/`at_popups` (the trigger for
/// *reaching* it) becomes the *previous* stage's `StageEnd` (the condition for *leaving* it) --
/// the last level never gets an `end`, since there's nothing after it to transition to. One
/// zero-duration, linear, unaffected `Transition` is synthesized per adjacent stage pair, since
/// a cumulative level's values always apply instantly, never interpolated.
pub(super) fn levels_to_experience(levels: Vec<Level>) -> (Experience, Vec<(MediaSlot, String)>) {
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
                    // Every tag here is an Edgeware mood the author wrote, so the stage owns none
                    // of them: renaming a converted stage must not rewrite `succubus`.
                    owned_tag: None,
                    // Filled in by the importer once the file has an id -- see the return value.
                    wallpaper: None,
                    audio: None,
                    audio_random: false,
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
    // Paired back up by position: a stage is built from the level at the same index, so this is
    // the only place that knows which file each stage's slot is waiting on.
    let wallpapers = stages
        .iter()
        .zip(&levels)
        .filter_map(|(stage, level)| {
            level.wallpaper.clone().map(|name| {
                (
                    MediaSlot::StageWallpaper {
                        stage: stage.id.clone(),
                    },
                    name,
                )
            })
        })
        .collect();
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
    // Edgeware's own name for a multi-level progression is "corruption"; presenting the timeline
    // mode under that label keeps converted packs legible to the users they came from. A
    // single-stage timeline has no progression, so it keeps the mode's own name.
    let label = (stages.len() > 1).then(|| "Corruption".to_string());
    (
        Experience {
            timeline: Timeline {
                stages,
                transitions,
            },
            label,
        },
        wallpapers,
    )
}

/// `config.json` keys `resolve_anchor_series`/`resolve_popup_anchor_series` read. Anything else
/// in the pack's `config.json` has no Lewdware equivalent (theme, hibernate, mitosis, drive,
/// scheduler, ... -- see `EdgewarePlusPlus/edgeware/src/config/items.py`) and is silently dropped, per
/// `behaviour-design/edgeware-compat.md`'s "warn + skip the rest".
pub(super) const CONFIG_ANCHOR_KEYS: &[&str] = &[
    "delay",
    "popupMod",
    "vidMod",
    "webMod",
    "notificationChance",
    "promptMod",
];
/// Keys `build_timeline` reads to pace `corruption.json` (only meaningful alongside it, but
/// still a recognized, converted setting rather than a dropped one -- excluded from the
/// "leftover, unmapped" tally same as `CONFIG_ANCHOR_KEYS`). `corruptionMode` (the on/off switch
/// itself) is deliberately *not* here: a present `corruption.json` always converts regardless of
/// whether the pack's `config.json` happened to leave corruption disabled, so the switch itself
/// has nothing to map to and stays counted as dropped.
pub(super) const CONFIG_TIMELINE_KEYS: &[&str] =
    &["corruptionTime", "corruptionPopups", "corruptionTrigger"];
/// Keys Edgeware's own `load_config` filters out before anything sees them (`pack/load.py`) --
/// excluded from the "leftover, unmapped" tally since they were never real settings to begin
/// with.
pub(super) const CONFIG_META_KEYS: &[&str] = &["version", "versionplusplus", "packPath"];

pub(super) fn warn_unmapped_config_keys(config: &Map<String, Value>, warnings: &mut Vec<Warning>) {
    let mut leftover: Vec<&str> = config
        .keys()
        .map(String::as_str)
        .filter(|key| {
            !CONFIG_ANCHOR_KEYS.contains(key)
                && !CONFIG_TIMELINE_KEYS.contains(key)
                && !CONFIG_META_KEYS.contains(key)
        })
        .collect();
    if leftover.is_empty() {
        return;
    }
    leftover.sort_unstable();
    warnings.push(Warning::new(
        WarningKind::ConfigNotConverted,
        format!(
            "config.json's other setting(s) ({}) have no Lewdware equivalent and were dropped",
            leftover.join(", ")
        ),
    ));
}

/// Edgeware's own hardcoded fallback for a setting the pack's `config.json` doesn't mention --
/// see `EdgewarePlusPlus/edgeware/assets/default_config.json`. Used (not the schema-wide
/// "absent means off" convention) because these are genuinely what a user launching this pack in
/// Edgeware would experience; only mapping keys the pack explicitly set would misrepresent the
/// pack's actual behaviour, not just decline to state an opinion.
pub(super) const DEFAULT_DELAY_MS: f64 = 5000.0;
pub(super) const DEFAULT_CORRUPTION_TIME_SECONDS: f64 = 60.0;
pub(super) const DEFAULT_CORRUPTION_POPUPS: f64 = 5.0;
// Edgeware's own `assets/default_config.json`: a pack shipping no config.json at all still runs
// with these chances in real Edgeware, so falling back to 0 here (like every other -- genuinely
// opt-in -- anchor key) would misrepresent the pack, the same reasoning that already justifies
// the three fallback constants above.
pub(super) const DEFAULT_POPUP_MOD: f64 = 100.0;
pub(super) const DEFAULT_VID_MOD: f64 = 10.0;

/// Synthetic tag `build_timeline` assigns to every previously-untagged media file, so a
/// timeline level's `any`-of-tags restriction still includes mood-less media -- see its use site
/// for the full reasoning.
pub(super) const MOODLESS_TAG: &str = "corruption-moodless";

pub(super) fn config_number(config: &Map<String, Value>, key: &str) -> Option<f64> {
    config.get(key).and_then(Value::as_f64)
}

/// Edgeware's shared popup-tick model: every `delay` milliseconds, each feature independently
/// rolls a `chance` percent chance to fire (see `EdgewarePlusPlus/edgeware/src/main_edgeware.py`'s
/// `main()` and `roll_targets`). Converted to lewdware's "seconds between events" convention:
/// `events/sec = (chance / 100) * (1000 / delay_ms)`, inverted. `chance <= 0` means the feature
/// never fires -- absent, not a degenerate huge period, matching `FrequencyAnchors`'
/// absent-means-off convention.
pub(super) fn chance_to_period_seconds(delay_ms: f64, chance_percent: f64) -> Option<f64> {
    if chance_percent <= 0.0 {
        return None;
    }
    Some(delay_ms / (10.0 * chance_percent))
}

/// Edgeware `config.json`/per-level `config` override keys that map onto a `FrequencyAnchors`
/// field -- recognized and converted (see `resolve_anchor_series`/`resolve_popup_anchor_series`),
/// unlike any other per-level `config` key (e.g. `promptMistakes`), which still just warns and
/// drops (see `build_timeline`).
pub(super) const LEVEL_ANCHOR_CONFIG_KEYS: &[&str] = &[
    "popupMod",
    "vidMod",
    "webMod",
    "notificationChance",
    "promptMod",
];

// Sensible default pacing assumed for a feature the pack clearly has content for, but that no
// config.json/corruption.json data anywhere gives an actual chance/rate for -- see
// `resolve_anchor_series`. Matches the same defaults already used as starting points in the pack
// editor's Experience tab (`OptionalNumberField`'s `default` prop), for consistency between "what
// a fresh toggle-on shows in the editor" and "what the converter assumes".
pub(super) const DEFAULT_WEB_PERIOD_SECONDS: f64 = 300.0;
pub(super) const DEFAULT_NOTIFICATION_PERIOD_SECONDS: f64 = 300.0;
pub(super) const DEFAULT_PROMPT_PERIOD_SECONDS: f64 = 90.0;

/// One `FrequencyAnchors` field's value at every point in the timeline (one entry per generated
/// level, or a single entry when there's no corruption.json at all) -- reconciles `config.json`'s
/// global setting with `corruption.json`'s own per-level `config` overrides for the same key.
/// Edgeware allows both, which is genuinely ambiguous to convert if either is trusted alone (see
/// `behaviour-design/edgeware-compat.md`'s Goal section): a pack can leave `config.json` silent on
/// a feature entirely and still turn it on and off across the corruption arc via per-level
/// overrides (e.g. `corruption.json`'s `{"config": {"1": {"promptMod": 0}, "4": {"promptMod":
/// 10}}}` -- prompts off at the baseline, on from level 4 onward).
///
/// Two cases:
/// - **The key appears somewhere** (`config.json`, or any corruption level's own override): fold
///   it cumulatively across levels, exactly like Edgeware's own runtime model (a corruption
///   level's config override is a persistent app-config mutation, not a one-shot pulse) -- seeded
///   from `config.json`'s value if set, else Edgeware's own real default for this key (`0`, i.e.
///   off, for every key this function handles -- popup's own real default is handled separately
///   by `resolve_popup_anchor_series`, since Edgeware's default for it is genuinely on).
/// - **The key never appears anywhere**: there's no numeric signal to convert at all. Defaulting
///   to Edgeware's literal "off" here would silently disable a feature the pack clearly has
///   content for -- so instead, assume the feature runs at a sensible default pace
///   (`default_period_if_content`) whenever the pack has usable content for it (`has_content`),
///   and stays genuinely absent otherwise.
pub(super) fn resolve_anchor_series(
    key: &str,
    delay_ms: f64,
    has_content: bool,
    default_period_if_content: f64,
    global_config: &Map<String, Value>,
    corruption_levels: &[CorruptionLevel],
) -> Vec<Option<f64>> {
    let explicit_anywhere = config_number(global_config, key).is_some()
        || corruption_levels
            .iter()
            .any(|level| level.config.contains_key(key));

    if !explicit_anywhere {
        let value = has_content.then_some(default_period_if_content);
        return vec![value; corruption_levels.len().max(1)];
    }

    let mut current_chance = config_number(global_config, key).unwrap_or(0.0);
    if corruption_levels.is_empty() {
        return vec![chance_to_period_seconds(delay_ms, current_chance)];
    }
    corruption_levels
        .iter()
        .map(|level| {
            if let Some(v) = level.config.get(key).and_then(Value::as_f64) {
                current_chance = v;
            }
            chance_to_period_seconds(delay_ms, current_chance)
        })
        .collect()
}

/// Like `resolve_anchor_series`, but for `popup` specifically: two Edgeware keys (`popupMod`/
/// `vidMod`) sum into lewdware's single combined anchor (Sandbox has no separate image/video
/// frequency either -- media type is a toggle, not a rate), and Edgeware's own real default for
/// this pair is genuinely *on* (`assets/default_config.json`: `popupMod: 100, vidMod: 10`), unlike
/// every other anchor key -- so when neither key appears anywhere, the "no signal -> fall back to
/// content presence" branch checks `has_popup_media` rather than a specific content pool, and
/// assumes Edgeware's own real default pace rather than a made-up one.
///
/// *Popup* media, not any media: the wallpaper and the splash are marked non-popup by
/// `discover_media` (and the hypno spirals never come in at all), so a pack holding nothing else
/// has no popups to pace.
pub(super) fn resolve_popup_anchor_series(
    delay_ms: f64,
    has_popup_media: bool,
    global_config: &Map<String, Value>,
    corruption_levels: &[CorruptionLevel],
) -> Vec<Option<f64>> {
    let explicit_anywhere = config_number(global_config, "popupMod").is_some()
        || config_number(global_config, "vidMod").is_some()
        || corruption_levels.iter().any(|level| {
            level.config.contains_key("popupMod") || level.config.contains_key("vidMod")
        });

    if !explicit_anywhere {
        let default_period =
            chance_to_period_seconds(delay_ms, DEFAULT_POPUP_MOD + DEFAULT_VID_MOD);
        let value = has_popup_media.then_some(default_period).flatten();
        return vec![value; corruption_levels.len().max(1)];
    }

    let mut current_popup_mod =
        config_number(global_config, "popupMod").unwrap_or(DEFAULT_POPUP_MOD);
    let mut current_vid_mod = config_number(global_config, "vidMod").unwrap_or(DEFAULT_VID_MOD);
    if corruption_levels.is_empty() {
        return vec![chance_to_period_seconds(
            delay_ms,
            current_popup_mod + current_vid_mod,
        )];
    }
    corruption_levels
        .iter()
        .map(|level| {
            if let Some(v) = level.config.get("popupMod").and_then(Value::as_f64) {
                current_popup_mod = v;
            }
            if let Some(v) = level.config.get("vidMod").and_then(Value::as_f64) {
                current_vid_mod = v;
            }
            chance_to_period_seconds(delay_ms, current_popup_mod + current_vid_mod)
        })
        .collect()
}

/// One resolved anchor value per generated level, for each of the 4 `FrequencyAnchors` fields --
/// bundled purely to keep `build_timeline`'s argument count reasonable.
pub(super) struct AnchorSeries {
    pub(super) popup: Vec<Option<f64>>,
    pub(super) web: Vec<Option<f64>>,
    pub(super) notification: Vec<Option<f64>>,
    pub(super) prompt: Vec<Option<f64>>,
}

/// Folds `corruption.json`'s levels (already parsed by `parse::corruption::load_corruption`) into
/// a `Vec<Level>` (never empty -- see `build_experience`), resolving each level's wallpaper
/// filename to a tag + (deduplicated) `ConvertedMedia` entry and pulling that level's own anchor
/// values from the already-resolved `anchors` series (see `resolve_anchor_series`). `design` stays
/// `DesignValues::default()` on every level: it isn't a pacing concept, and per-level design-value
/// overrides are out of scope here (see `design/release-plan.md`'s M4 converter bullet). Level 0
/// (Edgeware's own level 1, applied immediately at session start -- see the `at_seconds` comment
/// below) doubles as the new schema's baseline: it already has `at_seconds: 0.0`/`at_popups: None`,
/// exactly what a trigger-less baseline level needs, so no separate synthetic entry is required.
pub(super) fn build_timeline(
    levels: &[CorruptionLevel],
    anchors: &AnchorSeries,
    config: &Map<String, Value>,
    source: &dyn PackSource,
    content: &Content,
    media: &mut Vec<ConvertedMedia>,
    warnings: &mut Vec<Warning>,
) -> Vec<Level> {
    if levels.is_empty() {
        return Vec::new();
    }

    // Corruption moods only ever named in `corruption.json`'s add/remove lists (a mood with no
    // media or text of its own, so absent from `collect_reserved_tags`' content/media scan) are
    // still part of the tag namespace the synthetic tags must dodge -- fold them in here.
    let mut reserved = collect_reserved_tags(content, media);
    for level in levels {
        reserved.extend(level.added_moods.iter().cloned());
        reserved.extend(level.removed_moods.iter().cloned());
    }

    let corruption_time_seconds =
        config_number(config, "corruptionTime").unwrap_or(DEFAULT_CORRUPTION_TIME_SECONDS);
    let corruption_popups =
        config_number(config, "corruptionPopups").unwrap_or(DEFAULT_CORRUPTION_POPUPS);
    let trigger = config
        .get("corruptionTrigger")
        .and_then(Value::as_str)
        .unwrap_or("Timed");

    if matches!(trigger, "Launch" | "Script") {
        warnings.push(Warning::new(
            WarningKind::UnsupportedFeatureDropped,
            format!(
                "corruption.json's pack-recommended \"{trigger}\" trigger has no Lewdware \
                 equivalent -- the timeline advances on elapsed time instead"
            ),
        ));
    }

    let mut active: BTreeSet<String> = BTreeSet::new();

    // Edgeware's corruption model is exclusion-based: mood-less media always stays in the pool,
    // only mood-tagged media becomes conditionally available (`pack.active_moods` only ever gates
    // media that actually has a mood). `Modifiers.tags` is structurally an *inclusion* (`any`-of)
    // filter, so without this a level's tag restriction would wrongly drop every untagged file.
    // Tagging previously-untagged media with one synthetic tag and folding that tag into every
    // level's active set (never removed) reconciles the two models exactly: mood-less media now
    // matches every level's `any` filter, same as it always matched Edgeware's absence of
    // exclusion. Only seeded when needed, and only into levels (baseline -- `tags: None` -- is
    // already unrestricted and needs no help). Collision with a real mood of this exact name is
    // the same accepted, vanishingly small risk as `resolve_wallpaper_tag`'s `wallpaper-<n>`
    // tags.
    if media.iter().any(|m| m.tags.is_empty()) {
        let moodless_tag = free_tag(MOODLESS_TAG, &reserved);
        for item in media.iter_mut() {
            if item.tags.is_empty() {
                item.tags.push(moodless_tag.clone());
            }
        }
        active.insert(moodless_tag);
    }

    let root = RootFiles::read(source);

    // Seeded with the pack's primary wallpaper: `discover_media` already added it to `media`, and
    // levels reusing it (Edgeware's ordinary case) must not import it a second time. Asked of the
    // source rather than read off `content.wallpaper`, which holds a media id the converter never
    // sees -- `find_wallpaper` is the same answer `discover_media` itself used.
    let mut imported_wallpapers: HashSet<String> = find_wallpaper(source).into_iter().collect();

    levels
        .iter()
        .enumerate()
        .map(|(i, level)| {
            for mood in &level.removed_moods {
                active.remove(mood);
            }
            for mood in &level.added_moods {
                active.insert(mood.clone());
            }

            let mut leftover: Vec<&str> = level
                .config
                .keys()
                .map(String::as_str)
                .filter(|key| !LEVEL_ANCHOR_CONFIG_KEYS.contains(key))
                .collect();
            if !leftover.is_empty() {
                leftover.sort_unstable();
                warnings.push(Warning::new(
                    WarningKind::UnsupportedFeatureDropped,
                    format!(
                        "corruption.json level {}'s config override(s) ({}) have no Lewdware \
                         equivalent and were dropped",
                        i + 1,
                        leftover.join(", ")
                    ),
                ));
            }

            let wallpaper = level.wallpaper.as_ref().and_then(|file| {
                resolve_wallpaper(file, &root, media, &mut imported_wallpapers, warnings)
            });

            Level {
                // Level index 0 (Edgeware's level 1) is applied immediately at session start
                // (`handle_corruption` calls `apply_corruption_level` before ever waiting on
                // `corruption_time`), not after one interval -- see
                // `EdgewarePlusPlus/edgeware/src/features/corruption.py`. This also makes it
                // exactly the new schema's baseline level: `at_seconds: 0.0`/`at_popups: None`,
                // never read as a trigger.
                at_seconds: i as f64 * corruption_time_seconds,
                // Only meaningful (and only known) for the "Popup" trigger -- `corruption_popups`
                // isn't consulted at all for the other triggers, so inventing a popup count for
                // them would be fabricating a fact, not converting one.
                at_popups: (trigger == "Popup" && i > 0)
                    .then_some((i as f64 * corruption_popups) as u32),
                anchors: FrequencyAnchors {
                    popup: anchors.popup[i],
                    web: anchors.web[i],
                    notification: anchors.notification[i],
                    prompt: anchors.prompt[i],
                },
                design: DesignValues::default(),
                tags: Some(active.iter().cloned().collect()),
                wallpaper,
            }
        })
        .collect()
}

/// Warns about a `media_moods` entry naming a file the conversion never took in.
///
/// Deliberately checked against everything that ended up in `media`, at the end, rather than
/// against the media directories while they're being scanned: a pack routinely tags files it
/// keeps in the root, and those are picked up later as the wallpaper, the splash or a timeline
/// stage's wallpaper. Warning during the scan reported them as lost when they had simply not been
/// reached yet -- on a pack driving Edgeware's wallpaper rotation through a `"wallpapers"` mood,
/// that was every one of its wallpapers, all of which converted fine.
///
/// What survives is the real case: a mood naming a file that isn't in the pack at all. The file
/// keeps its mood in the pack's own JSON either way; there is simply nothing to attach it to.
///
/// Matched the way `media_moods_by_lowercase` matches, so that the warning and the tagging never
/// disagree -- a reference resolved for one has to count as resolved for the other, or a file
/// would lose its mood without anything saying so.
pub(super) fn warn_untagged_media_moods(
    index: &EdgewareIndex,
    media: &[ConvertedMedia],
    warnings: &mut Vec<Warning>,
) {
    let converted: HashSet<String> = media
        .iter()
        .map(|item| item.suggested_name.to_ascii_lowercase())
        .collect();
    // Sorted for deterministic output -- `index.media_moods` is a HashMap.
    let mut missing: Vec<(&String, &String)> = index
        .media_moods
        .iter()
        .filter(|(file, _)| !converted.contains(&file.to_ascii_lowercase()))
        .collect();
    missing.sort_by(|a, b| a.0.cmp(b.0));
    for (file, mood) in missing {
        warnings.push(Warning::new(
            WarningKind::UnreadableMediaFile,
            format!("\"{file}\" (tagged \"{mood}\") is referenced but isn't in the pack"),
        ));
    }
}

/// The pack root's file names, for resolving a JSON reference whose spelling doesn't match the
/// file's.
///
/// Packs are overwhelmingly authored on Windows, where the filesystem doesn't care: a
/// `corruption.json` naming `wallpaper.png` finds `Wallpaper.png` and nobody ever notices the
/// disagreement. On a case-sensitive filesystem -- or reading straight out of a zip, which is what
/// the converter does -- the same pack loses every one of those references.
pub(super) struct RootFiles(Vec<String>);

impl RootFiles {
    fn read(source: &dyn PackSource) -> Self {
        Self(source.list_dir(""))
    }

    /// The file `wanted` names, preferring an exact match so a root holding both spellings still
    /// resolves each of them to itself.
    fn resolve(&self, wanted: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|name| *name == wanted)
            .or_else(|| self.0.iter().find(|name| name.eq_ignore_ascii_case(wanted)))
            .map(String::as_str)
    }
}

/// Resolves one corruption level's wallpaper filename to the media name its stage references,
/// adding a `ConvertedMedia` entry the first time a given file is seen.
///
/// A stage's wallpaper is one file, so the filename *is* the reference -- no synthetic tag to
/// mint, and nothing to garbage-collect when a stage is later deleted. Reusing the same
/// wallpaper across levels is Edgeware's ordinary case (the pack's primary especially, which
/// `discover_media` already added and `build_timeline` pre-seeds into `seen`), so `seen` keeps
/// that from importing the same file twice -- and because both go through `RootFiles`, the two
/// agree even when `corruption.json` and the file on disk disagree about capitalization.
///
/// Returns `None` -- no override, meaning the previous level's wallpaper or `Content::wallpaper`
/// stays in effect -- if the referenced file doesn't exist, after warning.
pub(super) fn resolve_wallpaper(
    file: &str,
    root: &RootFiles,
    media: &mut Vec<ConvertedMedia>,
    seen: &mut HashSet<String>,
    warnings: &mut Vec<Warning>,
) -> Option<String> {
    let Some(name) = root.resolve(file) else {
        warnings.push(Warning::new(
            WarningKind::UnreadableMediaFile,
            format!("corruption.json references wallpaper \"{file}\" which wasn't found"),
        ));
        return None;
    };

    if seen.insert(name.to_string()) {
        media.push(ConvertedMedia {
            source_path: name.to_string(),
            suggested_name: name.to_string(),
            tags: vec![NON_POPUP_TAG.to_string()],
        });
    }
    Some(name.to_string())
}

/// `corruption.json`/`config.json` -> `behaviour.experience`. `None` (not
/// `Some(Experience::default())`) when neither contributes anything -- presence must stay
/// structurally meaningful, matching `Behaviour::experience`'s own doc comment and
/// `pack_has_experience`. With no `corruption.json` at all, `config.json`'s anchors alone still
/// produce a valid one-level (baseline-only) `Timeline` -- exactly what "a purely
/// statically-designed pack" is under the new per-level schema.
pub(super) fn build_experience(
    corruption_levels: &[CorruptionLevel],
    content: &Content,
    config: &Map<String, Value>,
    source: &dyn PackSource,
    media: &mut Vec<ConvertedMedia>,
    warnings: &mut Vec<Warning>,
) -> Option<(Experience, Vec<(MediaSlot, String)>)> {
    let delay_ms = config_number(config, "delay")
        .unwrap_or(DEFAULT_DELAY_MS)
        .max(1.0);

    let has_popup_media = media
        .iter()
        .any(|item| !item.tags.iter().any(|tag| tag == NON_POPUP_TAG));

    let anchors = AnchorSeries {
        popup: resolve_popup_anchor_series(delay_ms, has_popup_media, config, corruption_levels),
        web: resolve_anchor_series(
            "webMod",
            delay_ms,
            !content.web_links.is_empty(),
            DEFAULT_WEB_PERIOD_SECONDS,
            config,
            corruption_levels,
        ),
        notification: resolve_anchor_series(
            "notificationChance",
            delay_ms,
            !content.notifications.is_empty(),
            DEFAULT_NOTIFICATION_PERIOD_SECONDS,
            config,
            corruption_levels,
        ),
        prompt: resolve_anchor_series(
            "promptMod",
            delay_ms,
            !content.prompts.is_empty(),
            DEFAULT_PROMPT_PERIOD_SECONDS,
            config,
            corruption_levels,
        ),
    };

    let all_absent = corruption_levels.is_empty()
        && anchors.popup[0].is_none()
        && anchors.web[0].is_none()
        && anchors.notification[0].is_none()
        && anchors.prompt[0].is_none();
    if all_absent {
        return None;
    }

    let levels = if corruption_levels.is_empty() {
        vec![Level {
            at_seconds: 0.0,
            at_popups: None,
            anchors: FrequencyAnchors {
                popup: anchors.popup[0],
                web: anchors.web[0],
                notification: anchors.notification[0],
                prompt: anchors.prompt[0],
            },
            design: DesignValues::default(),
            tags: None,
            wallpaper: None,
        }]
    } else {
        build_timeline(
            corruption_levels,
            &anchors,
            config,
            source,
            content,
            media,
            warnings,
        )
    };

    Some(levels_to_experience(levels))
}

/// Every tag already live in the converted pack's flat tag namespace: the Edgeware mood names
/// (carried as media tags and content-group filter tags -- the slugified `ContentGroup.id` is
/// only a key, never a filter) plus the managed markers `discover_media` assigns.
/// `build_timeline` mints its synthetic `corruption-moodless` tag into
/// this same namespace, so it consults this set to guarantee it never aliases a tag the pack
/// itself uses -- otherwise a mood named `corruption-moodless` would let an unrelated mood change
/// drop the mood-less media the tag exists to protect.
pub(super) fn collect_reserved_tags(
    content: &Content,
    media: &[ConvertedMedia],
) -> HashSet<String> {
    let mut reserved = HashSet::new();
    for item in media {
        reserved.extend(item.tags.iter().cloned());
    }
    for group in &content.content_groups {
        reserved.extend(group.tags.iter().cloned());
    }
    reserved
}

/// Returns `base` if it's free in `reserved`, otherwise the first of `base-2`, `base-3`, ... that
/// is -- keeping a synthetic tag from ever aliasing a real one. Mirrors `unique_slug`'s collision
/// suffixing, but tests against a fixed set rather than mutating one (the synthetic tags are minted
/// at most once each, so nothing needs marking as taken afterwards).
pub(super) fn free_tag(base: &str, reserved: &HashSet<String>) -> String {
    if !reserved.contains(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !reserved.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}
