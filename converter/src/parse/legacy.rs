use serde::Deserialize;
use serde_json::Value;

use crate::model::{EdgewareIndex, Warning, WarningKind, WebEntry};
use crate::source::PackSource;

use super::try_load_json5;

/// Loads the four legacy files (`captions.json`, `media.json`, `prompt.json`, `web.json`) into
/// the unified internal model -- mirrors `load_index_fallback`
/// (`EdgewarePlusPlus/edgeware/src/pack/load.py`). Each file is independently optional; a
/// mood referenced by any one of them (even one with no media at all, e.g. a caption-only
/// intensity label) still gets an `EdgewareMood`/`ContentGroup`, matching Python's
/// `get_or_add_mood`. Processed in the order media -> captions -> prompts -> web so a mood's
/// first appearance (and so its position in the final `moods` list) follows a deterministic,
/// human-legible order instead of Python's incidental (hash-order) one.
pub fn load(source: &dyn PackSource, warnings: &mut Vec<Warning>) -> EdgewareIndex {
    let mut index = EdgewareIndex::default();
    load_media(source, &mut index, warnings);
    load_captions(source, &mut index, warnings);
    load_prompts(source, &mut index, warnings);
    load_web(source, &mut index, warnings);
    index
}

fn as_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn as_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn warn_malformed_object(warnings: &mut Vec<Warning>, file: &str) {
    warnings.push(Warning::new(
        WarningKind::MalformedSource,
        format!("{file} is not a JSON object; treated as absent"),
    ));
}

fn warn_if_tuned(max_clicks: u32, mood_label: &str, warnings: &mut Vec<Warning>) {
    if max_clicks > 1 {
        warnings.push(Warning::new(
            WarningKind::UnsupportedFeatureDropped,
            format!(
                "\"{mood_label}\"'s multi-click dismissal (max: {max_clicks}) isn't supported and was dropped"
            ),
        ));
    }
}

/// Edgeware's own "no suffix" encoding is an args list that's empty or a single empty string;
/// normalize both down to `vec![]` to match the schema's "empty means unmodified" docs.
fn normalize_args(args: Vec<String>) -> Vec<String> {
    if args.iter().all(|a| a.is_empty()) {
        Vec::new()
    } else {
        args
    }
}

fn load_media(source: &dyn PackSource, index: &mut EdgewareIndex, warnings: &mut Vec<Warning>) {
    let Some(media) = try_load_json5::<Value>(source, "media.json", warnings) else {
        return;
    };
    let Some(obj) = media.as_object() else {
        warn_malformed_object(warnings, "media.json");
        return;
    };

    for (mood_name, files) in obj {
        if mood_name == "default" {
            continue;
        }
        index.mood_index(mood_name);
        for file in as_string_array(Some(files)) {
            index.media_moods.insert(file, mood_name.clone());
        }
    }
}

fn load_captions(source: &dyn PackSource, index: &mut EdgewareIndex, warnings: &mut Vec<Warning>) {
    let Some(captions) = try_load_json5::<Value>(source, "captions.json", warnings) else {
        return;
    };
    let Some(obj) = captions.as_object() else {
        warn_malformed_object(warnings, "captions.json");
        return;
    };

    index.default.captions = as_string_array(obj.get("default"));
    index.default.denial = as_string_array(obj.get("denial"));
    index.default.subliminals = as_string_array(obj.get("subliminals"));
    index.default.notifications = as_string_array(obj.get("notifications"));
    if let Some(popup_close) = as_string(obj.get("subtext")) {
        index.default_extra.popup_close = Some(popup_close);
    }

    if !index.default.denial.is_empty() {
        warnings.push(Warning::new(
            WarningKind::UnsupportedFeatureDropped,
            format!(
                "{} denial caption(s) for \"default\" were dropped (denial/blur isn't supported yet)",
                index.default.denial.len()
            ),
        ));
    }

    let prefix_settings = obj.get("prefix_settings").and_then(Value::as_object);

    // A prefix listed here without a matching top-level array key gets treated as "no captions
    // for this mood" rather than discarding the whole file -- Python's stricter voluptuous
    // schema requires an exact match and would reject the entire file, but that's harsher than
    // this crate's general "parse leniently" stance warrants for one sloppy prefix name.
    for name in as_string_array(obj.get("prefix")) {
        if name == "default" {
            continue;
        }

        let captions_for_prefix = as_string_array(obj.get(name.as_str()));
        let idx = index.mood_index(&name);
        index.moods[idx].base.captions = captions_for_prefix;

        let max_clicks = prefix_settings
            .and_then(|settings| settings.get(name.as_str()))
            .and_then(|settings| settings.get("max"))
            .and_then(Value::as_u64)
            .unwrap_or(1) as u32;
        warn_if_tuned(max_clicks, &name, warnings);
    }
}

