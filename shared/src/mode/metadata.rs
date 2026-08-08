use std::{collections::HashMap, io};

use ciborium::{from_reader, into_writer};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub type ShowWhen = IndexMap<String, ConditionValue>;

/// A user-owned permission (`shared::user_config::Capabilities`) that a mode's entry says it
/// makes use of.
///
/// This is a *declaration*, not a request: nothing a mode says here can grant it anything, and a
/// denied permission still just makes the corresponding call no-op (see `LewdwareApp::open_link`
/// and friends). Its only job is to let `config/` say "this option won't do anything until you
/// allow X" in the place where the user is configuring that option, instead of leaving them to
/// discover the silence for themselves. There is deliberately no hard/soft distinction -- a mode
/// that could declare a permission mandatory would have leverage to pressure the user into
/// granting it, and every capability already degrades gracefully.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    SetWallpaper,
    OpenLinks,
    SendNotifications,
}

/// A single mode's metadata -- one `.lwmode` file is exactly one mode (see `Header::id`'s doc
/// comment for why: a stable per-mode identity, used for `lewdware.storage`, wouldn't have a
/// clean meaning if one file could contain several).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Metadata {
    pub name: String,
    pub version: Option<String>,
    pub author: Option<String>,
    pub entrypoint: String,
    pub entries: IndexMap<String, ModeEntry>,
    pub files: HashMap<String, SourceFile>,
    /// Permissions the mode uses regardless of how it is configured. `#[serde(default)]` is
    /// load-bearing, not polish: every `.lwmode` built before this field existed has no key for
    /// it, and must keep decoding.
    #[serde(default)]
    pub needs_permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceFile {
    pub offset: u64,
    pub length: u64,
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
    /// Permissions every option in this group depends on. See `Metadata::needs_permissions`.
    #[serde(default)]
    pub needs_permissions: Vec<Permission>,
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
    /// Permissions this option depends on. See `Metadata::needs_permissions`.
    #[serde(default)]
    pub needs_permissions: Vec<Permission>,
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
            (Self::Str(s), OptionValue::Enum(v)) | (Self::Str(s), OptionValue::String(v)) => s == v,
            _ => false,
        }
    }
}

impl Metadata {
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
        fn find<'a>(entries: &'a IndexMap<String, ModeEntry>, key: &str) -> Option<&'a ModeOption> {
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

/// Resolves stored option values against a schema: every option in the tree gets a value,
/// the user's where it fits the option's type and the schema default otherwise. Walks
/// groups depth-first, same as `Metadata::all_options`.
///
/// This is the only way to obtain an `OptionValue` from something a user wrote, and it is
/// deliberately the only one -- see `StoredValue`.
pub fn resolve_options(
    entries: &IndexMap<String, ModeEntry>,
    stored: &HashMap<String, StoredValue>,
) -> HashMap<String, OptionValue> {
    fn walk(
        entries: &IndexMap<String, ModeEntry>,
        stored: &HashMap<String, StoredValue>,
        out: &mut HashMap<String, OptionValue>,
    ) {
        for (key, entry) in entries {
            match entry {
                ModeEntry::Option(opt) => {
                    out.insert(key.clone(), opt.resolve(stored.get(key)));
                }
                ModeEntry::Group(group) => walk(&group.entries, stored, out),
            }
        }
    }

    let mut out = HashMap::new();
    walk(entries, stored, &mut out);
    out
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

/// An option value as it sits in the user's `config.json` (or arrives from `config/`'s
/// frontend): one of the shapes JSON has, and nothing more.
///
/// Deliberately *not* `OptionValue`. Which `OptionType` a value belongs to is a fact about
/// the schema, not about the value, and it cannot survive a trip through JSON: `5` is the
/// only way to write a whole `Number`, and an `Enum` member is indistinguishable from any
/// other string. (The frontend is more emphatic still -- `config/src/lib/types.ts` types an
/// option value as `number | string | boolean | null`, because JavaScript cannot tell an
/// integer from a float at all.) A type that claimed otherwise would be lying, and the lie
/// would be read back as fact.
///
/// So stored values keep their own type, and `ModeOption::resolve` -- the only thing that
/// turns one into an `OptionValue` -- lets the schema decide what it means.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum StoredValue {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Null,
}

