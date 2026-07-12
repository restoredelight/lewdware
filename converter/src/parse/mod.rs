pub mod legacy;
pub mod modern;

use serde::{Deserialize, de::DeserializeOwned};

use crate::{
    model::{Warning, WarningKind},
    source::PackSource,
};

/// Mirrors Python's `try_load` (`EdgewarePlusPlus/edgeware/src/pack/load.py`): `None` if `path`
/// doesn't exist in `source` (silent -- a missing file is normal, not a problem), `Some(value)`
/// if it parses, and `None` with a pushed `Warning::MalformedSource` if it exists but doesn't
/// parse (the section is then treated as absent, not fatal). Uses `json5` rather than strict
/// JSON for Edgeware's real-world sloppiness (trailing commas, `//` comments) -- see
/// `EdgewarePlusPlus/examples/index.json`.
pub fn try_load_json5<T: DeserializeOwned>(
    source: &dyn PackSource,
    path: &str,
    warnings: &mut Vec<Warning>,
) -> Option<T> {
    let bytes = source.read_file(path)?;
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(err) => {
            warnings.push(Warning::new(
                WarningKind::MalformedSource,
                format!("{path} is not valid UTF-8 ({err}); treated as absent"),
            ));
            return None;
        }
    };
    match json5::from_str(text) {
        Ok(value) => Some(value),
        Err(err) => {
            warnings.push(Warning::new(
                WarningKind::MalformedSource,
                format!("{path} is not valid JSON ({err}); treated as absent"),
            ));
            None
        }
    }
}

/// `info.json` -> pack metadata. Shared by both the modern and legacy layouts (the file's shape
/// doesn't change between them). Unlike Python's loader, each field is independently optional
/// rather than all-or-nothing -- a pack with a `name` but no `id` still gets its name through,
/// matching this crate's general "parse leniently" stance. `id` (Edgeware's own local
/// mood-file bookkeeping key) has no lewdware equivalent and isn't mapped.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InfoJson {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub creator: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

pub fn load_info(source: &dyn PackSource, warnings: &mut Vec<Warning>) -> InfoJson {
    try_load_json5(source, "info.json", warnings).unwrap_or_default()
}

/// Picks the modern (`index.json`) or legacy (`captions`/`media`/`prompt`/`web.json`) layout and
/// loads it into the unified internal model. `index.json`'s presence, not its validity, decides
/// the branch: a pack with a malformed `index.json` gets a `Warning::MalformedSource` from
/// `modern::load` and stops there -- it does *not* silently fall back to the legacy files, since
/// a pack author who wrote a broken `index.json` should be told, not routed around.
pub fn load_edgeware_index(
    source: &dyn PackSource,
    warnings: &mut Vec<Warning>,
) -> crate::model::EdgewareIndex {
    if source.file_exists("index.json") {
        modern::load(source, warnings)
    } else {
        legacy::load(source, warnings)
    }
}

#[cfg(test)]
mod index_routing_tests {
    use crate::source::DirSource;

    use super::*;

    #[test]
    fn prefers_modern_layout_when_index_json_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("index.json"),
            r#"{"default": {"captions": ["modern"]}}"#,
        )
        .unwrap();
        // Also drop a legacy file in, to prove it's ignored once index.json exists.
        std::fs::write(
            dir.path().join("captions.json"),
            r#"{"default": ["legacy"]}"#,
        )
        .unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let index = load_edgeware_index(&source, &mut warnings);

        assert_eq!(index.default.captions, vec!["modern".to_string()]);
    }

    #[test]
    fn falls_back_to_legacy_layout_when_index_json_absent() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("captions.json"),
            r#"{"default": ["legacy"]}"#,
        )
        .unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let index = load_edgeware_index(&source, &mut warnings);

        assert_eq!(index.default.captions, vec!["legacy".to_string()]);
    }

    #[test]
    fn malformed_index_json_warns_and_does_not_fall_back_to_legacy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.json"), b"not json {{{").unwrap();
        std::fs::write(
            dir.path().join("captions.json"),
            r#"{"default": ["legacy"]}"#,
        )
        .unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let index = load_edgeware_index(&source, &mut warnings);

        assert!(index.default.captions.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::MalformedSource);
    }
}

#[cfg(test)]
mod tests {
    use crate::source::DirSource;

    use super::*;

    #[test]
    fn load_info_missing_file_is_silent() {
        let dir = tempfile::tempdir().unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let info = load_info(&source, &mut warnings);

        assert_eq!(info.name, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_info_partial_file_salvages_present_fields() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("info.json"), br#"{"name": "Test Pack"}"#).unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let info = load_info(&source, &mut warnings);

        assert_eq!(info.name.as_deref(), Some("Test Pack"));
        assert_eq!(info.creator, None);
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_info_malformed_file_warns_and_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("info.json"), b"not json at all {{{").unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let info = load_info(&source, &mut warnings);

        assert_eq!(info.name, None);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, WarningKind::MalformedSource);
    }

    #[test]
    fn load_info_tolerates_trailing_commas() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("info.json"), br#"{"name": "Test Pack",}"#).unwrap();
        let source = DirSource::new(dir.path());
        let mut warnings = Vec::new();

        let info = load_info(&source, &mut warnings);

        assert_eq!(info.name.as_deref(), Some("Test Pack"));
        assert!(warnings.is_empty());
    }
}