fn load_prompts(source: &dyn PackSource, index: &mut EdgewareIndex, warnings: &mut Vec<Warning>) {
    let Some(prompts) = try_load_json5::<Value>(source, "prompt.json", warnings) else {
        return;
    };
    let Some(obj) = prompts.as_object() else {
        warn_malformed_object(warnings, "prompt.json");
        return;
    };

    // `commandtext` (-> prompt_command) and `minLen`/`maxLen` are dropped silently, same as the
    // modern layout's `promptCommand`/`promptMinLength`/`promptMaxLength` -- see
    // `parse::modern` for why. `freqList` (per-mood weighting) is dropped silently too, for a
    // different reason: reading `EdgewarePlusPlus/edgeware/src/pack/load.py`'s
    // `load_index_fallback` shows it's validated but never actually assigned to anything --
    // it's dead in upstream Edgeware++ itself, so converting it would warn about a feature that
    // never did anything.
    index.default.prompts = as_string_array(obj.get("default"));
    if let Some(prompt_submit) = as_string(obj.get("subtext")) {
        index.default_extra.prompt_submit = Some(prompt_submit);
    }

    for name in as_string_array(obj.get("moods")) {
        if name == "default" {
            continue;
        }
        let prompts_for_mood = as_string_array(obj.get(name.as_str()));
        let idx = index.mood_index(&name);
        index.moods[idx].base.prompts = prompts_for_mood;
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RawWeb {
    urls: Vec<String>,
    args: Vec<String>,
    moods: Vec<String>,
}

fn load_web(source: &dyn PackSource, index: &mut EdgewareIndex, warnings: &mut Vec<Warning>) {
    let Some(raw) = try_load_json5::<RawWeb>(source, "web.json", warnings) else {
        return;
    };

    for (i, url) in raw.urls.into_iter().enumerate() {
        // Legacy `args` is one comma-separated string *per url* (unlike the modern layout's
        // `webArgs: [[str]]`) -- must split, matching `web["args"][i].split(",")` exactly.
        let args_str = raw.args.get(i).cloned().unwrap_or_default();
        let args = normalize_args(args_str.split(',').map(str::to_string).collect());
        let entry = WebEntry { url, args };

        match raw.moods.get(i).map(String::as_str) {
            None | Some("default") => index.default.web.push(entry),
            Some(name) => {
                let idx = index.mood_index(name);
                index.moods[idx].base.web.push(entry);
            }
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
            std::fs::write(dir.path().join(name), contents).unwrap();
        }
        let source = DirSource::new(dir.path());
        (dir, source)
    }

    #[test]
    fn media_json_tags_files_and_excludes_default_bucket() {
        let (_dir, source) = source_with(&[(
            "media.json",
            r#"{"default": ["untagged.mp4"], "vanilla": ["a.png", "b.png"]}"#,
        )]);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index.media_moods.get("a.png"), Some(&"vanilla".to_string()));
        assert_eq!(index.media_moods.get("b.png"), Some(&"vanilla".to_string()));
        assert_eq!(index.media_moods.get("untagged.mp4"), None);
        assert_eq!(index.moods.len(), 1);
        assert_eq!(index.moods[0].name, "vanilla");
        assert!(warnings.is_empty());
    }

    #[test]
    fn captions_json_default_and_prefixes() {
        let (_dir, source) = source_with(&[(
            "captions.json",
            r#"{
                "prefix": ["low", "high"],
                "prefix_settings": {"high": {"max": 3}},
                "default": ["default caption"],
                "subtext": "Click to close!",
                "low": ["low caption"],
                "high": ["high caption"]
            }"#,
        )]);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index.default.captions, vec!["default caption".to_string()]);
        assert_eq!(
            index.default_extra.popup_close.as_deref(),
            Some("Click to close!")
        );
        let low = index.moods.iter().find(|m| m.name == "low").unwrap();
        assert_eq!(low.base.captions, vec!["low caption".to_string()]);
        let high = index.moods.iter().find(|m| m.name == "high").unwrap();
        assert_eq!(high.base.captions, vec!["high caption".to_string()]);
        // max: 3 on "high" -> one UnsupportedFeatureDropped warning, "low" stays silent.
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnsupportedFeatureDropped);
    }

    #[test]
    fn captions_json_denial_warns() {
        let (_dir, source) =
            source_with(&[("captions.json", r#"{"default": [], "denial": ["no"]}"#)]);
        let mut warnings = Vec::new();

        load(&source, &mut warnings);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnsupportedFeatureDropped);
    }

    #[test]
    fn prompt_json_default_and_moods_ignoring_freq_and_lengths() {
        let (_dir, source) = source_with(&[(
            "prompt.json",
            r#"{
                "moods": ["default", "high"],
                "minLen": 1, "maxLen": 1, "freqList": [100, 50],
                "subtext": "I Submit <3",
                "default": ["default prompt"],
                "high": ["high prompt"]
            }"#,
        )]);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index.default.prompts, vec!["default prompt".to_string()]);
        assert_eq!(
            index.default_extra.prompt_submit.as_deref(),
            Some("I Submit <3")
        );
        let high = index.moods.iter().find(|m| m.name == "high").unwrap();
        assert_eq!(high.base.prompts, vec!["high prompt".to_string()]);
        // "default" in `moods` is skipped (already covered by the `default` key directly), and
        // freqList/minLen/maxLen are dead/dropped -- no warnings at all.
        assert!(warnings.is_empty());
    }

    #[test]
    fn web_json_splits_comma_args_and_tags_by_mood() {
        let (_dir, source) = source_with(&[(
            "web.json",
            r#"{
                "urls": ["https://a", "https://b", "https://c"],
                "args": ["x,y", "", "z"],
                "moods": ["google", "default", "github"]
            }"#,
        )]);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        let google = index.moods.iter().find(|m| m.name == "google").unwrap();
        assert_eq!(google.base.web[0].url, "https://a");
        assert_eq!(
            google.base.web[0].args,
            vec!["x".to_string(), "y".to_string()]
        );

        // "default" mood entry -> untagged, and empty args string normalizes away.
        assert_eq!(index.default.web[0].url, "https://b");
        assert!(index.default.web[0].args.is_empty());

        let github = index.moods.iter().find(|m| m.name == "github").unwrap();
        assert_eq!(github.base.web[0].args, vec!["z".to_string()]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn web_json_untagged_when_moods_array_absent() {
        let (_dir, source) =
            source_with(&[("web.json", r#"{"urls": ["https://a"], "args": [""]}"#)]);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index.default.web.len(), 1);
        assert!(index.moods.is_empty());
    }

    #[test]
    fn mood_only_referenced_by_prompts_still_gets_a_mood_entry() {
        let (_dir, source) = source_with(&[(
            "prompt.json",
            r#"{"moods": ["intensity"], "intensity": ["only a prompt"]}"#,
        )]);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index.moods.len(), 1);
        assert_eq!(index.moods[0].name, "intensity");
        assert!(warnings.is_empty());
    }

    #[test]
    fn all_files_missing_returns_empty_index_silently() {
        let dir = tempfile::tempdir().unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index, EdgewareIndex::default());
        assert!(warnings.is_empty());
    }
}
