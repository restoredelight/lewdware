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

fn return_true() -> bool {
    true
}
