use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;
use serde_json::{Map, Value};
use shared::behaviour::{
    Behaviour, Content, ContentGroup, DesignValues, Experience, FrequencyAnchors, Level, Modifiers,
    PromptSettings, TextItem, Timeline, WebLink,
};
use shared::read_pack::{Metadata, RecommendedMode};

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
        &config,
        source,
        &mut media,
        &mut warnings,
    );
    let has_timeline = experience.as_ref().is_some_and(|e| e.timeline.is_some());

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

/// `config.json` keys `build_anchors` reads. Anything else in the pack's `config.json` has no
/// Lewdware equivalent (theme, hibernate, mitosis, drive, scheduler, ... -- see
/// `EdgewarePlusPlus/edgeware/src/config/items.py`) and is silently dropped, per
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

/// `config.json` -> `experience.anchors`, per `CONFIG_ANCHOR_KEYS`. `image_chance`/`video_chance`
/// both spawn on the same popup tick and share lewdware's single combined `popup` anchor (Sandbox
/// has no separate image/video frequency either -- media type is a toggle, not a rate), so their
/// chances sum before conversion.
fn build_anchors(config: &Map<String, Value>) -> FrequencyAnchors {
    let delay_ms = config_number(config, "delay")
        .unwrap_or(DEFAULT_DELAY_MS)
        .max(1.0);
    let popup_chance = config_number(config, "popupMod").unwrap_or(0.0)
        + config_number(config, "vidMod").unwrap_or(0.0);

    FrequencyAnchors {
        popup: chance_to_period_seconds(delay_ms, popup_chance),
        web: chance_to_period_seconds(delay_ms, config_number(config, "webMod").unwrap_or(0.0)),
        notification: chance_to_period_seconds(
            delay_ms,
            config_number(config, "notificationChance").unwrap_or(0.0),
        ),
        prompt: chance_to_period_seconds(
            delay_ms,
            config_number(config, "promptMod").unwrap_or(0.0),
        ),
        subliminal: chance_to_period_seconds(
            delay_ms,
            config_number(config, "capPopChance").unwrap_or(0.0),
        ),
    }
}

