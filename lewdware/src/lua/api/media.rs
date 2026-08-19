//! The `lewdware.media` table: looking a file up by name, and querying the pack by type
//! and tag.

use std::collections::HashMap;

use mlua::{ExternalError, ExternalResult, FromLua, Lua, LuaSerdeExt};
use serde::{Deserialize, Serialize};

// The Lua layer's own names for these: they describe a colour and an alignment, and a *theme*
// needs to as well, so they live with the themes and are re-exported here.

use crate::{
    lua::{Media, MediaType},
    media::{MediaManager, MediaTypes, TagFilter},
};

pub(super) fn get_media_type(
    name: String,
    types: MediaTypes,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    media_manager
        .get_media(name, types)
        .map_err(|err| err.into_lua_err())
}

pub(super) fn get_media(
    _: &Lua,
    name: String,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::ALL, media_manager)
}

pub(super) fn get_image(
    _: &Lua,
    name: String,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::IMAGE, media_manager)
}

pub(super) fn get_video(
    _: &Lua,
    name: String,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::VIDEO, media_manager)
}

pub(super) fn get_audio(
    _: &Lua,
    name: String,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::AUDIO, media_manager)
}

pub(super) fn list_media_type(
    types: MediaTypes,
    tags: Option<TagFilter>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    media_manager
        .list_media(types, tags)
        .map_err(|err| err.into_lua_err())
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum OneOrMore<T> {
    One(T),
    More(Vec<T>),
}

impl From<OneOrMore<MediaType>> for MediaTypes {
    fn from(value: OneOrMore<MediaType>) -> Self {
        match value {
            OneOrMore::One(MediaType::Image) => Self::IMAGE,
            OneOrMore::One(MediaType::Video) => Self::VIDEO,
            OneOrMore::One(MediaType::Audio) => Self::AUDIO,
            OneOrMore::More(items) => Self::from_slice(&items),
        }
    }
}

/// The Lua-facing shape of a tag filter: either a plain list of tags (shorthand for `any`) or
/// a table with `any`/`all`/`none` lists.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum TagFilterInput {
    Any(Vec<String>),
    Filter {
        any: Option<Vec<String>>,
        all: Option<Vec<String>>,
        none: Option<Vec<String>>,
    },
}

impl From<TagFilterInput> for TagFilter {
    fn from(value: TagFilterInput) -> Self {
        match value {
            TagFilterInput::Any(tags) => TagFilter::any_of(tags),
            TagFilterInput::Filter { any, all, none } => TagFilter {
                any: any.unwrap_or_default(),
                all: all.unwrap_or_default(),
                none: none.unwrap_or_default(),
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct QueryMediaOpts {
    #[serde(rename = "type")]
    pub(super) types: Option<OneOrMore<MediaType>>,
    pub(super) tags: Option<TagFilterInput>,
    pub(super) weights: Option<HashMap<u64, f64>>,
}

impl FromLua for QueryMediaOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub(super) fn list_media(
    _: &Lua,
    opts: Option<QueryMediaOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let (types, tags) = match opts {
        Some(QueryMediaOpts { types, tags, .. }) => (
            types.map_or(MediaTypes::ALL, MediaTypes::from),
            tags.map(TagFilter::from),
        ),
        None => (MediaTypes::ALL, None),
    };

    list_media_type(types, tags, media_manager)
}

#[derive(Serialize, Deserialize, Default)]
pub(super) struct QueryMediaTypeOpts {
    pub(super) tags: Option<TagFilterInput>,
    pub(super) weights: Option<HashMap<u64, f64>>,
}

impl FromLua for QueryMediaTypeOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub(super) fn list_images(
    _: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    list_media_type(MediaTypes::IMAGE, tags, media_manager)
}

pub(super) fn list_videos(
    _: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    list_media_type(MediaTypes::VIDEO, tags, media_manager)
}

pub(super) fn list_audio(
    _: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    list_media_type(MediaTypes::AUDIO, tags, media_manager)
}

pub(super) fn random_media_type(
    _: &Lua,
    types: MediaTypes,
    tags: Option<TagFilter>,
    weights: Option<HashMap<u64, f64>>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    media_manager
        .random_media(types, tags, weights)
        .map_err(|err| err.into_lua_err())
}

pub(super) fn random_media(
    lua: &Lua,
    opts: Option<QueryMediaOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (types, tags, weights) = match opts {
        Some(QueryMediaOpts {
            types,
            tags,
            weights,
        }) => (
            types.map_or(MediaTypes::ALL, MediaTypes::from),
            tags.map(TagFilter::from),
            weights,
        ),
        None => (MediaTypes::ALL, None, None),
    };

    random_media_type(lua, types, tags, weights, media_manager)
}

pub(super) fn random_image(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (tags, weights) = opts
        .map(|x| (x.tags.map(TagFilter::from), x.weights))
        .unwrap_or_default();

    random_media_type(lua, MediaTypes::IMAGE, tags, weights, media_manager)
}

pub(super) fn random_video(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (tags, weights) = opts
        .map(|x| (x.tags.map(TagFilter::from), x.weights))
        .unwrap_or_default();

    random_media_type(lua, MediaTypes::VIDEO, tags, weights, media_manager)
}

pub(super) fn random_audio(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (tags, weights) = opts
        .map(|x| (x.tags.map(TagFilter::from), x.weights))
        .unwrap_or_default();

    random_media_type(lua, MediaTypes::AUDIO, tags, weights, media_manager)
}

pub(super) fn list_tags(_: &Lua, _: (), media_manager: MediaManager) -> mlua::Result<Vec<String>> {
    media_manager.list_tags().into_lua_err()
}
