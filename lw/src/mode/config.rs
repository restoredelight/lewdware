use std::{collections::HashSet, path::PathBuf};

use anyhow::{Context, Result, bail};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use shared::mode;

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub include: Vec<PathBuf>,
    pub name: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub modes: IndexMap<String, Mode>,
}

#[derive(Serialize, Deserialize)]
pub struct Mode {
    pub name: String,
    pub entrypoint: String,
    #[serde(default)]
    pub options: IndexMap<String, JsonValue>,
}

/// A flat option entry as parsed from JSONC.
#[derive(Serialize, Deserialize)]
pub struct ModeOption {
    pub label: String,
    pub description: Option<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub enabled_by_default: bool,
    #[serde(default)]
    pub show_when: Option<IndexMap<String, JsonValue>>,
    #[serde(flatten)]
    pub option_type: OptionType,
}

/// A group entry as parsed from JSONC.
#[derive(Serialize, Deserialize)]
pub struct GroupConfig {
    pub label: String,
    pub description: Option<String>,
    #[serde(default)]
    pub show_when: Option<IndexMap<String, JsonValue>>,
    pub options: IndexMap<String, JsonValue>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OptionType {
    #[serde(rename = "integer")]
    Integer {
        default: Option<i64>,
        min: Option<i64>,
        max: Option<i64>,
        step: Option<i64>,
        #[serde(default)]
        clamp: bool,
        slider: Option<bool>,
    },
    #[serde(rename = "number")]
    Number {
        default: Option<f64>,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
        #[serde(default)]
        clamp: bool,
        slider: Option<bool>,
    },
    #[serde(rename = "string")]
    String { default: Option<String> },
    #[serde(rename = "boolean")]
    Boolean { default: Option<bool> },
    #[serde(rename = "enum")]
    Enum {
        default: Option<String>,
        values: IndexMap<String, String>,
    },
}

// ─── Parsing ─────────────────────────────────────────────────────────────────

/// Parse a raw JSON options map into `mode::ModeEntry` values, validating that
/// all option keys are globally unique across groups.
pub fn parse_entries(
    raw: IndexMap<String, JsonValue>,
) -> Result<IndexMap<String, mode::ModeEntry>> {
    let entries = raw
        .into_iter()
        .map(|(key, value)| {
            let entry = parse_entry(&key, value)
                .with_context(|| format!("Error in entry `{key}`"))?;
            Ok((key, entry))
        })
        .collect::<Result<IndexMap<_, _>>>()?;

    // Validate uniqueness of option keys across groups
    let mut seen = HashSet::new();
    validate_unique_option_keys(&entries, &mut seen)?;

    Ok(entries)
}

fn parse_entry(key: &str, value: JsonValue) -> Result<mode::ModeEntry> {
    let type_str = value
        .get("type")
        .and_then(|v| v.as_str())
        .with_context(|| format!("`{key}` is missing a `type` field"))?;

    if type_str == "group" {
        let group: GroupConfig = serde_json::from_value(value)
            .with_context(|| format!("Error parsing group `{key}`"))?;
        let entries = group
            .options
            .into_iter()
            .map(|(k, v)| {
                let entry = parse_entry(&k, v)
                    .with_context(|| format!("Error in entry `{k}`"))?;
                Ok((k, entry))
            })
            .collect::<Result<IndexMap<_, _>>>()?;
        let show_when = parse_show_when(group.show_when)?;
        Ok(mode::ModeEntry::Group(mode::ModeGroup {
            label: group.label,
            description: group.description,
            show_when,
            entries,
        }))
    } else {
        let ModeOption {
            label,
            description,
            optional,
            enabled_by_default,
            show_when: raw_show_when,
            option_type,
        } = serde_json::from_value(value)
            .with_context(|| format!("Error parsing option `{key}`"))?;
        let show_when = parse_show_when(raw_show_when)?;
        let meta_opt =
            convert_option(key, label, description, optional, enabled_by_default, option_type, show_when)?;
        Ok(mode::ModeEntry::Option(meta_opt))
    }
}

fn convert_option(
    key: &str,
    label: String,
    description: Option<String>,
    optional: bool,
    enabled_by_default: bool,
    option_type: OptionType,
    show_when: Option<mode::ShowWhen>,
) -> Result<mode::ModeOption> {
    let has_default = match &option_type {
        OptionType::Integer { default, .. } => default.is_some(),
        OptionType::Number { default, .. } => default.is_some(),
        OptionType::String { default } => default.is_some(),
        OptionType::Boolean { default } => default.is_some(),
        OptionType::Enum { default, .. } => default.is_some(),
    };
    if !has_default {
        bail!("`{label}` has no default value");
    }

    let option_type = option_type
        .try_into()
        .with_context(|| format!("Error in `{key}`"))?;

    Ok(mode::ModeOption {
        label,
        description,
        option_type,
        optional,
        enabled_by_default,
        show_when,
    })
}

fn parse_show_when(
    raw: Option<IndexMap<String, JsonValue>>,
) -> Result<Option<mode::ShowWhen>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let result = raw
        .into_iter()
        .map(|(key, value)| {
            let cond = match value {
                JsonValue::Bool(b) => mode::ConditionValue::Bool(b),
                JsonValue::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        mode::ConditionValue::Int(i)
                    } else {
                        mode::ConditionValue::Float(n.as_f64().unwrap_or(0.0))
                    }
                }
                JsonValue::String(s) => mode::ConditionValue::Str(s),
                other => bail!(
                    "invalid show_when value for key `{key}`: expected bool, number, or string, got {other}"
                ),
            };
            Ok((key, cond))
        })
        .collect::<Result<IndexMap<_, _>>>()?;
    Ok(Some(result))
}