/// Folds `corruption.json`'s levels (already parsed by `parse::corruption::load_corruption`) into
/// an absolute-per-level `Timeline`, and resolves each level's wallpaper filename to a tag +
/// (deduplicated) `ConvertedMedia` entry. `None` if there are no levels to convert.
fn build_timeline(
    levels: &[CorruptionLevel],
    config: &Map<String, Value>,
    source: &dyn PackSource,
    media: &mut Vec<ConvertedMedia>,
    warnings: &mut Vec<Warning>,
) -> Option<Timeline> {
    if levels.is_empty() {
        return None;
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
    // the same accepted, vanishingly small risk as `resolve_wallpaper_tag`'s `corruption-wallpaper-*`
    // tags.
    if media.iter().any(|m| m.tags.is_empty()) {
        for item in media.iter_mut() {
            if item.tags.is_empty() {
                item.tags.push(MOODLESS_TAG.to_string());
            }
        }
        active.insert(MOODLESS_TAG.to_string());
    }

    let mut wallpaper_tag_ids: HashSet<String> = HashSet::new();
    let mut wallpaper_tags_by_file: HashMap<String, String> = HashMap::new();

    let out_levels = levels
        .iter()
        .enumerate()
        .map(|(i, level)| {
            for mood in &level.removed_moods {
                active.remove(mood);
            }
            for mood in &level.added_moods {
                active.insert(mood.clone());
            }

            if !level.config_keys.is_empty() {
                warnings.push(Warning::new(
                    WarningKind::UnsupportedFeatureDropped,
                    format!(
                        "corruption.json level {}'s config override(s) ({}) have no Lewdware \
                         equivalent -- a timeline level's modifier is a single scalar, not \
                         per-setting, so they were dropped",
                        i + 1,
                        level.config_keys.join(", ")
                    ),
                ));
            }

            let wallpaper_tags = level.wallpaper.as_ref().and_then(|file| {
                resolve_wallpaper_tag(
                    file,
                    source,
                    media,
                    &mut wallpaper_tag_ids,
                    &mut wallpaper_tags_by_file,
                    warnings,
                )
            });

            Level {
                // Level index 0 (Edgeware's level 1) is applied immediately at session start
                // (`handle_corruption` calls `apply_corruption_level` before ever waiting on
                // `corruption_time`), not after one interval -- see
                // `EdgewarePlusPlus/edgeware/src/features/corruption.py`.
                at_seconds: i as f64 * corruption_time_seconds,
                // Only meaningful (and only known) for the "Popup" trigger -- `corruption_popups`
                // isn't consulted at all for the other triggers, so inventing a popup count for
                // them would be fabricating a fact, not converting one.
                at_popups: (trigger == "Popup" && i > 0)
                    .then_some((i as f64 * corruption_popups) as u32),
                modifiers: Modifiers {
                    // Per-level config overrides are the only source of a genuine rate change in
                    // Edgeware's corruption model, and those are dropped (warned above) -- so
                    // there's no reliable signal left to drive `modifier` from.
                    modifier: None,
                    tags: Some(active.iter().cloned().collect()),
                    wallpaper_tags,
                },
            }
        })
        .collect();

    Some(Timeline { levels: out_levels })
}

/// Resolves one corruption level's wallpaper filename to a tag, adding a `ConvertedMedia` entry
/// the first time a given filename is seen. `"wallpaper.png"` reuses the tag `discover_media`
/// already assigned the pack's primary wallpaper (no duplicate media entry); any other filename
/// mints a fresh `corruption-wallpaper-<slug>` tag. Returns `None` (no override -- the previous
/// level's wallpaper, or `Content::wallpaper_tags`, stays in effect) if the referenced file
/// doesn't actually exist, after warning.
fn resolve_wallpaper_tag(
    file: &str,
    source: &dyn PackSource,
    media: &mut Vec<ConvertedMedia>,
    used_ids: &mut HashSet<String>,
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

    let tag = format!("corruption-wallpaper-{}", unique_slug(file, used_ids));
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
/// `pack_has_experience`. `design` is out of this bullet's scope (`DesignValues` isn't a
/// chance-per-tick concept the way `FrequencyAnchors` is -- see `design/release-plan.md`'s M4
/// converter bullet).
fn build_experience(
    corruption_levels: &[CorruptionLevel],
    config: &Map<String, Value>,
    source: &dyn PackSource,
    media: &mut Vec<ConvertedMedia>,
    warnings: &mut Vec<Warning>,
) -> Option<Experience> {
    let anchors = build_anchors(config);
    let timeline = build_timeline(corruption_levels, config, source, media, warnings);

    if anchors == FrequencyAnchors::default() && timeline.is_none() {
        return None;
    }

    Some(Experience {
        anchors,
        design: DesignValues::default(),
        timeline,
    })
}

#[cfg(test)]
mod tests {
    use crate::source::DirSource;

    use super::*;

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
        // 100% chance every 5000ms tick -> one event every 5 seconds.
        assert_eq!(
            output.behaviour.experience.as_ref().unwrap().anchors.popup,
            Some(5.0)
        );
        assert_eq!(
            output.metadata.recommended_mode,
            Some(RecommendedMode::Sandbox)
        );
        assert_eq!(output.behaviour.experience.as_ref().unwrap().timeline, None);
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
            output.behaviour.experience.as_ref().unwrap().anchors.popup,
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
    fn missing_delay_falls_back_to_edgewares_own_default() {
        let (_dir, source) = source_with(&[("config.json", r#"{"promptMod": 100}"#)]);
        let output = convert(&source);
        // 100% chance every (default) 5000ms -> one event every 5 seconds.
        assert_eq!(
            output.behaviour.experience.as_ref().unwrap().anchors.prompt,
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
        let timeline = output
            .behaviour
            .experience
            .as_ref()
            .unwrap()
            .timeline
            .as_ref()
            .unwrap();
        assert_eq!(timeline.levels.len(), 2);
        // Level 1 is applied immediately (Edgeware applies it at session start, not after one
        // corruption_time interval) -- see `build_timeline`.
        assert_eq!(timeline.levels[0].at_seconds, 0.0);
        assert_eq!(
            timeline.levels[0].modifiers.tags,
            Some(vec!["angel".to_string(), "apple".to_string()])
        );
        // Default corruption_time (config.json absent) is Edgeware's own 60s default.
        assert_eq!(timeline.levels[1].at_seconds, 60.0);
        // "apple" removed, "globe" added -- cumulative, not a replacement.
        assert_eq!(
            timeline.levels[1].modifiers.tags,
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
        let timeline = output.behaviour.experience.unwrap().timeline.unwrap();
        assert_eq!(timeline.levels[0].at_popups, None);
        assert_eq!(timeline.levels[1].at_popups, Some(10));
    }

    #[test]
    fn timed_trigger_never_sets_at_popups() {
        let (_dir, source) = source_with(&[(
            "corruption.json",
            r#"{"moods": {"1": {"add": [], "remove": []}, "2": {"add": [], "remove": []}}, "wallpapers": {}, "config": {}}"#,
        )]);
        let output = convert(&source);
        let timeline = output.behaviour.experience.unwrap().timeline.unwrap();
        assert!(timeline.levels.iter().all(|l| l.at_popups.is_none()));
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
        assert!(output.behaviour.experience.unwrap().timeline.is_some());
    }

    #[test]
    fn per_level_config_override_warns_and_is_dropped() {
        let (_dir, source) = source_with(&[(
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {}, "config": {"1": {"promptMod": 0}}}"#,
        )]);
        let output = convert(&source);
        let timeline = output.behaviour.experience.unwrap().timeline.unwrap();
        assert_eq!(timeline.levels[0].modifiers.modifier, None);
        assert!(
            output
                .warnings
                .iter()
                .any(|w| w.kind == WarningKind::UnsupportedFeatureDropped
                    && w.message.contains("promptMod"))
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
        let timeline = output.behaviour.experience.unwrap().timeline.unwrap();
        assert_eq!(
            timeline.levels[0].modifiers.wallpaper_tags,
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
        let timeline = output.behaviour.experience.unwrap().timeline.unwrap();
        let tags = timeline.levels[0].modifiers.wallpaper_tags.clone().unwrap();
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
        let timeline = output.behaviour.experience.unwrap().timeline.unwrap();
        assert_eq!(timeline.levels[0].modifiers.wallpaper_tags, None);
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

        let timeline = output.behaviour.experience.unwrap().timeline.unwrap();
        for level in &timeline.levels {
            assert!(
                level
                    .modifiers
                    .tags
                    .as_ref()
                    .unwrap()
                    .contains(&MOODLESS_TAG.to_string()),
                "expected every level to keep mood-less media eligible: {level:?}"
            );
        }
        // Never removed by an unrelated mood change.
        assert!(
            timeline.levels[1]
                .modifiers
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
        let timeline = output.behaviour.experience.unwrap().timeline.unwrap();
        assert!(
            !timeline.levels[0]
                .modifiers
                .tags
                .as_ref()
                .unwrap()
                .contains(&MOODLESS_TAG.to_string())
        );
    }
}
