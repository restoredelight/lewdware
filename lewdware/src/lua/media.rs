use mlua::{FromLua, IntoLua, LuaSerdeExt, SerializeOptions};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Media {
    pub id: u64,
    pub name: String,
    #[serde(flatten)]
    pub media_data: MediaData,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum MediaData {
    #[serde(rename = "image")]
    Image {
        width: u32,
        height: u32
    },
    #[serde(rename = "video")]
    Video {
        width: u32,
        height: u32,
        duration: f64,
    },
    #[serde(rename = "audio")]
    Audio {
        duration: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaType {
    #[serde(rename = "image")]
    Image,
    #[serde(rename = "video")]
    Video,
    #[serde(rename = "audio")]
    Audio
}

impl IntoLua for Media {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        lua.to_value_with(&self, SerializeOptions::new().serialize_none_to_null(false))
    }
}

impl FromLua for Media {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}
