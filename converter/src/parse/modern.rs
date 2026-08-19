use serde::Deserialize;

use crate::model::{DefaultExtra, EdgewareIndex, MoodBase, Warning, WarningKind, WebEntry};
use crate::source::PackSource;

use super::try_load_json5;

/// The pools shared by `default` and every mood entry -- mirrors Python's `base_schema`
/// (`EdgewarePlusPlus/edgeware/src/pack/load.py`).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct RawBase {
    captions: Vec<String>,
    denial: Vec<String>,
    subliminals: Vec<String>,
    notifications: Vec<String>,
    prompts: Vec<String>,
    web: Vec<String>,
    #[serde(rename = "webArgs")]
    web_args: Vec<Vec<String>>,
    #[serde(rename = "maxClicks", default = "default_max_clicks")]
    max_clicks: u32,
}

impl Default for RawBase {
    fn default() -> Self {
        Self {
            captions: Vec::new(),
            denial: Vec::new(),
            subliminals: Vec::new(),
            notifications: Vec::new(),
            prompts: Vec::new(),
            web: Vec::new(),
            web_args: Vec::new(),
            max_clicks: 1,
        }
    }
}

fn default_max_clicks() -> u32 {
    1
}

/// `promptCommand`/`promptMinLength`/`promptMaxLength`/`promptSubmit` are deliberately not parsed
/// here at all -- all of them are dropped silently (no schema equivalent worth having yet; see the
/// plan doc's rationale, and for `promptSubmit` the fixed "Submit" button in
/// `default-modes/shared/lib/prompts.lua`), so there's nothing to do with them even if we read
/// them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RawDefault {
    #[serde(flatten)]
    base: RawBase,
    #[serde(rename = "popupClose")]
    popup_close: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawMood {
    mood: String,
    #[serde(flatten)]
    base: RawBase,
    #[serde(default)]
    media: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RawIndex {
    default: RawDefault,
    moods: Vec<RawMood>,
}

/// Loads `index.json` (the modern layout) into the unified internal model. Callers decide
/// whether to try this at all -- see `parse::load_edgeware_index`'s file-existence check, since
/// a malformed `index.json` should warn and stop, not silently fall back to the legacy layout.
pub fn load(source: &dyn PackSource, warnings: &mut Vec<Warning>) -> EdgewareIndex {
    let raw: RawIndex = try_load_json5(source, "index.json", warnings).unwrap_or_default();
    build_index(raw, warnings)
}

fn build_index(raw: RawIndex, warnings: &mut Vec<Warning>) -> EdgewareIndex {
    warn_if_tuned(raw.default.base.max_clicks, "default", warnings);

    let mut index = EdgewareIndex {
        default_extra: DefaultExtra {
            popup_close: non_empty(raw.default.popup_close),
        },
        default: convert_base(raw.default.base, "default", warnings),
        ..Default::default()
    };

    for raw_mood in raw.moods {
        let name = raw_mood.mood;
        warn_if_tuned(raw_mood.base.max_clicks, &name, warnings);
        let base = convert_base(raw_mood.base, &name, warnings);

        let idx = index.mood_index(&name);
        index.moods[idx].base = base;

        for file in raw_mood.media {
            index.media_moods.insert(file, name.clone());
        }
    }

    index
}

fn convert_base(raw: RawBase, mood_label: &str, warnings: &mut Vec<Warning>) -> MoodBase {
    if !raw.denial.is_empty() {
        warnings.push(Warning::new(
            WarningKind::UnsupportedFeatureDropped,
            format!(
                "{} denial caption(s) for \"{mood_label}\" were dropped (denial/blur isn't supported yet)",
                raw.denial.len()
            ),
        ));
    }

    if !raw.subliminals.is_empty() {
        warnings.push(Warning::new(
            WarningKind::UnsupportedFeatureDropped,
            format!(
                "{} subliminal(s) for \"{mood_label}\" were dropped (subliminals aren't supported)",
                raw.subliminals.len()
            ),
        ));
    }

    let mut web = Vec::with_capacity(raw.web.len());
    for (i, url) in raw.web.into_iter().enumerate() {
        let args = raw.web_args.get(i).cloned().unwrap_or_default();
        web.push(WebEntry {
            url,
            args: normalize_args(args),
        });
    }

    MoodBase {
        captions: raw.captions,
        denial: raw.denial,
        subliminals: raw.subliminals,
        notifications: raw.notifications,
        prompts: raw.prompts,
        web,
    }
}

fn warn_if_tuned(max_clicks: u32, mood_label: &str, warnings: &mut Vec<Warning>) {
    if max_clicks > 1 {
        warnings.push(Warning::new(
            WarningKind::UnsupportedFeatureDropped,
            format!(
                "\"{mood_label}\"'s multi-click dismissal (maxClicks: {max_clicks}) isn't supported and was dropped"
            ),
        ));
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.is_empty())
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

#[cfg(test)]
mod tests {
    use crate::source::DirSource;

    use super::*;

    fn source_with_index(json: &str) -> (tempfile::TempDir, DirSource) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.json"), json).unwrap();
        let source = DirSource::new(dir.path());
        (dir, source)
    }

    #[test]
    fn parses_default_and_moods() {
        let (_dir, source) = source_with_index(
            r#"{
                "default": { "captions": ["hi"], "web": ["https://a"], "webArgs": [["x"]] },
                "moods": [
                    { "mood": "vanilla", "captions": ["v1"], "media": ["a.png", "b.png"] }
                ]
            }"#,
        );
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index.default.captions, vec!["hi".to_string()]);
        assert_eq!(index.default.web[0].url, "https://a");
        assert_eq!(index.default.web[0].args, vec!["x".to_string()]);
        assert_eq!(index.moods.len(), 1);
        assert_eq!(index.moods[0].name, "vanilla");
        assert_eq!(index.moods[0].base.captions, vec!["v1".to_string()]);
        assert_eq!(index.media_moods.get("a.png"), Some(&"vanilla".to_string()));
        assert_eq!(index.media_moods.get("b.png"), Some(&"vanilla".to_string()));
        assert!(warnings.is_empty());
    }

    #[test]
    fn popup_close_only_set_when_present() {
        let (_dir, source) = source_with_index(r#"{"default": {}}"#);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index.default_extra.popup_close, None);
    }

    /// `promptSubmit` rides along to prove that reading past it is silent: the prompt dialog's
    /// button is always "Submit", so an authored override is neither converted nor warned about.
    #[test]
    fn popup_close_captured_when_authored_and_prompt_submit_ignored() {
        let (_dir, source) =
            source_with_index(r#"{"default": {"popupClose": "Close me", "promptSubmit": "Go"}}"#);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index.default_extra.popup_close.as_deref(), Some("Close me"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn denial_warns_and_is_dropped() {
        let (_dir, source) = source_with_index(r#"{"default": {"denial": ["no"]}}"#);
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert!(index.default.captions.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnsupportedFeatureDropped);
    }

    #[test]
    fn max_clicks_above_one_warns() {
        let (_dir, source) = source_with_index(r#"{"default": {"maxClicks": 3}}"#);
        let mut warnings = Vec::new();

        load(&source, &mut warnings);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::UnsupportedFeatureDropped);
    }

    #[test]
    fn max_clicks_of_one_is_silent() {
        let (_dir, source) = source_with_index(r#"{"default": {"maxClicks": 1}}"#);
        let mut warnings = Vec::new();

        load(&source, &mut warnings);

        assert!(warnings.is_empty());
    }

    #[test]
    fn missing_index_json_returns_empty_index_silently() {
        let dir = tempfile::tempdir().unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert_eq!(index, EdgewareIndex::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn empty_web_args_normalize_to_empty_vec() {
        let (_dir, source) = source_with_index(
            r#"{"default": {"web": ["https://a", "https://b"], "webArgs": [[""], []]}}"#,
        );
        let mut warnings = Vec::new();

        let index = load(&source, &mut warnings);

        assert!(index.default.web[0].args.is_empty());
        assert!(index.default.web[1].args.is_empty());
    }
}