fn validate_unique_option_keys(
    entries: &IndexMap<String, mode::ModeEntry>,
    seen: &mut HashSet<String>,
) -> Result<()> {
    for (key, entry) in entries {
        match entry {
            mode::ModeEntry::Option(_) => {
                if !seen.insert(key.clone()) {
                    bail!("duplicate option key `{key}`");
                }
            }
            mode::ModeEntry::Group(group) => {
                validate_unique_option_keys(&group.entries, seen)?;
            }
        }
    }
    Ok(())
}

// ─── OptionType conversion ────────────────────────────────────────────────────

impl TryFrom<OptionType> for mode::OptionType {
    type Error = anyhow::Error;

    fn try_from(value: OptionType) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            OptionType::Integer {
                default,
                min,
                max,
                step,
                clamp,
                slider,
            } => Self::Integer {
                default: default.unwrap_or(0),
                min,
                max,
                step,
                clamp,
                slider: slider.unwrap_or_else(|| min.is_some() && max.is_some()),
            },
            OptionType::Number {
                default,
                min,
                max,
                step,
                clamp,
                slider,
            } => Self::Number {
                default: default.unwrap_or(0.0),
                min,
                max,
                step,
                clamp,
                slider: slider.unwrap_or_else(|| min.is_some() && max.is_some()),
            },
            OptionType::String { default } => Self::String {
                default: default.unwrap_or_default(),
            },
            OptionType::Boolean { default } => Self::Boolean {
                default: default.unwrap_or(false),
            },
            OptionType::Enum { default, values } => {
                if let Some(ref d) = default {
                    if !values.keys().any(|key| key == d) {
                        bail!("`default` ('{d}') is not a valid option");
                    }
                }
                Self::Enum {
                    default: default.unwrap_or_default(),
                    values,
                }
            }
        })
    }
}

