use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
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
    pub options: IndexMap<String, ModeOption>,
}

#[derive(Serialize, Deserialize)]
pub struct ModeOption {
    pub label: String,
    pub description: Option<String>,
    #[serde(flatten)]
    pub option_type: OptionType,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OptionType {
    #[serde(rename = "integer")]
    Integer {
        default: i64,
        min: Option<i64>,
        max: Option<i64>,
        step: Option<i64>,
        #[serde(default)]
        clamp: bool,
        slider: Option<bool>,
    },
    #[serde(rename = "number")]
    Number {
        default: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
        #[serde(default)]
        clamp: bool,
        slider: Option<bool>,
    },
    #[serde(rename = "string")]
    String { default: String },
    #[serde(rename = "boolean")]
    Boolean { default: bool },
    #[serde(rename = "enum")]
    Enum {
        default: String,
        values: IndexMap<String, String>,
    },
}

impl TryFrom<ModeOption> for mode::ModeOption {
    type Error = anyhow::Error;

    fn try_from(
        ModeOption {
            label,
            description,
            option_type,
        }: ModeOption,
    ) -> std::result::Result<Self, Self::Error> {
        let option_type = option_type
            .try_into()
            .with_context(|| format!("Error in `{}`", label))?;

        Ok(Self {
            label,
            description,
            option_type,
        })
    }
}

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
                default,
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
                default,
                min,
                max,
                step,
                clamp,
                slider: slider.unwrap_or_else(|| min.is_some() && max.is_some()),
            },
            OptionType::String { default } => Self::String { default },
            OptionType::Boolean { default } => Self::Boolean { default },
            OptionType::Enum { default, values } => {
                if !values.keys().any(|key| key == &default) {
                    bail!("`default` ('{default}') is not a valid option");
                }

                Self::Enum { default, values }
            }
        })
    }
}

impl OptionType {
    pub fn validate(&self) -> Result<()> {
        match &self {
            Self::Enum { default, values } => {
                if !values.keys().any(|key| key == default) {
                    bail!("`default` ('{default}') does not match any options");
                }
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_enum_option(default: &str, values: &[&str]) -> ModeOption {
        let mut map = IndexMap::new();
        for v in values {
            map.insert(v.to_string(), v.to_string());
        }
        ModeOption {
            label: "test".to_string(),
            description: None,
            option_type: OptionType::Enum {
                default: default.to_string(),
                values: map,
            },
        }
    }

    #[test]
    fn enum_valid_default_converts() {
        let opt = make_enum_option("a", &["a", "b"]);
        let result: Result<mode::ModeOption> = opt.try_into();
        assert!(result.is_ok());
    }

    #[test]
    fn enum_invalid_default_rejected() {
        let opt = make_enum_option("z", &["a", "b"]);
        let result: Result<mode::ModeOption> = opt.try_into();
        assert!(result.is_err());
    }

    #[test]
    fn slider_defaults_true_when_min_and_max_set() {
        let opt = OptionType::Integer {
            default: 0,
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
            default: 0,
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
            default: 0,
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
