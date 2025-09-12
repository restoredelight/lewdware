use std::{collections::HashMap, io};

use ciborium::{from_reader, into_writer};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct PackOpts {
    #[serde(flatten)]
    pub metadata: Metadata,
    #[serde(flatten)]
    pub media: MediaOpts,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub tags: HashMap<String, StringOrVec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_tag: Option<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
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
    pub popups: Option<Popups>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notifications: Option<Notifications>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Links>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<Prompts>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrVec<T> {
    String(String),
    Vec(Vec<T>)
}

#[derive(Serialize, Deserialize)]
pub struct Popups {
    #[serde(default)]
    pub default: PopupOpts,
    pub items: Vec<StringOrObject<Popup>>,
}

#[derive(Serialize, Deserialize)]
pub struct Popup {
    pub path: String,
    #[serde(flatten)]
    pub opts: PopupOpts,
}

#[derive(Serialize, Deserialize, Default)]
pub struct PopupOpts {
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Notifications {
    #[serde(default)]
    pub default: NotificationOpts,
    pub items: Vec<StringOrObject<Notification>>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct NotificationOpts {
    #[serde(default)]
    pub tags: Vec<String>,
    pub summary: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Notification {
    #[serde(flatten)]
    pub opts: NotificationOpts,
    pub body: String,
}

#[derive(Serialize, Deserialize)]
pub struct Links {
    #[serde(default)]
    pub default: LinkOpts,
    pub items: Vec<StringOrObject<Link>>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct LinkOpts {
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Link {
    #[serde(flatten)]
    pub opts: LinkOpts,
    pub link: String,
}

#[derive(Serialize, Deserialize)]
pub struct Prompts {
    #[serde(default)]
    pub default: PromptOpts,
    pub items: Vec<StringOrObject<Prompt>>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct PromptOpts {
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Prompt {
    #[serde(flatten)]
    pub opts: PromptOpts,
    pub prompt: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Transition {
    #[serde(default)]
    pub transition: TransitionType,
    #[serde(default)]
    pub order: Order,
    #[serde(rename = "loop")]
    #[serde(default = "return_true")]
    pub loop_items: bool,
    pub items: Vec<TransitionItem>,
}

fn return_true() -> bool { true }

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
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
    Object(T)
}