impl OptionType {
    pub fn validate(&self) -> Result<()> {
        if let Self::Enum { default, values } = self {
            if let Some(d) = default {
                if !values.keys().any(|key| key == d) {
                    bail!("`default` ('{d}') does not match any options");
                }
            }
        }
        Ok(())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_enum_value(default: &str, values: &[&str]) -> JsonValue {
        let values_obj: serde_json::Map<String, JsonValue> = values
            .iter()
            .map(|v| (v.to_string(), JsonValue::String(v.to_string())))
            .collect();
        serde_json::json!({
            "label": "test",
            "type": "enum",
            "default": default,
            "values": values_obj,
        })
    }

    #[test]
    fn enum_valid_default_converts() {
        let mut map = IndexMap::new();
        map.insert("opt".to_string(), make_enum_value("a", &["a", "b"]));
        let result = parse_entries(map);
        assert!(result.is_ok());
    }

    #[test]
    fn enum_invalid_default_rejected() {
        let mut map = IndexMap::new();
        map.insert("opt".to_string(), make_enum_value("z", &["a", "b"]));
        let result = parse_entries(map);
        assert!(result.is_err());
    }

    #[test]
    fn slider_defaults_true_when_min_and_max_set() {
        let opt = OptionType::Integer {
            default: Some(0),
            min: Some(0),
            max: Some(10),
            step: None,
            clamp: false,
            slider: None,
        };
        let converted: mode::OptionType = opt.try_into().unwrap();
        assert!(matches!(
            converted,
            mode::OptionType::Integer { slider: true, .. }
        ));
    }

    #[test]
    fn slider_defaults_false_when_only_min_set() {
        let opt = OptionType::Integer {
            default: Some(0),
            min: Some(0),
            max: None,
            step: None,
            clamp: false,
            slider: None,
        };
        let converted: mode::OptionType = opt.try_into().unwrap();
        assert!(matches!(
            converted,
            mode::OptionType::Integer { slider: false, .. }
        ));
    }

    #[test]
    fn explicit_slider_false_overrides_min_max() {
        let opt = OptionType::Integer {
            default: Some(0),
            min: Some(0),
            max: Some(10),
            step: None,
            clamp: false,
            slider: Some(false),
        };
        let converted: mode::OptionType = opt.try_into().unwrap();
        assert!(matches!(
            converted,
            mode::OptionType::Integer { slider: false, .. }
        ));
    }

    #[test]
    fn duplicate_option_key_across_groups_rejected() {
        let raw = serde_json::json!({
            "count": { "label": "Count", "type": "integer", "default": 1 },
            "grp": {
                "label": "Group",
                "type": "group",
                "options": {
                    "count": { "label": "Count again", "type": "integer", "default": 2 }
                }
            }
        });
        let map: IndexMap<String, JsonValue> = serde_json::from_value(raw).unwrap();
        assert!(parse_entries(map).is_err());
    }

    #[test]
    fn group_entries_parse_correctly() {
        let raw = serde_json::json!({
            "enabled": { "label": "Enabled", "type": "boolean", "default": true },
            "settings": {
                "label": "Settings",
                "type": "group",
                "show_when": { "enabled": true },
                "options": {
                    "speed": { "label": "Speed", "type": "number", "default": 1.0 }
                }
            }
        });
        let map: IndexMap<String, JsonValue> = serde_json::from_value(raw).unwrap();
        let entries = parse_entries(map).unwrap();
        assert!(matches!(entries["enabled"], mode::ModeEntry::Option(_)));
        assert!(matches!(entries["settings"], mode::ModeEntry::Group(_)));
        if let mode::ModeEntry::Group(ref g) = entries["settings"] {
            assert!(g.show_when.is_some());
            assert!(g.entries.contains_key("speed"));
        }
    }

    #[test]
    fn config_parses_from_json5() {
        let src = r#"
        {
            include: ["src"],
            name: "my-mode",
            version: "0.1.0",
            author: "tester",
            modes: {
                main: {
                    name: "Main",
                    entrypoint: "src/main.lua",
                    options: {
                        count: {
                            label: "Count",
                            type: "integer",
                            default: 3,
                        }
                    }
                }
            }
        }
        "#;
        let config: Config = json5::from_str(src).unwrap();
        assert_eq!(config.name, "my-mode");
        assert_eq!(config.modes.len(), 1);
        assert!(config.modes.contains_key("main"));
        assert_eq!(config.modes["main"].options.len(), 1);
    }
}
