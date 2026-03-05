use std::{collections::HashMap, io};

use ciborium::{from_reader, into_writer};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

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
    pub options: IndexMap<String, ModeOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModeOption {
    pub label: String,
    pub description: Option<String>,
    pub option_type: OptionType,
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
}

impl mlua::IntoLua for OptionValue {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        match self {
            OptionValue::Integer(x) => x.into_lua(lua),
            OptionValue::Number(x) => x.into_lua(lua),
            OptionValue::String(x) => x.into_lua(lua),
            OptionValue::Boolean(x) => x.into_lua(lua),
            OptionValue::Enum(x) => x.into_lua(lua),
        }
    }
}

impl ModeOption {
    pub fn default_value(&self) -> OptionValue {
        match &self.option_type {
            OptionType::Integer { default, .. } => OptionValue::Integer(*default),
            OptionType::Number { default, .. } => OptionValue::Number(*default),
            OptionType::String { default } => OptionValue::String(default.clone()),
            OptionType::Boolean { default } => OptionValue::Boolean(*default),
            OptionType::Enum { default, .. } => OptionValue::Enum(default.clone()),
        }
    }

    pub fn matches_value(&self, value: &OptionValue) -> bool {
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