/// An option value that has been resolved against a schema, and so is known to be one the
/// option can actually hold. This is what reaches a mode's `lewdware.config` table and what
/// `ShowWhen` conditions are evaluated against.
///
/// Only `ModeOption::resolve` and `ModeOption::default_value` produce these, which is what
/// makes the guarantee worth anything: there is no path from a user-written value to a mode
/// that skips the schema. Values the engine synthesises itself (the `pack_has_*` constants
/// in `behaviour::resolver`) are the exception, and are constants, not user input.
///
/// `Serialize` but deliberately not `Deserialize`: serialising is how a resolved value
/// reaches `config/`'s frontend for display (untagged, so JS sees a plain value), but
/// deserialising one would be claiming to know an option's type without having consulted
/// the schema -- exactly the thing this type exists to rule out. Read a `StoredValue`.
#[derive(Debug, Clone, Serialize, PartialEq)]
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

    /// Resolves what this option is actually set to: the stored value read as the type this
    /// option declares, or the schema default if there is nothing stored or what is stored
    /// isn't a value this option can hold.
    ///
    /// Total by construction -- an option always has a value -- and the only route from a
    /// `StoredValue` to an `OptionValue`.
    pub fn resolve(&self, stored: Option<&StoredValue>) -> OptionValue {
        stored
            .and_then(|value| self.read(value))
            .unwrap_or_else(|| self.default_value())
    }

    /// Reads a stored value as this option's type, or `None` if it isn't one this option can
    /// hold. The schema decides what the value *is*: JSON's shapes don't line up with
    /// `OptionType` (see `StoredValue`), so `Int` serves both numeric types and `Str` serves
    /// both string-shaped ones.
    fn read(&self, value: &StoredValue) -> Option<OptionValue> {
        if matches!(value, StoredValue::Null) {
            // Null is a value only for an option that can be switched off; for anything else
            // it means "no setting", and the default applies.
            return self.optional.then_some(OptionValue::Null);
        }
        match (&self.option_type, value) {
            (OptionType::Integer { .. }, StoredValue::Int(i)) => Some(OptionValue::Integer(*i)),
            // A whole-numbered float is the same setting; anything with a fractional part (or
            // beyond i64, or NaN) is not an integer, and falls back to the default.
            (OptionType::Integer { .. }, StoredValue::Float(f)) => {
                let truncated = f.trunc();
                (*f == truncated && truncated >= i64::MIN as f64 && truncated <= i64::MAX as f64)
                    .then_some(OptionValue::Integer(truncated as i64))
            }
            (OptionType::Number { .. }, StoredValue::Float(f)) => Some(OptionValue::Number(*f)),
            (OptionType::Number { .. }, StoredValue::Int(i)) => {
                Some(OptionValue::Number(*i as f64))
            }
            (OptionType::String { .. }, StoredValue::Str(s)) => {
                Some(OptionValue::String(s.clone()))
            }
            (OptionType::Boolean { .. }, StoredValue::Bool(b)) => Some(OptionValue::Boolean(*b)),
            // Enum members are the only values an enum can hold, so a string left over from an
            // older schema has to be rejected here -- otherwise it would reach the mode as a
            // variant the mode never declared and cannot have written a branch for.
            (OptionType::Enum { values, .. }, StoredValue::Str(s)) => {
                values.contains_key(s).then(|| OptionValue::Enum(s.clone()))
            }
            _ => None,
        }
    }

    /// Whether this option would keep `value` rather than fall back to its default.
    pub fn accepts(&self, value: &StoredValue) -> bool {
        self.read(value).is_some()
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
            needs_permissions: Vec::new(),
        }
    }

    fn sample_metadata() -> Metadata {
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
                needs_permissions: Vec::new(),
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
                needs_permissions: Vec::new(),
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
                needs_permissions: Vec::new(),
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
                needs_permissions: Vec::new(),
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
                needs_permissions: Vec::new(),
            }),
        );
        entries.insert(
            "advanced".to_string(),
            ModeEntry::Group(ModeGroup {
                label: "Advanced".to_string(),
                description: None,
                show_when: None,
                needs_permissions: Vec::new(),
                entries: group_entries,
            }),
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
            entrypoint: "main.lua".to_string(),
            entries,
            files,
            needs_permissions: Vec::new(),
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
            entrypoint: "main.lua".to_string(),
            entries: IndexMap::new(),
            files: HashMap::new(),
            needs_permissions: Vec::new(),
        };
        let buf = original.to_buf().unwrap();
        let decoded = Metadata::from_buf(&buf).unwrap();
        assert_eq!(original, decoded);
    }

    /// The real risk in adding `requires`: every `.lwmode` built before it existed has no key for
    /// it anywhere in its CBOR, and must keep decoding. `#[serde(default)]` is what makes that
    /// true, so it needs a test that fails if the attribute is ever dropped.
    #[test]
    fn metadata_without_needs_permissions_still_decodes() {
        #[derive(Serialize)]
        struct OldOption {
            label: String,
            description: Option<String>,
            option_type: OptionType,
            optional: bool,
            enabled_by_default: bool,
            show_when: Option<ShowWhen>,
        }

        #[derive(Serialize)]
        #[serde(tag = "kind")]
        enum OldEntry {
            Option(OldOption),
        }

        #[derive(Serialize)]
        struct OldMetadata {
            name: String,
            version: Option<String>,
            author: Option<String>,
            entrypoint: String,
            entries: IndexMap<String, OldEntry>,
            files: HashMap<String, SourceFile>,
        }

        let mut entries = IndexMap::new();
        entries.insert(
            "enabled".to_string(),
            OldEntry::Option(OldOption {
                label: "Enabled".to_string(),
                description: None,
                option_type: OptionType::Boolean { default: true },
                optional: false,
                enabled_by_default: false,
                show_when: None,
            }),
        );

        let mut buf = Vec::new();
        into_writer(
            &OldMetadata {
                name: "old".to_string(),
                version: None,
                author: None,
                entrypoint: "main.lua".to_string(),
                entries,
                files: HashMap::new(),
            },
            &mut buf,
        )
        .unwrap();

        let decoded =
            Metadata::from_buf(&buf).expect("a pre-`needs_permissions` mode must still decode");
        assert!(decoded.needs_permissions.is_empty());
        assert!(
            decoded
                .get_option("enabled")
                .unwrap()
                .needs_permissions
                .is_empty()
        );
    }

    #[test]
    fn needs_permissions_survives_a_roundtrip() {
        let mut original = sample_metadata();
        original.needs_permissions = vec![Permission::SendNotifications];
        if let Some(ModeEntry::Option(opt)) = original.entries.get_mut("enabled") {
            opt.needs_permissions = vec![Permission::SetWallpaper, Permission::OpenLinks];
        }
        if let Some(ModeEntry::Group(group)) = original.entries.get_mut("advanced") {
            group.needs_permissions = vec![Permission::OpenLinks];
        }

        let decoded = Metadata::from_buf(&original.to_buf().unwrap()).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn all_options_flattens_groups() {
        let meta = sample_metadata();
        let options = meta.all_options();
        let keys: Vec<&str> = options.iter().map(|(k, _)| *k).collect();
        assert!(keys.contains(&"count"));
        assert!(keys.contains(&"variant")); // inside group
        assert_eq!(keys.len(), 5);
    }

    #[test]
    fn get_option_finds_in_group() {
        let meta = sample_metadata();
        assert!(meta.get_option("variant").is_some());
        assert!(meta.get_option("count").is_some());
        assert!(meta.get_option("nonexistent").is_none());
    }

    #[test]
    fn resolve_options_prefers_matching_stored_value() {
        let meta = sample_metadata();
        let mut stored = HashMap::new();
        stored.insert("count".to_string(), StoredValue::Int(42));

        let resolved = resolve_options(&meta.entries, &stored);

        assert_eq!(resolved.get("count"), Some(&OptionValue::Integer(42)));
    }

    #[test]
    fn resolve_options_falls_back_to_default_when_missing_or_mismatched() {
        let meta = sample_metadata();
        let mut stored = HashMap::new();
        // "label" is a String option; a boolean stored value shouldn't match it.
        stored.insert("label".to_string(), StoredValue::Bool(true));

        let resolved = resolve_options(&meta.entries, &stored);

        // Missing entirely -> default.
        assert_eq!(resolved.get("count"), Some(&OptionValue::Integer(5)));
        // Present but wrong type -> default.
        assert_eq!(
            resolved.get("label"),
            Some(&OptionValue::String("hello".to_string()))
        );
    }

    #[test]
    fn resolve_options_walks_into_groups() {
        let meta = sample_metadata();
        let resolved = resolve_options(&meta.entries, &HashMap::new());

        // "variant" lives inside the "advanced" group.
        assert_eq!(
            resolved.get("variant"),
            Some(&OptionValue::Enum("a".to_string()))
        );
        assert_eq!(resolved.len(), meta.all_options().len());
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
                    default: 3.15,
                    min: None,
                    max: None,
                    step: None,
                    clamp: false,
                    slider: false,
                },
                OptionValue::Number(3.15),
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
    fn each_option_type_accepts_its_own_stored_shape() {
        let mut enum_values = IndexMap::new();
        enum_values.insert("a".to_string(), "A".to_string());
        enum_values.insert("b".to_string(), "B".to_string());

        let pairs: &[(OptionType, StoredValue, OptionValue)] = &[
            (
                OptionType::Integer {
                    default: 0,
                    min: None,
                    max: None,
                    step: None,
                    clamp: false,
                    slider: false,
                },
                StoredValue::Int(42),
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
                StoredValue::Float(1.5),
                OptionValue::Number(1.5),
            ),
            (
                OptionType::String {
                    default: String::new(),
                },
                StoredValue::Str("s".to_string()),
                OptionValue::String("s".to_string()),
            ),
            (
                OptionType::Boolean { default: true },
                StoredValue::Bool(false),
                OptionValue::Boolean(false),
            ),
            (
                OptionType::Enum {
                    default: "a".to_string(),
                    values: enum_values,
                },
                StoredValue::Str("b".to_string()),
                OptionValue::Enum("b".to_string()),
            ),
        ];

        for (option_type, stored, expected) in pairs {
            let opt = make_option(option_type.clone());
            assert!(opt.accepts(stored));
            assert_eq!(opt.resolve(Some(stored)), *expected);
        }
    }

    #[test]
    fn an_option_falls_back_to_its_default_for_a_shape_it_cannot_hold() {
        let opt = integer_option(7);

        assert!(!opt.accepts(&StoredValue::Str("oops".to_string())));
        assert_eq!(
            opt.resolve(Some(&StoredValue::Str("oops".to_string()))),
            OptionValue::Integer(7)
        );
        // Nothing stored at all is the same story.
        assert_eq!(opt.resolve(None), OptionValue::Integer(7));
    }

    fn number_option(default: f64) -> ModeOption {
        make_option(OptionType::Number {
            default,
            min: None,
            max: None,
            step: None,
            clamp: false,
            slider: false,
        })
    }

    fn integer_option(default: i64) -> ModeOption {
        make_option(OptionType::Integer {
            default,
            min: None,
            max: None,
            step: None,
            clamp: false,
            slider: false,
        })
    }

    fn enum_option() -> ModeOption {
        let mut values = IndexMap::new();
        values.insert("constant".to_string(), "Constant".to_string());
        values.insert("accelerating".to_string(), "Accelerating".to_string());
        make_option(OptionType::Enum {
            default: "constant".to_string(),
            values,
        })
    }

    /// The schema, not the stored shape, decides what a value is: JSON has one number type
    /// for both `Integer` and `Number` options, and no way at all to mark a string as an enum
    /// member. If those shapes didn't resolve, the user's setting would be silently traded
    /// for the default on the next load.
    #[test]
    fn the_schema_decides_which_type_a_stored_shape_reads_as() {
        assert_eq!(
            number_option(1.0).resolve(Some(&StoredValue::Int(5))),
            OptionValue::Number(5.0)
        );
        assert_eq!(
            enum_option().resolve(Some(&StoredValue::Str("accelerating".to_string()))),
            OptionValue::Enum("accelerating".to_string())
        );
        // A whole-numbered float is still the same integer setting.
        assert_eq!(
            integer_option(0).resolve(Some(&StoredValue::Float(5.0))),
            OptionValue::Integer(5)
        );
    }

    #[test]
    fn stored_shapes_the_type_cannot_hold_are_rejected() {
        // Fractional, non-finite and out-of-range floats are not integers.
        for f in [5.5, f64::INFINITY, f64::NAN, 1e30] {
            assert!(
                !integer_option(0).accepts(&StoredValue::Float(f)),
                "{f} should not read as an integer"
            );
        }
        // A string that isn't one of the declared members is not a value of this enum.
        assert!(!enum_option().accepts(&StoredValue::Str("nonsense".to_string())));
        // Nothing is read across the number/string/bool divides.
        assert!(!number_option(1.0).accepts(&StoredValue::Str("5".to_string())));
        assert!(!number_option(1.0).accepts(&StoredValue::Bool(true)));
        assert!(!integer_option(0).accepts(&StoredValue::Str("oops".to_string())));
    }

    #[test]
    fn null_is_a_value_only_for_an_optional_option() {
        let mut opt = integer_option(5);
        // Not switchable off -> null is not a setting, so the default applies.
        assert_eq!(
            opt.resolve(Some(&StoredValue::Null)),
            OptionValue::Integer(5)
        );

        opt.optional = true;
        assert_eq!(opt.resolve(Some(&StoredValue::Null)), OptionValue::Null);
    }

    /// The end-to-end shape of the bug this type split exists to prevent: options read back
    /// out of a config file on disk must resolve to the values the user chose, not to the
    /// schema defaults.
    ///
    /// `5` is exactly what lands in `config.json` for a `Number` option the user set to a
    /// whole number -- the config UI round-trips `mode_options` through the frontend, and
    /// JavaScript has no integer/float distinction to preserve `5.0` with. Enums lose their
    /// variant on any write at all, JS or not.
    #[test]
    fn resolve_options_survives_a_json_round_trip() {
        let meta = sample_metadata();
        let stored: HashMap<String, StoredValue> =
            serde_json::from_str(r#"{"speed": 5, "variant": "b", "count": 42}"#).unwrap();

        // Precondition: what JSON hands back is not what was written.
        assert_eq!(stored.get("speed"), Some(&StoredValue::Int(5)));
        assert_eq!(
            stored.get("variant"),
            Some(&StoredValue::Str("b".to_string()))
        );

        let resolved = resolve_options(&meta.entries, &stored);

        assert_eq!(resolved.get("speed"), Some(&OptionValue::Number(5.0)));
        assert_eq!(
            resolved.get("variant"),
            Some(&OptionValue::Enum("b".to_string()))
        );
        assert_eq!(resolved.get("count"), Some(&OptionValue::Integer(42)));
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
