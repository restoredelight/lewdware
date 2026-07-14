use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Map;

use crate::model::{CorruptionLevel, Warning, WarningKind};
use crate::parse::try_load_json5;
use crate::source::PackSource;

/// Mirrors `EdgewarePlusPlus/edgeware/src/pack/load.py`'s `load_corruption` schema exactly:
/// `moods`/`wallpapers`/`config` are all keyed by the level's 1-based number as a JSON string
/// (`wallpapers` additionally allows `"default"`).
#[derive(Debug, Deserialize, Default)]
struct RawCorruption {
    #[serde(default)]
    moods: HashMap<String, RawMoodChange>,
    #[serde(default)]
    wallpapers: HashMap<String, String>,
    #[serde(default)]
    config: HashMap<String, Map<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, Default)]
struct RawMoodChange {
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

/// `corruption.json` -> a flat, level-ordered list. Empty if the file is absent (silent), or
/// malformed (`Warning::MalformedSource`, same as every other `try_load_json5` caller), or
/// present but legitimately declares zero levels (`Warning::CorruptionNotConverted` -- distinct
/// from "absent", which is unremarkable, this means the author's `corruption.json` has nothing
/// this converter could act on).
pub fn load_corruption(
    source: &dyn PackSource,
    warnings: &mut Vec<Warning>,
) -> Vec<CorruptionLevel> {
    let Some(raw) = try_load_json5::<RawCorruption>(source, "corruption.json", warnings) else {
        return Vec::new();
    };

    let has_default_wallpaper = raw.wallpapers.contains_key("default");
    let non_default_wallpapers = raw.wallpapers.len() - has_default_wallpaper as usize;
    let level_count = raw
        .moods
        .len()
        .max(non_default_wallpapers)
        .max(raw.config.len());

    if level_count == 0 {
        warnings.push(Warning::new(
            WarningKind::CorruptionNotConverted,
            "corruption.json is present but its moods/wallpapers/config sections are all empty \
             -- no timeline levels to convert",
        ));
        return Vec::new();
    }

    (0..level_count)
        .map(|i| {
            let key = (i + 1).to_string();

            let mood_change = raw.moods.get(&key);
            let wallpaper = raw.wallpapers.get(&key).cloned().or_else(|| {
                (i == 0)
                    .then(|| raw.wallpapers.get("default").cloned())
                    .flatten()
            });
            let config = raw.config.get(&key).cloned().unwrap_or_default();

            CorruptionLevel {
                added_moods: mood_change.map(|m| m.add.clone()).unwrap_or_default(),
                removed_moods: mood_change.map(|m| m.remove.clone()).unwrap_or_default(),
                wallpaper,
                config,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::model::WarningKind;
    use crate::source::DirSource;

    use super::*;

    fn source_with(files: &[(&str, &str)]) -> (tempfile::TempDir, DirSource) {
        let dir = tempfile::tempdir().unwrap();
        for (name, contents) in files {
            std::fs::write(dir.path().join(name), contents).unwrap();
        }
        let source = DirSource::new(dir.path());
        (dir, source)
    }

    #[test]
    fn absent_file_is_silent_and_empty() {
        let (_dir, source) = source_with(&[]);
        let mut warnings = Vec::new();
        let levels = load_corruption(&source, &mut warnings);
        assert!(levels.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn malformed_file_warns_and_returns_empty() {
        let (_dir, source) = source_with(&[("corruption.json", "not json {{{")]);
        let mut warnings = Vec::new();
        let levels = load_corruption(&source, &mut warnings);
        assert!(levels.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::MalformedSource);
    }

    #[test]
    fn empty_sections_produce_zero_levels_and_warn() {
        let (_dir, source) = source_with(&[(
            "corruption.json",
            r#"{"moods": {}, "wallpapers": {}, "config": {}}"#,
        )]);
        let mut warnings = Vec::new();
        let levels = load_corruption(&source, &mut warnings);
        assert!(levels.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::CorruptionNotConverted);
    }

    #[test]
    fn level_count_is_the_max_across_sections() {
        // 2 mood levels, 1 (non-default) wallpaper level, 3 config levels -> 3 levels total.
        let (_dir, source) = source_with(&[(
            "corruption.json",
            r#"{
                "moods": {"1": {"add": ["a"], "remove": []}, "2": {"add": ["b"], "remove": []}},
                "wallpapers": {"1": "bg1.png", "default": "bg0.png"},
                "config": {"1": {"promptMod": 0}, "2": {}, "3": {"promptMod": 10}}
            }"#,
        )]);
        let mut warnings = Vec::new();
        let levels = load_corruption(&source, &mut warnings);
        assert_eq!(levels.len(), 3);
    }

    #[test]
    fn cumulative_add_remove_is_captured_per_level_not_folded_here() {
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
        let mut warnings = Vec::new();
        let levels = load_corruption(&source, &mut warnings);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].added_moods, vec!["angel", "apple"]);
        assert!(levels[0].removed_moods.is_empty());
        assert_eq!(levels[1].added_moods, vec!["globe"]);
        assert_eq!(levels[1].removed_moods, vec!["apple"]);
    }

    #[test]
    fn level_one_wallpaper_falls_back_to_default_only_when_unset() {
        let (_dir, source) = source_with(&[(
            "corruption.json",
            r#"{
                "moods": {},
                "wallpapers": {"1": "explicit.png", "2": "explicit2.png", "default": "fallback.png"},
                "config": {}
            }"#,
        )]);
        let mut warnings = Vec::new();
        let levels = load_corruption(&source, &mut warnings);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].wallpaper.as_deref(), Some("explicit.png"));
        assert_eq!(levels[1].wallpaper.as_deref(), Some("explicit2.png"));
    }

    #[test]
    fn only_level_one_uses_the_default_wallpaper_fallback() {
        let (_dir, source) = source_with(&[(
            "corruption.json",
            r#"{
                "moods": {"1": {"add": [], "remove": []}, "2": {"add": [], "remove": []}},
                "wallpapers": {"default": "fallback.png"},
                "config": {}
            }"#,
        )]);
        let mut warnings = Vec::new();
        let levels = load_corruption(&source, &mut warnings);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].wallpaper.as_deref(), Some("fallback.png"));
        assert_eq!(levels[1].wallpaper, None);
    }

    #[test]
    fn config_values_are_captured_not_just_key_names() {
        let (_dir, source) = source_with(&[(
            "corruption.json",
            r#"{
                "moods": {},
                "wallpapers": {},
                "config": {"1": {"promptMistakes": 10, "promptMod": 10}}
            }"#,
        )]);
        let mut warnings = Vec::new();
        let levels = load_corruption(&source, &mut warnings);
        assert_eq!(levels.len(), 1);
        assert_eq!(
            levels[0].config.get("promptMod"),
            Some(&serde_json::json!(10))
        );
        assert_eq!(
            levels[0].config.get("promptMistakes"),
            Some(&serde_json::json!(10))
        );
    }
}
