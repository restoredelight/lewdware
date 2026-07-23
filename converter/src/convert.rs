use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;
use serde_json::{Map, Value};
use shared::behaviour::{
    Behaviour, ContentSelection, CountScope, Easing, EndStrategy, EventCountCondition, EventKind,
    EventSchedule, Events, Experience, Interval, Mitosis, Movement, Stage, StageEnd, Timeline,
    Transition,
};
use shared::behaviour::{Content, ContentGroup, PromptSettings, TextItem, WebLink};
use shared::read_pack::{Metadata, RecommendedMode};

/// An intermediate, cumulative-timeline-friendly shape the converter builds internally --
/// mirrors the pre-Transitions "one flat, self-contained snapshot per level" schema, which is a
/// far simpler target for `build_timeline`'s cumulative tag/anchor folding than the stage graph
/// (transitions between stages, `end` conditions referencing the *next* stage) is. Converted to
/// real `Stage`/`Transition`s by `levels_to_experience` once every level is resolved.
#[derive(Debug, Clone, Default, PartialEq)]
struct Level {
    at_seconds: f64,
    at_popups: Option<u32>,
    anchors: FrequencyAnchors,
    design: DesignValues,
    tags: Option<Vec<String>>,
    wallpaper_tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct FrequencyAnchors {
    popup: Option<f64>,
    web: Option<f64>,
    notification: Option<f64>,
    prompt: Option<f64>,
    subliminal: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct DesignValues {
    movement_speed_min: Option<f64>,
    movement_speed_max: Option<f64>,
    mitosis_chance: Option<f64>,
    mitosis_count: Option<u32>,
}

fn schedule(seconds: Option<f64>) -> Option<EventSchedule> {
    seconds.map(|seconds| EventSchedule {
        interval: Interval::Fixed { seconds },
        initial_delay_seconds: None,
        max_concurrent: None,
    })
}

fn movement(design: &DesignValues) -> Option<Movement> {
    match (design.movement_speed_min, design.movement_speed_max) {
        (None, None) => None,
        (minimum_speed, maximum_speed) => Some(Movement {
            minimum_speed,
            maximum_speed,
        }),
    }
}

fn mitosis(design: &DesignValues) -> Option<Mitosis> {
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
fn levels_to_experience(levels: Vec<Level>) -> Experience {
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
                    wallpaper_tags: level.wallpaper_tags.clone(),
                },
                events: Events {
                    popup: schedule(level.anchors.popup),
                    web: schedule(level.anchors.web),
                    notification: schedule(level.anchors.notification),
                    prompt: schedule(level.anchors.prompt),
                    subliminal: schedule(level.anchors.subliminal),
                },
                movement: movement(&level.design),
                mitosis: mitosis(&level.design),
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
    // Edgeware's own name for a multi-level progression is "corruption"; presenting the timeline
    // mode under that label keeps converted packs legible to the users they came from. A
    // single-stage timeline has no progression, so it keeps the mode's own name.
    let label = (stages.len() > 1).then(|| "Corruption".to_string());
    Experience {
        timeline: Timeline {
            stages,
            transitions,
        },
        label,
    }
}

use crate::model::{CorruptionLevel, EdgewareIndex, EdgewareMood, Warning, WarningKind};
use crate::parse::{self, InfoJson};
use crate::slug::unique_slug;
use crate::source::PackSource;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConvertedMedia {
    /// Pack-root-relative, forward-slash path, resolvable against the same `PackSource` that
    /// was converted (e.g. `"img/foo.png"`, `"wallpaper.png"`). The converter never reads media
    /// bytes -- re-encoding/copying is the front end's job.
    pub source_path: String,
    pub suggested_name: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConversionOutput {
    pub metadata: Metadata,
    pub behaviour: Behaviour,
    pub media: Vec<ConvertedMedia>,
    /// `icon.ico`'s path, if present -- a thumbnail candidate. The converter doesn't decode or
    /// convert it; that's front-end encoding work.
    pub icon: Option<String>,
    pub warnings: Vec<Warning>,
}

/// Converts an Edgeware/Edgeware++ pack into the pieces a `.lwpack` needs: media (with tags),
/// pack metadata, and a behaviour.json `Content` + `Experience` section (`corruption.json` ->
/// transition timeline, `config.json` -> frequency anchors).
///
/// Infallible: every pack-content problem degrades to a `Warning` rather than failing the
/// conversion (an unreadable/nonexistent source just reads as an empty pack, since every
/// `PackSource` method already treats "can't read this" as "not present" -- the one place a
/// *hard* I/O failure can be meaningfully distinguished from "legitimately empty" is opening the
/// source itself, which is why `ZipSource::open` returns a `Result` but this function doesn't).
pub fn convert(source: &dyn PackSource) -> ConversionOutput {
    let mut warnings = Vec::new();

    let info = parse::load_info(source, &mut warnings);
    let index = parse::load_edgeware_index(source, &mut warnings);
    let corruption_levels = parse::corruption::load_corruption(source, &mut warnings);
    let config = parse::load_config(source, &mut warnings);

    let mut used_ids = HashSet::new();
    let content_groups = build_content_groups(&index, &mut used_ids);
    let mut content = Content {
        content_groups,
        ..Content::default()
    };
    populate_captions(&index, &mut content);
    populate_prompts(&index, &mut content);
    populate_notifications(&index, &mut content);
    populate_subliminals(&index, &mut content);
    populate_web_links(&index, &mut content);

    let mut media = Vec::new();
    discover_media(source, &index, &mut content, &mut media, &mut warnings);

    let experience = build_experience(
        &corruption_levels,
        &content,
        &config,
        source,
        &mut media,
        &mut warnings,
    );
    // More than just the baseline level means the pack actually designs an escalating arc, not
    // just a static config.json-derived pace -- see `build_experience`.
    let has_timeline = experience
        .as_ref()
        .is_some_and(|e| e.timeline.stages.len() > 1);

    check_unsupported_files(source, &mut warnings);
    warn_unmapped_config_keys(&config, &mut warnings);

    let metadata = build_metadata(info, has_timeline);

    let icon = source
        .file_exists("icon.ico")
        .then(|| "icon.ico".to_string());

    ConversionOutput {
        metadata,
        behaviour: Behaviour {
            content,
            experience,
            ..Behaviour::new()
        },
        media,
        icon,
        warnings,
    }
}

fn build_metadata(info: InfoJson, has_timeline: bool) -> Metadata {
    Metadata {
        name: info.name.unwrap_or_else(|| "Unnamed Pack".to_string()),
        creator: info.creator,
        description: info.description,
        version: info.version,
        // Experience only when the pack actually designs an arc (a corruption timeline);
        // config.json pacing alone still populates anchors but doesn't flip the recommendation --
        // see `behaviour-design/edgeware-compat.md`'s Goal section.
        recommended_mode: Some(if has_timeline {
            RecommendedMode::Experience
        } else {
            RecommendedMode::Sandbox
        }),
    }
}

fn build_content_groups(
    index: &EdgewareIndex,
    used_ids: &mut HashSet<String>,
) -> Vec<ContentGroup> {
    index
        .moods
        .iter()
        .map(|mood| ContentGroup {
            id: unique_slug(&mood.name, used_ids),
            label: mood.name.clone(),
            description: None,
            tags: vec![mood.name.clone()],
            enabled_by_default: true,
        })
        .collect()
}

/// Builds a `TextItem` pool from `default_pool` (untagged) plus each mood's own pool (tagged
/// `[mood.name]`), via `pool` selecting which `MoodBase` field to read. Shared by captions,
/// prompts, notifications and subliminals -- they're all `{ tags: string[] }` pools over the
/// same mood structure.
fn text_items(
    default_pool: &[String],
    moods: &[EdgewareMood],
    pool: impl Fn(&crate::model::MoodBase) -> &Vec<String>,
) -> Vec<TextItem> {
    let mut items: Vec<TextItem> = default_pool
        .iter()
        .map(|text| TextItem {
            text: text.clone(),
            tags: vec![],
        })
        .collect();
    for mood in moods {
        for text in pool(&mood.base) {
            items.push(TextItem {
                text: text.clone(),
                tags: vec![mood.name.clone()],
            });
        }
    }
    items
}

fn populate_captions(index: &EdgewareIndex, content: &mut Content) {
    content.captions = text_items(&index.default.captions, &index.moods, |base| &base.captions);
    // Fold the close-button label into captions (per `behaviour-design/edgeware-compat.md`) --
    // only when the author actually set it, so a pack that never touched `popupClose` doesn't
    // get Edgeware's own boilerplate ("I Submit <3") injected as a fake caption.
    if let Some(popup_close) = &index.default_extra.popup_close {
        content.captions.push(TextItem {
            text: popup_close.clone(),
            tags: vec![],
        });
    }
}

fn populate_prompts(index: &EdgewareIndex, content: &mut Content) {
    content.prompts = text_items(&index.default.prompts, &index.moods, |base| &base.prompts);
    content.prompt_settings = PromptSettings {
        submit_label: index.default_extra.prompt_submit.clone(),
    };
}

fn populate_notifications(index: &EdgewareIndex, content: &mut Content) {
    content.notifications = text_items(&index.default.notifications, &index.moods, |base| {
        &base.notifications
    });
}

fn populate_subliminals(index: &EdgewareIndex, content: &mut Content) {
    content.subliminals = text_items(&index.default.subliminals, &index.moods, |base| {
        &base.subliminals
    });
}

fn populate_web_links(index: &EdgewareIndex, content: &mut Content) {
    let mut links: Vec<WebLink> = index
        .default
        .web
        .iter()
        .map(|entry| WebLink {
            url: entry.url.clone(),
            args: entry.args.clone(),
            tags: vec![],
        })
        .collect();
    for mood in &index.moods {
        for entry in &mood.base.web {
            links.push(WebLink {
                url: entry.url.clone(),
                args: entry.args.clone(),
                tags: vec![mood.name.clone()],
            });
        }
    }
    content.web_links = links;
}

const MEDIA_DIRS: &[&str] = &["img", "vid", "aud"];
const SPLASH_CANDIDATES: &[&str] = &[
    "loading_splash.png",
    "loading_splash.gif",
    "loading_splash.jpg",
    "loading_splash.jpeg",
    "loading_splash.bmp",
];

fn discover_media(
    source: &dyn PackSource,
    index: &EdgewareIndex,
    content: &mut Content,
    media: &mut Vec<ConvertedMedia>,
    warnings: &mut Vec<Warning>,
) {
    let mut discovered_files: HashSet<String> = HashSet::new();

    for dir in MEDIA_DIRS {
        for file in source.list_dir(dir) {
            discovered_files.insert(file.clone());
            let tags = index
                .media_moods
                .get(&file)
                .cloned()
                .map(|mood| vec![mood])
                .unwrap_or_default();
            media.push(ConvertedMedia {
                source_path: format!("{dir}/{file}"),
                suggested_name: file,
                tags,
            });
        }
    }

    // A `media_moods` entry that names a file never actually found under img/vid/aud -- warn,
    // sorted for deterministic output (`index.media_moods` is a HashMap).
    let mut missing: Vec<(&String, &String)> = index
        .media_moods
        .iter()
        .filter(|(file, _)| !discovered_files.contains(*file))
        .collect();
    missing.sort_by(|a, b| a.0.cmp(b.0));
    for (file, mood) in missing {
        warnings.push(Warning::new(
            WarningKind::UnreadableMediaFile,
            format!("\"{file}\" (tagged \"{mood}\") is referenced but wasn't found in img/vid/aud"),
        ));
    }

    // `hypno/` (or the legacy `subliminals/` dir, if `hypno/` is empty or absent) -- every file
    // tagged `"hypno"`, per the concept-mapping table in
    // `behaviour-design/edgeware-compat.md`.
    let hypno = source.list_dir("hypno");
    let (hypno_dir, hypno_files) = if hypno.is_empty() {
        ("subliminals", source.list_dir("subliminals"))
    } else {
        ("hypno", hypno)
    };
    for file in hypno_files {
        media.push(ConvertedMedia {
            source_path: format!("{hypno_dir}/{file}"),
            suggested_name: file,
            tags: vec!["hypno".to_string()],
        });
    }

    if source.file_exists("wallpaper.png") {
        media.push(ConvertedMedia {
            source_path: "wallpaper.png".to_string(),
            suggested_name: "wallpaper.png".to_string(),
            tags: vec!["wallpaper".to_string()],
        });
        content.wallpaper_tags = vec!["wallpaper".to_string()];
    }

    if let Some(splash) = SPLASH_CANDIDATES
        .iter()
        .find(|name| source.file_exists(name))
    {
        media.push(ConvertedMedia {
            source_path: splash.to_string(),
            suggested_name: splash.to_string(),
            tags: vec!["splash".to_string()],
        });
        content.splash_tags = vec!["splash".to_string()];
    }
}

fn check_unsupported_files(source: &dyn PackSource, warnings: &mut Vec<Warning>) {
    if source.file_exists("script.lua") {
        warnings.push(Warning::new(
            WarningKind::ScriptSkipped,
            "script.lua is present but Edgeware's custom scripting isn't converted",
        ));
    }
    if source.file_exists("discord.dat") {
        warnings.push(Warning::new(
            WarningKind::DiscordSkipped,
            "discord.dat is present but Discord rich presence isn't supported",
        ));
    }
}

/// `config.json` keys `resolve_anchor_series`/`resolve_popup_anchor_series` read. Anything else
/// in the pack's `config.json` has no Lewdware equivalent (theme, hibernate, mitosis, drive,
/// scheduler, ... -- see `EdgewarePlusPlus/edgeware/src/config/items.py`) and is silently dropped, per
/// `behaviour-design/edgeware-compat.md`'s "warn + skip the rest".
const CONFIG_ANCHOR_KEYS: &[&str] = &[
    "delay",
    "popupMod",
    "vidMod",
    "webMod",
    "notificationChance",
    "promptMod",
    "capPopChance",
];
/// Keys `build_timeline` reads to pace `corruption.json` (only meaningful alongside it, but
/// still a recognized, converted setting rather than a dropped one -- excluded from the
/// "leftover, unmapped" tally same as `CONFIG_ANCHOR_KEYS`). `corruptionMode` (the on/off switch
/// itself) is deliberately *not* here: a present `corruption.json` always converts regardless of
/// whether the pack's `config.json` happened to leave corruption disabled, so the switch itself
/// has nothing to map to and stays counted as dropped.
const CONFIG_TIMELINE_KEYS: &[&str] = &["corruptionTime", "corruptionPopups", "corruptionTrigger"];
/// Keys Edgeware's own `load_config` filters out before anything sees them (`pack/load.py`) --
/// excluded from the "leftover, unmapped" tally since they were never real settings to begin
/// with.
const CONFIG_META_KEYS: &[&str] = &["version", "versionplusplus", "packPath"];

fn warn_unmapped_config_keys(config: &Map<String, Value>, warnings: &mut Vec<Warning>) {
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
const DEFAULT_DELAY_MS: f64 = 5000.0;
const DEFAULT_CORRUPTION_TIME_SECONDS: f64 = 60.0;
const DEFAULT_CORRUPTION_POPUPS: f64 = 5.0;
// Edgeware's own `assets/default_config.json`: a pack shipping no config.json at all still runs
// with these chances in real Edgeware, so falling back to 0 here (like every other -- genuinely
// opt-in -- anchor key) would misrepresent the pack, the same reasoning that already justifies
// the three fallback constants above.
const DEFAULT_POPUP_MOD: f64 = 100.0;
const DEFAULT_VID_MOD: f64 = 10.0;

/// Synthetic tag `build_timeline` assigns to every previously-untagged media file, so a
/// timeline level's `any`-of-tags restriction still includes mood-less media -- see its use site
/// for the full reasoning.
const MOODLESS_TAG: &str = "corruption-moodless";

fn config_number(config: &Map<String, Value>, key: &str) -> Option<f64> {
    config.get(key).and_then(Value::as_f64)
}

/// Edgeware's shared popup-tick model: every `delay` milliseconds, each feature independently
/// rolls a `chance` percent chance to fire (see `EdgewarePlusPlus/edgeware/src/main_edgeware.py`'s
/// `main()` and `roll_targets`). Converted to lewdware's "seconds between events" convention:
/// `events/sec = (chance / 100) * (1000 / delay_ms)`, inverted. `chance <= 0` means the feature
/// never fires -- absent, not a degenerate huge period, matching `FrequencyAnchors`'
/// absent-means-off convention.
fn chance_to_period_seconds(delay_ms: f64, chance_percent: f64) -> Option<f64> {
    if chance_percent <= 0.0 {
        return None;
    }
    Some(delay_ms / (10.0 * chance_percent))
}

/// Edgeware `config.json`/per-level `config` override keys that map onto a `FrequencyAnchors`
/// field -- recognized and converted (see `resolve_anchor_series`/`resolve_popup_anchor_series`),
/// unlike any other per-level `config` key (e.g. `promptMistakes`), which still just warns and
/// drops (see `build_timeline`).
const LEVEL_ANCHOR_CONFIG_KEYS: &[&str] = &[
    "popupMod",
    "vidMod",
    "webMod",
    "notificationChance",
    "promptMod",
    "capPopChance",
];

// Sensible default pacing assumed for a feature the pack clearly has content for, but that no
// config.json/corruption.json data anywhere gives an actual chance/rate for -- see
// `resolve_anchor_series`. Matches the same defaults already used as starting points in the pack
// editor's Experience tab (`OptionalNumberField`'s `default` prop), for consistency between "what
// a fresh toggle-on shows in the editor" and "what the converter assumes".
const DEFAULT_WEB_PERIOD_SECONDS: f64 = 300.0;
const DEFAULT_NOTIFICATION_PERIOD_SECONDS: f64 = 300.0;
const DEFAULT_PROMPT_PERIOD_SECONDS: f64 = 90.0;
const DEFAULT_SUBLIMINAL_PERIOD_SECONDS: f64 = 60.0;

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
fn resolve_anchor_series(
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
/// content presence" branch checks `has_media` (any media at all) rather than a specific content
/// pool, and assumes Edgeware's own real default pace rather than a made-up one.
fn resolve_popup_anchor_series(
    delay_ms: f64,
    has_media: bool,
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
        let value = has_media.then_some(default_period).flatten();
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

/// One resolved anchor value per generated level, for each of the 5 `FrequencyAnchors` fields --
/// bundled purely to keep `build_timeline`'s argument count reasonable.
struct AnchorSeries {
    popup: Vec<Option<f64>>,
    web: Vec<Option<f64>>,
    notification: Vec<Option<f64>>,
    prompt: Vec<Option<f64>>,
    subliminal: Vec<Option<f64>>,
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
fn build_timeline(
    levels: &[CorruptionLevel],
    anchors: &AnchorSeries,
    config: &Map<String, Value>,
    source: &dyn PackSource,
    media: &mut Vec<ConvertedMedia>,
    reserved: &HashSet<String>,
    warnings: &mut Vec<Warning>,
) -> Vec<Level> {
    if levels.is_empty() {
        return Vec::new();
    }

    // Corruption moods only ever named in `corruption.json`'s add/remove lists (a mood with no
    // media or text of its own, so absent from `collect_reserved_tags`' content/media scan) are
    // still part of the tag namespace the synthetic tags must dodge -- fold them in here.
    let mut reserved = reserved.clone();
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

    let mut next_wallpaper_index: u32 = 1;
    let mut wallpaper_tags_by_file: HashMap<String, String> = HashMap::new();

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

            let wallpaper_tags = level.wallpaper.as_ref().and_then(|file| {
                resolve_wallpaper_tag(
                    file,
                    source,
                    media,
                    &mut next_wallpaper_index,
                    &reserved,
                    &mut wallpaper_tags_by_file,
                    warnings,
                )
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
                    subliminal: anchors.subliminal[i],
                },
                design: DesignValues::default(),
                tags: Some(active.iter().cloned().collect()),
                wallpaper_tags,
            }
        })
        .collect()
}

/// Resolves one corruption level's wallpaper filename to a tag, adding a `ConvertedMedia` entry
/// the first time a given filename is seen. `"wallpaper.png"` reuses the tag `discover_media`
/// already assigned the pack's primary wallpaper (no duplicate media entry); any other filename
/// mints a fresh `wallpaper-<n>` tag, numbering distinct override wallpapers `1`, `2`, ... in the
/// order they're first referenced (skipping any index already taken by a real tag in `reserved`,
/// so the synthetic tag never aliases a mood the pack itself named `wallpaper-<n>`). Returns `None`
/// (no override -- the previous level's wallpaper, or `Content::wallpaper_tags`, stays in effect)
/// if the referenced file doesn't actually exist, after warning.
fn resolve_wallpaper_tag(
    file: &str,
    source: &dyn PackSource,
    media: &mut Vec<ConvertedMedia>,
    next_index: &mut u32,
    reserved: &HashSet<String>,
    known: &mut HashMap<String, String>,
    warnings: &mut Vec<Warning>,
) -> Option<Vec<String>> {
    if let Some(tag) = known.get(file) {
        return Some(vec![tag.clone()]);
    }

    if file == "wallpaper.png" {
        known.insert(file.to_string(), "wallpaper".to_string());
        return Some(vec!["wallpaper".to_string()]);
    }

    if !source.file_exists(file) {
        warnings.push(Warning::new(
            WarningKind::UnreadableMediaFile,
            format!("corruption.json references wallpaper \"{file}\" which wasn't found"),
        ));
        return None;
    }

    let tag = loop {
        let candidate = format!("wallpaper-{next_index}");
        *next_index += 1;
        if !reserved.contains(&candidate) {
            break candidate;
        }
    };
    media.push(ConvertedMedia {
        source_path: file.to_string(),
        suggested_name: file.to_string(),
        tags: vec![tag.clone()],
    });
    known.insert(file.to_string(), tag.clone());
    Some(vec![tag])
}

/// `corruption.json`/`config.json` -> `behaviour.experience`. `None` (not
/// `Some(Experience::default())`) when neither contributes anything -- presence must stay
/// structurally meaningful, matching `Behaviour::experience`'s own doc comment and
/// `pack_has_experience`. With no `corruption.json` at all, `config.json`'s anchors alone still
/// produce a valid one-level (baseline-only) `Timeline` -- exactly what "a purely
/// statically-designed pack" is under the new per-level schema.
fn build_experience(
    corruption_levels: &[CorruptionLevel],
    content: &Content,
    config: &Map<String, Value>,
    source: &dyn PackSource,
    media: &mut Vec<ConvertedMedia>,
    warnings: &mut Vec<Warning>,
) -> Option<Experience> {
    let delay_ms = config_number(config, "delay")
        .unwrap_or(DEFAULT_DELAY_MS)
        .max(1.0);

    let anchors = AnchorSeries {
        popup: resolve_popup_anchor_series(delay_ms, !media.is_empty(), config, corruption_levels),
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
        subliminal: resolve_anchor_series(
            "capPopChance",
            delay_ms,
            !content.subliminals.is_empty(),
            DEFAULT_SUBLIMINAL_PERIOD_SECONDS,
            config,
            corruption_levels,
        ),
    };

    let all_absent = corruption_levels.is_empty()
        && anchors.popup[0].is_none()
        && anchors.web[0].is_none()
        && anchors.notification[0].is_none()
        && anchors.prompt[0].is_none()
        && anchors.subliminal[0].is_none();
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
                subliminal: anchors.subliminal[0],
            },
            design: DesignValues::default(),
            tags: None,
            wallpaper_tags: None,
        }]
    } else {
        let reserved = collect_reserved_tags(content, media);
        build_timeline(
            corruption_levels,
            &anchors,
            config,
            source,
            media,
            &reserved,
            warnings,
        )
    };

    Some(levels_to_experience(levels))
}

/// Every tag already live in the converted pack's flat tag namespace: the raw Edgeware mood names
/// (carried verbatim as media tags and content-group filter tags -- the slugified `ContentGroup.id`
/// is only a key, never a filter) plus the built-in `wallpaper`/`splash`/`hypno` tags
/// `discover_media` assigns. `build_timeline` mints its synthetic `corruption-moodless` and
/// `wallpaper-<n>` tags into this same namespace, so it consults this set to guarantee they never
/// alias a tag the pack itself uses -- otherwise a mood literally named `wallpaper-1` would pull
/// its own media into a level's wallpaper slot, and one named `corruption-moodless` would let an
/// unrelated mood change drop the mood-less media the tag exists to protect.
fn collect_reserved_tags(content: &Content, media: &[ConvertedMedia]) -> HashSet<String> {
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
fn free_tag(base: &str, reserved: &HashSet<String>) -> String {
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

#[cfg(test)]
mod tests {
    use crate::source::DirSource;

    use super::*;

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

    #[test]
    fn wallpaper_and_splash_tagged_and_reflected_in_content() {
        let (_dir, source) = source_with(&[("wallpaper.png", "w"), ("loading_splash.gif", "s")]);
        let output = convert(&source);
        assert_eq!(
            output.behaviour.content.wallpaper_tags,
            vec!["wallpaper".to_string()]
        );
        assert_eq!(
            output.behaviour.content.splash_tags,
            vec!["splash".to_string()]
        );
        assert!(
            output.media.iter().any(
                |m| m.source_path == "wallpaper.png" && m.tags == vec!["wallpaper".to_string()]
            )
        );
        assert!(
            output
                .media
                .iter()
                .any(|m| m.source_path == "loading_splash.gif"
                    && m.tags == vec!["splash".to_string()])
        );
    }

    #[test]
    fn hypno_falls_back_to_legacy_subliminals_dir() {
        let (_dir, source) = source_with(&[("subliminals/x.gif", "x")]);
        let output = convert(&source);
        let entry = output
            .media
            .iter()
            .find(|m| m.suggested_name == "x.gif")
            .unwrap();
        assert_eq!(entry.source_path, "subliminals/x.gif");
        assert_eq!(entry.tags, vec!["hypno".to_string()]);
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
        assert!(
            !output
                .warnings
                .iter()
                .any(|w| w.kind == WarningKind::UnsupportedFeatureDropped
                    && w.message.contains("promptMod"))
        );
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
    fn level_wallpaper_reuses_the_primary_wallpaper_tag_without_duplicate_media() {
        let (_dir, source) = source_with(&[
            ("wallpaper.png", "w"),
            (
                "corruption.json",
                r#"{"moods": {}, "wallpapers": {"1": "wallpaper.png"}, "config": {}}"#,
            ),
        ]);
        let output = convert(&source);
        let timeline = output.behaviour.experience.unwrap().timeline;
        assert_eq!(
            timeline.stages[0].content.wallpaper_tags,
            Some(vec!["wallpaper".to_string()])
        );
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
    fn level_wallpaper_other_than_primary_gets_its_own_tag_and_media_entry() {
        let (_dir, source) = source_with(&[
            ("wallpaper2.png", "w2"),
            (
                "corruption.json",
                r#"{"moods": {}, "wallpapers": {"1": "wallpaper2.png"}, "config": {}}"#,
            ),
        ]);
        let output = convert(&source);
        let timeline = output.behaviour.experience.unwrap().timeline;
        let tags = timeline.stages[0].content.wallpaper_tags.clone().unwrap();
        assert_eq!(tags.len(), 1);
        assert_ne!(tags[0], "wallpaper");
        let entry = output
            .media
            .iter()
            .find(|m| m.source_path == "wallpaper2.png")
            .unwrap();
        assert_eq!(entry.tags, tags);
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
        assert_eq!(timeline.stages[0].content.wallpaper_tags, None);
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
    fn level_wallpaper_tag_dodges_a_real_mood_of_the_same_name() {
        // A pack whose own mood is literally named `wallpaper-1` (its media carries that tag). The
        // minted override-wallpaper tag must not reuse it, or the level's wallpaper slot would pull
        // from that mood's media.
        let (_dir, source) = source_with(&[
            (
                "index.json",
                r#"{"moods": [{"mood": "wallpaper-1", "media": ["a.png"]}]}"#,
            ),
            ("img/a.png", "a"),
            ("wallpaper2.png", "w2"),
            (
                "corruption.json",
                r#"{"moods": {}, "wallpapers": {"1": "wallpaper2.png"}, "config": {}}"#,
            ),
        ]);
        let output = convert(&source);

        let timeline = output.behaviour.experience.clone().unwrap().timeline;
        assert_eq!(
            timeline.stages[0].content.wallpaper_tags,
            Some(vec!["wallpaper-2".to_string()]),
            "the override wallpaper must skip the taken `wallpaper-1` and take `wallpaper-2`"
        );
        let entry = output
            .media
            .iter()
            .find(|m| m.source_path == "wallpaper2.png")
            .unwrap();
        assert_eq!(entry.tags, vec!["wallpaper-2".to_string()]);
        // The real mood's media keeps `wallpaper-1`, unshared with the wallpaper slot.
        let a = output
            .media
            .iter()
            .find(|m| m.suggested_name == "a.png")
            .unwrap();
        assert_eq!(a.tags, vec!["wallpaper-1".to_string()]);
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
}
