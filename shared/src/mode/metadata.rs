use std::{collections::HashMap, io};

use ciborium::{from_reader, into_writer};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub type ShowWhen = IndexMap<String, ConditionValue>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Metadata {
    pub name: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub modes: IndexMap<String, Mode>,
    pub files: HashMap<String, SourceFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceFile {
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mode {
    pub name: String,
    pub entrypoint: String,
    pub entries: IndexMap<String, ModeEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ModeEntry {
    Option(ModeOption),
    Group(ModeGroup),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModeGroup {
    pub label: String,
    pub description: Option<String>,
    pub show_when: Option<ShowWhen>,
    pub entries: IndexMap<String, ModeEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModeOption {
    pub label: String,
    pub description: Option<String>,
    pub option_type: OptionType,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub enabled_by_default: bool,
    pub show_when: Option<ShowWhen>,
}

/// A value used in a `show_when` condition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ConditionValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

impl ConditionValue {
    pub fn matches(&self, value: &OptionValue) -> bool {
        match (self, value) {
            (Self::Bool(b), OptionValue::Boolean(v)) => b == v,
            (Self::Int(i), OptionValue::Integer(v)) => i == v,
            (Self::Float(f), OptionValue::Number(v)) => f == v,
            (Self::Str(s), OptionValue::Enum(v)) | (Self::Str(s), OptionValue::String(v)) => {
                s == v
            }
            _ => false,
        }
    }
}

impl Mode {
    /// Returns all options in the mode as a flat list, depth-first through groups.
    pub fn all_options(&self) -> Vec<(&str, &ModeOption)> {
        fn collect<'a>(
            entries: &'a IndexMap<String, ModeEntry>,
            out: &mut Vec<(&'a str, &'a ModeOption)>,
        ) {
            for (key, entry) in entries {
                match entry {
                    ModeEntry::Option(opt) => out.push((key.as_str(), opt)),
                    ModeEntry::Group(group) => collect(&group.entries, out),
                }
            }
        }
        let mut result = Vec::new();
        collect(&self.entries, &mut result);
        result
    }

    /// Looks up an option by its key, searching within groups.
    pub fn get_option(&self, key: &str) -> Option<&ModeOption> {
        fn find<'a>(
            entries: &'a IndexMap<String, ModeEntry>,
            key: &str,
        ) -> Option<&'a ModeOption> {
            for (k, entry) in entries {
                match entry {
                    ModeEntry::Option(opt) if k == key => return Some(opt),
                    ModeEntry::Group(group) => {
                        if let Some(opt) = find(&group.entries, key) {
                            return Some(opt);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        find(&self.entries, key)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OptionType {
    Integer {
        default: i64,
        min: Option<i64>,
        max: Option<i64>,
        step: Option<i64>,
        clamp: bool,
        slider: bool,
    },
    Number {
        default: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
        clamp: bool,
        slider: bool,
    },
    String {
        default: String,
    },
    Boolean {
        default: bool,
    },
    Enum {
        default: String,
        values: IndexMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum OptionValue {
    Integer(i64),
    Number(f64),
    String(String),
    Boolean(bool),
    Enum(String),
    Null,
}

#[cfg(feature = "mlua")]
impl mlua::IntoLua for OptionValue {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        match self {
            OptionValue::Integer(x) => x.into_lua(lua),
            OptionValue::Number(x) => x.into_lua(lua),
            OptionValue::String(x) => x.into_lua(lua),
            OptionValue::Boolean(x) => x.into_lua(lua),
            OptionValue::Enum(x) => x.into_lua(lua),
            OptionValue::Null => Ok(mlua::Value::Nil),
        }
    }
}

impl ModeOption {
    pub fn default_value(&self) -> OptionValue {
        if self.optional && !self.enabled_by_default {
            return OptionValue::Null;
        }
        match &self.option_type {
            OptionType::Integer { default, .. } => OptionValue::Integer(*default),
            OptionType::Number { default, .. } => OptionValue::Number(*default),
            OptionType::String { default } => OptionValue::String(default.clone()),
            OptionType::Boolean { default } => OptionValue::Boolean(*default),
            OptionType::Enum { default, .. } => OptionValue::Enum(default.clone()),
        }
    }

    pub fn matches_value(&self, value: &OptionValue) -> bool {
        if self.optional && matches!(value, OptionValue::Null) {
            return true;
        }
        match &self.option_type {
            OptionType::Integer { .. } => matches!(value, OptionValue::Integer(_)),
            OptionType::Number { .. } => matches!(value, OptionValue::Number(_)),
            OptionType::String { .. } => matches!(value, OptionValue::String(_)),
            OptionType::Boolean { .. } => matches!(value, OptionValue::Boolean(_)),
            OptionType::Enum { .. } => matches!(value, OptionValue::Enum(_)),
        }
    }
}

impl Metadata {
    pub fn to_buf(&self) -> Result<Vec<u8>, ciborium::ser::Error<io::Error>> {
        let mut buf = Vec::new();
        into_writer(self, &mut buf)?;
        Ok(buf)
    }

    pub fn from_buf(buf: &[u8]) -> Result<Self, ciborium::de::Error<io::Error>> {
        from_reader(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_option(option_type: OptionType) -> ModeOption {
        ModeOption {
            label: "test".to_string(),
            description: None,
            option_type,
            optional: false,
            enabled_by_default: false,
            show_when: None,
        }
    }

    fn sample_metadata() -> Metadata {
        let mut modes = IndexMap::new();
        let mut entries = IndexMap::new();

        entries.insert(
            "count".to_string(),
            ModeEntry::Option(ModeOption {
                label: "Count".to_string(),
                description: None,
                option_type: OptionType::Integer {
                    default: 5,
                    min: Some(1),
                    max: Some(100),
                    step: None,
                    clamp: true,
                    slider: false,
                },
                optional: false,
                enabled_by_default: false,
                show_when: None,
            }),
        );
        entries.insert(
            "speed".to_string(),
            ModeEntry::Option(ModeOption {
                label: "Speed".to_string(),
                description: Some("How fast".to_string()),
                option_type: OptionType::Number {
                    default: 1.5,
                    min: None,
                    max: None,
                    step: None,
                    clamp: false,
                    slider: false,
                },
                optional: false,
                enabled_by_default: false,
                show_when: None,
            }),
        );
        entries.insert(
            "label".to_string(),
            ModeEntry::Option(ModeOption {
                label: "Label".to_string(),
                description: None,
                option_type: OptionType::String {
                    default: "hello".to_string(),
                },
                optional: false,
                enabled_by_default: false,
                show_when: None,
            }),
        );
        entries.insert(
            "enabled".to_string(),
            ModeEntry::Option(ModeOption {
                label: "Enabled".to_string(),
                description: None,
                option_type: OptionType::Boolean { default: true },
                optional: false,
                enabled_by_default: false,
                show_when: None,
            }),
        );

        let mut group_entries = IndexMap::new();
        let mut values = IndexMap::new();
        values.insert("a".to_string(), "Option A".to_string());
        values.insert("b".to_string(), "Option B".to_string());
        group_entries.insert(
            "variant".to_string(),
            ModeEntry::Option(ModeOption {
                label: "Variant".to_string(),
                description: None,
                option_type: OptionType::Enum {
                    default: "a".to_string(),
                    values,
                },
                optional: false,
                enabled_by_default: false,
                show_when: None,
            }),
        );
        entries.insert(
            "advanced".to_string(),
            ModeEntry::Group(ModeGroup {
                label: "Advanced".to_string(),
                description: None,
                show_when: None,
                entries: group_entries,
            }),
        );

        modes.insert(
            "main".to_string(),
            Mode {
                name: "Main".to_string(),
                entrypoint: "main.lua".to_string(),
                entries,
            },
        );

        let mut files = HashMap::new();
        files.insert(
            "main.lua".to_string(),
            SourceFile {
                offset: 32,
                length: 64,
            },
        );

        Metadata {
            name: "test-mode".to_string(),
            version: Some("1.0.0".to_string()),
            author: Some("tester".to_string()),
            modes,
            files,
        }
    }

    #[test]
    fn metadata_roundtrip() {
        let original = sample_metadata();
        let buf = original.to_buf().unwrap();
        let decoded = Metadata::from_buf(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn metadata_minimal_roundtrip() {
        let original = Metadata {
            name: "min".to_string(),
            version: None,
            author: None,
            modes: IndexMap::new(),
            files: HashMap::new(),
        };
        let buf = original.to_buf().unwrap();
        let decoded = Metadata::from_buf(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn all_options_flattens_groups() {
        let meta = sample_metadata();
        let mode = meta.modes.get("main").unwrap();
        let options = mode.all_options();
        let keys: Vec<&str> = options.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"count"));
        assert!(keys.contains(&"variant")); // inside group
        assert_eq!(keys.len(), 5);
    }

    #[test]
    fn get_option_finds_in_group() {
        let meta = sample_metadata();
        let mode = meta.modes.get("main").unwrap();
        assert!(mode.get_option("variant").is_some());
        assert!(mode.get_option("count").is_some());
        assert!(mode.get_option("nonexistent").is_none());
    }

    #[test]
    fn default_values() {
        let cases: &[(OptionType, OptionValue)] = &[
            (
                OptionType::Integer {
                    default: 7,
                    min: None,
                    max: None,
                    step: None,
                    clamp: false,
                    slider: false,
                },
                OptionValue::Integer(7),
            ),
            (
                OptionType::Number {
                    default: 3.14,
                    min: None,
                    max: None,
                    step: None,
                    clamp: false,
                    slider: false,
                },
                OptionValue::Number(3.14),
            ),
            (
                OptionType::String {
                    default: "hi".to_string(),
                },
                OptionValue::String("hi".to_string()),
            ),
            (
                OptionType::Boolean { default: false },
                OptionValue::Boolean(false),
            ),
            (
                OptionType::Enum {
                    default: "x".to_string(),
                    values: IndexMap::new(),
                },
                OptionValue::Enum("x".to_string()),
            ),
        ];

        for (option_type, expected) in cases {
            assert_eq!(make_option(option_type.clone()).default_value(), *expected);
        }
    }

    #[test]
    fn matches_value_correct_types() {
        let pairs: &[(OptionType, OptionValue)] = &[
            (
                OptionType::Integer {
                    default: 0,
                    min: None,
                    max: None,
                    step: None,
                    clamp: false,
                    slider: false,
                },
                OptionValue::Integer(42),
            ),
            (
                OptionType::Number {
                    default: 0.0,
                    min: None,
                    max: None,
                    step: None,
                    clamp: false,
                    slider: false,
                },
                OptionValue::Number(1.0),
            ),
            (
                OptionType::String {
                    default: String::new(),
                },
                OptionValue::String("s".to_string()),
            ),
            (
                OptionType::Boolean { default: true },
                OptionValue::Boolean(false),
            ),
            (
                OptionType::Enum {
                    default: "a".to_string(),
                    values: IndexMap::new(),
                },
                OptionValue::Enum("b".to_string()),
            ),
        ];

        for (option_type, value) in pairs {
            assert!(make_option(option_type.clone()).matches_value(value));
        }
    }

    #[test]
    fn matches_value_wrong_type() {
        let opt = make_option(OptionType::Integer {
            default: 0,
            min: None,
            max: None,
            step: None,
            clamp: false,
            slider: false,
        });
        assert!(!opt.matches_value(&OptionValue::String("oops".to_string())));
    }

    #[test]
    fn condition_value_matches() {
        assert!(ConditionValue::Bool(true).matches(&OptionValue::Boolean(true)));
        assert!(!ConditionValue::Bool(true).matches(&OptionValue::Boolean(false)));
        assert!(ConditionValue::Int(5).matches(&OptionValue::Integer(5)));
        assert!(ConditionValue::Str("x".to_string()).matches(&OptionValue::Enum("x".to_string())));
        assert!(!ConditionValue::Str("x".to_string()).matches(&OptionValue::Null));
    }
}
