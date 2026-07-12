use std::collections::HashSet;

use serde::Serialize;
use serde_json::Value;
use shared::behaviour::{Behaviour, Content, ContentGroup, PromptSettings, TextItem, WebLink};
use shared::read_pack::Metadata;

use crate::model::{EdgewareIndex, EdgewareMood, Warning, WarningKind};
use crate::parse::{self, InfoJson, try_load_json5};
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
/// pack metadata, and a behaviour.json `Content` section. Scoped to content only --
/// `corruption.json` -> timeline and `config.json` -> frequency anchors are M4 work (see
/// `design/release-plan.md`); their presence is only *noted* here via a warning.
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

    let metadata = build_metadata(info);

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

    check_unsupported_files(source, &mut warnings);

    let icon = source
        .file_exists("icon.ico")
        .then(|| "icon.ico".to_string());

    ConversionOutput {
        metadata,
        behaviour: Behaviour {
            content,
            ..Behaviour::new()
        },
        media,
        icon,
        warnings,
    }
}

fn build_metadata(info: InfoJson) -> Metadata {
    Metadata {
        name: info.name.unwrap_or_else(|| "Unnamed Pack".to_string()),
        creator: info.creator,
        description: info.description,
        version: info.version,
        // M4 emits this (corruption.json presence -> Experience); M3 never recommends a mode.
        recommended_mode: None,
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
    if source.file_exists("corruption.json") {
        warnings.push(Warning::new(
            WarningKind::CorruptionNotConverted,
            "corruption.json is present but timeline conversion isn't implemented yet (planned for M4)",
        ));
    }
    if let Some(config) = try_load_json5::<Value>(source, "config.json", warnings) {
        let has_keys = config.as_object().is_some_and(|obj| !obj.is_empty());
        if has_keys {
            warnings.push(Warning::new(
                WarningKind::ConfigNotConverted,
                "config.json is present but frequency-anchor conversion isn't implemented yet (planned for M4)",
            ));
        }
    }
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
}
