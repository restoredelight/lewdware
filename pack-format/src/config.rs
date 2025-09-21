use std::{collections::HashMap, io};

use ciborium::{from_reader, into_writer};
use merge::Merge;
use serde::{Deserialize, Serialize};

use crate::{create_arg, target::Target};

#[derive(Serialize, Deserialize, Default)]
pub struct PackOpts {
    #[serde(flatten)]
    pub metadata: Metadata,
    #[serde(flatten)]
    pub media: MediaOpts,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, OneOrMore<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_tag: Option<String>,
    pub ignore: Option<OneOrMore<String>>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Metadata {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<Transition>,
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

#[derive(Serialize, Deserialize, Default)]
pub struct MediaOpts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub popups: Option<Target<String, PathArg>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notifications: Option<Target<String, TextArg, NotificationOpts>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Target<String, LinkArg>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<Target<String, TextArg>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<Target<String, PathArg>>,
}

create_arg!(PathArg, path, String);
create_arg!(TextArg, text, String);
create_arg!(LinkArg, link, String);

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMore<T> {
    One(T),
    More(Vec<T>),
}

#[derive(Serialize, Deserialize, Default, Merge, Clone)]
pub struct NotificationOpts {
    #[merge(strategy = merge::option::overwrite_none)]
    pub summary: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Transition {
    #[serde(default)]
    pub transition: TransitionType,
    #[serde(default)]
    pub apply_to: TransitionApplyTo,
    #[serde(default)]
    pub order: Order,
    #[serde(rename = "loop")]
    #[serde(default = "return_true")]
    pub loop_items: bool,
    pub items: Vec<TransitionItem>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(untagged)]
pub enum TransitionApplyTo {
    #[default]
    #[serde(rename = "all")]
    All,
    Some(Vec<MediaType>),
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum MediaType {
    #[serde(rename = "popups")]
    Popups,
    #[serde(rename = "audio")]
    Audio,
    #[serde(rename = "notifications")]
    Notifications,
    #[serde(rename = "links")]
    Links,
    #[serde(rename = "prompts")]
    Prompts,
    #[serde(rename = "wallpaper")]
    Wallpaper,
}

fn return_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Eq)]
pub enum TransitionType {
    #[default]
    #[serde(rename = "linear")]
    Linear,
    #[serde(rename = "abrupt")]
    Abrupt,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
pub enum Order {
    #[default]
    #[serde(rename = "sequential")]
    Sequential,
    #[serde(rename = "random")]
    Random,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TransitionItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrObject<T> {
    String(String),
    Object(T),
}
