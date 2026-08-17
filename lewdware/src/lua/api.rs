use std::{collections::HashMap, rc::Rc, time::Duration};

use mlua::{
    ExternalError, ExternalResult, FromLua, IntoLua, Lua, LuaSerdeExt, serde::SerializeOptions,
};
use serde::{Deserialize, Serialize};
use shared::mode::OptionValue;

use rand::seq::IndexedRandom;

// The Lua layer's own names for these: they describe a colour and an alignment, and a *theme*
// needs to as well, so they live with the themes and are re-exported here.
pub use shared::theme::{Color, TextAlign};

use crate::{
    app::EventPoster,
    lua::{
        AudioHandles, Media, MediaData, MediaType, Window, Windows,
        audio::AudioHandle,
        interval::{Interval, Timer},
        request::RequestSender,
        window::{DialogWindow, ImageWindow, TextWindow, VideoWindow},
    },
    media::{MediaManager, MediaTypes, TagFilter},
    monitor::Monitor,
    utils::{
        calculate_media_popup_size, calculate_text_popup_size, random_position, random_position_in,
    },
    window::{AppearanceChoice, ChromeDefaults, Theme, ThemeChoice},
};

pub struct ApiOptions {
    pub pack_info: Option<crate::lua::PackInfo>,
    pub config: HashMap<String, OptionValue>,
    /// The behaviour `content` section for `Mode::Sandbox` and `Mode::Experience`, already
    /// resolved for Lua by `lua::lua_view` -- media slots carry file names here, not the ids the
    /// document stores.
    pub content: serde_json::Value,
    /// The behaviour `experience` section for `Mode::Experience`, resolved the same way.
    pub experience: serde_json::Value,
    /// The user's own window look, applied to any window the mode does not theme itself.
    pub chrome: ChromeDefaults,
    pub gpu_available: bool,
    pub dev_mode: bool,
}

pub fn create_api<T: EventPoster>(
    lua: &Lua,
    request_sender: RequestSender<T>,
    media_manager: MediaManager,
    windows: Windows<T>,
    audio_handles: AudioHandles<T>,
    storage: crate::lua::storage::Storage,
    options: ApiOptions,
) -> mlua::Result<()> {
    let ApiOptions {
        pack_info,
        config,
        content,
        experience,
        chrome,
        gpu_available,
        dev_mode,
    } = options;

    let api_table = lua.create_table()?;

    // Nothing empty may reach Lua as mlua's `Value::NULL` sentinel (a lightuserdata, distinct from
    // Lua `nil`, for JSON-style null-vs-absent round-tripping): that sentinel is *truthy*, so the
    // idiomatic `field or default` fallback used throughout `default-modes/shared/lib/*.lua` (e.g.
    // `PromptSettings.submit_label`) would silently never fall back. This is an internal
    // engine-to-Lua channel, not a JSON API needing that distinction, so plain `nil` is correct.
    //
    // Both options, not just `serialize_none_to_null`: these sections arrive as `serde_json::Value`
    // (already resolved for Lua by `lua::lua_view`), and a `Value::Null` serializes through
    // `serialize_unit`, not `serialize_none`.
    let for_lua = SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(false);

    // Private channel to the default-modes library code (never under the public `lewdware`
    // table, so it stays out of api.lua/the docs site): the whole behaviour.json `content`
    // section, empty for custom modes. See `ApiOptions::content`'s doc comment.
    let content_value = lua.to_value_with(&content, for_lua)?;
    lua.globals().set("__lewdware_content", content_value)?;

    // Mirrors `__lewdware_content` for the `experience` section -- only
    // `default-modes/experience/src/main.lua` reads this (empty for Sandbox/custom modes).
    let experience_value = lua.to_value_with(&experience, for_lua)?;
    lua.globals()
        .set("__lewdware_experience", experience_value)?;

    api_table.set("config", config.into_lua(lua)?)?;

    // The theme names this engine understands, so a mode can validate one that came from pack
    // data before passing it to a spawn call (where an unknown name is a hard error).
    api_table.set(
        "themes",
        ThemeChoice::ALL
            .iter()
            .map(|choice| choice.name())
            .collect::<Vec<_>>(),
    )?;
    api_table.set(
        "appearances",
        AppearanceChoice::ALL
            .iter()
            .map(|choice| choice.name())
            .collect::<Vec<_>>(),
    )?;

    // The user's own choice, which every window already uses unless the mode overrides it. Here
    // so a mode can deliberately *differ* from it for one window while leaving the rest alone —
    // as the name it was chosen by, not the look it resolves to, since that is what a mode would
    // pass back to a spawn call.
    api_table.set("user_theme", chrome.theme.name())?;
    api_table.set("user_appearance", chrome.appearance.name())?;

    let storage_table = lua.create_table()?;
    {
        let storage = storage.clone();
        storage_table.set(
            "get",
            lua.create_function(move |lua, key: String| storage.get(lua, &key))?,
        )?;
    }
    {
        let storage = storage.clone();
        storage_table.set(
            "set",
            lua.create_function(move |_, (key, value): (String, mlua::Value)| {
                storage.set(key, value)
            })?,
        )?;
    }
    {
        let storage = storage.clone();
        storage_table.set(
            "remove",
            lua.create_function(move |_, key: String| Ok(storage.remove(&key)))?,
        )?;
    }
    {
        let storage = storage.clone();
        storage_table.set(
            "clear",
            lua.create_function(move |_, ()| {
                storage.clear();
                Ok(())
            })?,
        )?;
    }
    {
        let storage = storage.clone();
        storage_table.set(
            "keys",
            lua.create_function(move |_, ()| Ok(storage.keys()))?,
        )?;
    }
    api_table.set("storage", storage_table)?;

    // `None` for a mode embedded in a pack -- see the doc comment on `PackInfo`'s only
    // constructor call site (`start_lua_thread`) for why.
    if let Some(pack_info) = pack_info {
        let pack_table = lua.create_table()?;
        pack_table.set("id", pack_info.id.to_string())?;
        pack_table.set("name", pack_info.metadata.name)?;
        pack_table.set("author", pack_info.metadata.creator)?;
        pack_table.set("version", pack_info.metadata.version)?;
        api_table.set("pack", pack_table)?;
    }

    let media_table = lua.create_table()?;

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get",
            lua.create_function(move |lua, name| get_media(lua, name, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_image",
            lua.create_function(move |lua, name| get_image(lua, name, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_video",
            lua.create_function(move |lua, name| get_video(lua, name, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_audio",
            lua.create_function(move |lua, name| get_audio(lua, name, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list",
            lua.create_function(move |lua, opts| list_media(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_images",
            lua.create_function(move |lua, opts| list_images(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_videos",
            lua.create_function(move |lua, opts| list_videos(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_audio",
            lua.create_function(move |lua, opts| list_audio(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random",
            lua.create_function(move |lua, opts| random_media(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_image",
            lua.create_function(move |lua, opts| random_image(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_video",
            lua.create_function(move |lua, opts| random_video(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_audio",
            lua.create_function(move |lua, opts| random_audio(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_tags",
            lua.create_function(move |lua, ()| list_tags(lua, (), media_manager.clone()))?,
        )?;
    }

    api_table.set("media", media_table)?;

    let popup_table = lua.create_table()?;

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        popup_table.set(
            "image",
            lua.create_function(move |lua, args| {
                spawn_image_popup(
                    lua,
                    args,
                    request_sender.clone(),
                    windows.clone(),
                    chrome,
                    gpu_available,
                    dev_mode,
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        popup_table.set(
            "video",
            lua.create_function(move |lua, args| {
                spawn_video_popup(
                    lua,
                    args,
                    request_sender.clone(),
                    windows.clone(),
                    chrome,
                    gpu_available,
                    dev_mode,
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        popup_table.set(
            "text",
            lua.create_function(move |lua, args| {
                spawn_text_popup(
                    lua,
                    args,
                    request_sender.clone(),
                    windows.clone(),
                    chrome,
                    gpu_available,
                    dev_mode,
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        popup_table.set(
            "dialog",
            lua.create_function(move |lua, args| {
                spawn_dialog(
                    lua,
                    args,
                    request_sender.clone(),
                    windows.clone(),
                    chrome,
                    gpu_available,
                    dev_mode,
                )
            })?,
        )?;
    }

    api_table.set("popup", popup_table)?;

    let wallpaper_table = lua.create_table()?;

    {
        let media_manager = media_manager.clone();
        let request_sender = request_sender.clone();

        wallpaper_table.set(
            "set",
            lua.create_function(move |lua, args| {
                set_wallpaper(lua, args, media_manager.clone(), request_sender.clone())
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        wallpaper_table.set(
            "reset",
            lua.create_function(move |lua, args| {
                reset_wallpaper(lua, args, request_sender.clone())
            })?,
        )?;
    }

    api_table.set("wallpaper", wallpaper_table)?;

    {
        let request_sender = request_sender.clone();
        let audio_handles = audio_handles.clone();

        api_table.set(
            "play_audio",
            lua.create_function(move |lua, args| {
                play_audio(
                    lua,
                    args,
                    request_sender.clone(),
                    audio_handles.clone(),
                    dev_mode,
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "open_link",
            lua.create_function(move |lua, url| open_link(lua, url, request_sender.clone()))?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "show_notification",
            lua.create_function(move |lua, notification| {
                show_notification(lua, notification, request_sender.clone())
            })?,
        )?;
    }

    let monitors_table = lua.create_table()?;

    {
        let request_sender = request_sender.clone();

        monitors_table.set(
            "list",
            lua.create_function(move |lua, args| list_monitors(lua, args, request_sender.clone()))?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        monitors_table.set(
            "primary",
            lua.create_function(move |lua, args| {
                primary_monitor(lua, args, request_sender.clone())
            })?,
        )?;
    }

    api_table.set("monitors", monitors_table)?;

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "exit",
            lua.create_function(move |lua, x| exit(lua, x, request_sender.clone()))?,
        )?;
    }

    api_table.set(
        "after",
        lua.create_function(move |lua, args| after(lua, args, dev_mode))?,
    )?;

    api_table.set(
        "every",
        lua.create_function(move |lua, args| every(lua, args, dev_mode))?,
    )?;

    lua.globals().set("lewdware", api_table)?;

    Ok(())
}

fn get_media_type(
    name: String,
    types: MediaTypes,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    media_manager
        .get_media(name, types)
        .map_err(|err| err.into_lua_err())
}

fn get_media(_: &Lua, name: String, media_manager: MediaManager) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::ALL, media_manager)
}

fn get_image(_: &Lua, name: String, media_manager: MediaManager) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::IMAGE, media_manager)
}

fn get_video(_: &Lua, name: String, media_manager: MediaManager) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::VIDEO, media_manager)
}

fn get_audio(_: &Lua, name: String, media_manager: MediaManager) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::AUDIO, media_manager)
}

fn list_media_type(
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
enum OneOrMore<T> {
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
enum TagFilterInput {
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
struct QueryMediaOpts {
    #[serde(rename = "type")]
    types: Option<OneOrMore<MediaType>>,
    tags: Option<TagFilterInput>,
    weights: Option<HashMap<u64, f64>>,
}

impl FromLua for QueryMediaOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn list_media(
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
struct QueryMediaTypeOpts {
    tags: Option<TagFilterInput>,
    weights: Option<HashMap<u64, f64>>,
}

impl FromLua for QueryMediaTypeOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn list_images(
    _: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    list_media_type(MediaTypes::IMAGE, tags, media_manager)
}

fn list_videos(
    _: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    list_media_type(MediaTypes::VIDEO, tags, media_manager)
}

fn list_audio(
    _: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    list_media_type(MediaTypes::AUDIO, tags, media_manager)
}

fn random_media_type(
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

fn random_media(
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

fn random_image(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (tags, weights) = opts
        .map(|x| (x.tags.map(TagFilter::from), x.weights))
        .unwrap_or_default();

    random_media_type(lua, MediaTypes::IMAGE, tags, weights, media_manager)
}

fn random_video(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (tags, weights) = opts
        .map(|x| (x.tags.map(TagFilter::from), x.weights))
        .unwrap_or_default();

    random_media_type(lua, MediaTypes::VIDEO, tags, weights, media_manager)
}

fn random_audio(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (tags, weights) = opts
        .map(|x| (x.tags.map(TagFilter::from), x.weights))
        .unwrap_or_default();

    random_media_type(lua, MediaTypes::AUDIO, tags, weights, media_manager)
}

fn list_tags(_: &Lua, _: (), media_manager: MediaManager) -> mlua::Result<Vec<String>> {
    media_manager.list_tags().into_lua_err()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Coord {
    Pixel(i32),
    Percent { percent: f64 },
}

impl Coord {
    /// Returns a signed pixel value. Callers are responsible for clamping to valid bounds.
    pub fn to_pixels(&self, total_size: u32) -> i32 {
        match self {
            Coord::Pixel(x) => *x,
            Coord::Percent { percent } => ((percent * total_size as f64) / 100.0).round() as i32,
        }
    }
}

/// A font size: either a literal point size, or a percentage of the monitor's (logical) height.
/// Kept separate from `Coord` so literal values keep `f32` precision rather than rounding to a
/// whole pixel.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(untagged)]
pub enum FontSize {
    Value(f32),
    Percent { percent: f64 },
}

impl FontSize {
    /// Resolve to a concrete point size. `monitor_height` (logical) is the basis for `Percent`
    /// and is ignored for `Value`.
    pub fn to_pixels(self, monitor_height: u32) -> f32 {
        match self {
            FontSize::Value(size) => size,
            FontSize::Percent { percent } => (percent / 100.0) as f32 * monitor_height as f32,
        }
    }
}

/// The full 3x3 grid: horizontal (left/center/right) and vertical (top/center/bottom) are
/// independent, so e.g. `TopCenter` centers the window horizontally on the given x while leaving
/// y unadjusted -- see `resolve_x`/`resolve_y`, which each only look at the matching axis.
#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    #[serde(rename = "top-left")]
    #[default]
    TopLeft,
    #[serde(rename = "top-center")]
    TopCenter,
    #[serde(rename = "top-right")]
    TopRight,
    #[serde(rename = "center-left")]
    CenterLeft,
    #[serde(rename = "center")]
    Center,
    #[serde(rename = "center-right")]
    CenterRight,
    #[serde(rename = "bottom-left")]
    BottomLeft,
    #[serde(rename = "bottom-center")]
    BottomCenter,
    #[serde(rename = "bottom-right")]
    BottomRight,
}

/// A rectangle of the monitor a randomly placed window is confined to, as fractions of the usable
/// area (so `{ x = 0, y = 0, width = 0.5, height = 1 }` is its left half).
///
/// Only consulted for an axis the mode gave no coordinate for: `x` and `y` say exactly where the
/// window goes, and a region is a statement about where it goes *at random*. A window too big for
/// the region is centred on it rather than pinned to a corner (see [`random_position_in`]), which
/// is what lets a zero-size region name a single placement.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct SpawnRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl SpawnRegion {
    /// This region's horizontal span in pixels on a monitor of `width`, as `(start, span)`.
    pub fn horizontal(&self, width: u32) -> (f64, f64) {
        (self.x * width as f64, self.width * width as f64)
    }

    /// This region's vertical span in pixels on a monitor of `height`. See [`Self::horizontal`].
    pub fn vertical(&self, height: u32) -> (f64, f64) {
        (self.y * height as f64, self.height * height as f64)
    }
}

/// Where a coordinate falls relative to the window's edge along one axis.
#[derive(Clone, Copy)]
enum AxisAnchor {
    Start,
    Center,
    End,
}

impl AxisAnchor {
    fn resolve(self, coord: i32, size: u32) -> i32 {
        match self {
            Self::Start => coord,
            Self::Center => coord - (size / 2) as i32,
            Self::End => coord - size as i32,
        }
    }
}

impl Anchor {
    fn horizontal(self) -> AxisAnchor {
        match self {
            Self::TopLeft | Self::CenterLeft | Self::BottomLeft => AxisAnchor::Start,
            Self::TopCenter | Self::Center | Self::BottomCenter => AxisAnchor::Center,
            Self::TopRight | Self::CenterRight | Self::BottomRight => AxisAnchor::End,
        }
    }

    fn vertical(self) -> AxisAnchor {
        match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => AxisAnchor::Start,
            Self::CenterLeft | Self::Center | Self::CenterRight => AxisAnchor::Center,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => AxisAnchor::End,
        }
    }

    pub fn resolve_x(&self, coord: i32, width: u32) -> i32 {
        self.horizontal().resolve(coord, width)
    }

    pub fn resolve_y(&self, coord: i32, height: u32) -> i32 {
        self.vertical().resolve(coord, height)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SpawnWindowOpts {
    pub x: Option<Coord>,
    pub y: Option<Coord>,
    pub width: Option<Coord>,
    pub height: Option<Coord>,
    /// Multiplies the size the engine would otherwise pick for media, *before* the monitor caps
    /// apply -- so a scaled-up popup is still at most a third of the screen wide and half of it
    /// tall. That is the point of putting this here rather than leaving callers to compute a
    /// `width`: an explicit `width` is taken literally, and a caller scaling one from the media's
    /// own dimensions would be stepping around the caps rather than through them.
    ///
    /// Ignored when `width` or `height` is given (they already say the size exactly), and for
    /// windows that are not sized from media.
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub anchor: Anchor,
    /// Confines a *randomly placed* window to part of the monitor. Ignored on an axis `x` or `y`
    /// already pins exactly. See [`SpawnRegion`].
    #[serde(default)]
    pub region: Option<SpawnRegion>,
    pub monitor: Option<Monitor>,
    #[serde(default = "return_true")]
    pub decorations: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "return_true")]
    pub closeable: bool,
    /// Whether the window can be moved by dragging its custom header.
    #[serde(default)]
    pub draggable: bool,
    #[serde(default)]
    pub opacity: Option<f32>,
    #[serde(default)]
    pub transparent: Option<bool>,
    #[serde(default)]
    pub background_color: Option<Color>,
    #[serde(default)]
    pub click_through: bool,
    #[serde(default = "return_true")]
    pub clamp: bool,
    /// Which named look to draw the window's chrome and widgets with.
    ///
    /// `None` — the mode said nothing — means the user's own setting (`AppConfig::theme`), which
    /// is what almost every window should use. It is an `Option` rather than a defaulted enum
    /// precisely so that "said nothing" stays distinguishable from an explicit choice: naming a
    /// theme is also how a mode pins the metrics its layout arithmetic depends on, and that has
    /// to keep working even when the user's setting happens to be the same value.
    #[serde(default)]
    pub theme: Option<ThemeChoice>,
    /// Which palette that look is drawn in. `None` means the user's own setting
    /// (`AppConfig::appearance`), for the same reason as `theme`.
    #[serde(default)]
    pub appearance: Option<AppearanceChoice>,
}

impl Default for SpawnWindowOpts {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: None,
            height: None,
            scale: None,
            anchor: Anchor::default(),
            region: None,
            monitor: None,
            decorations: true,
            title: None,
            closeable: true,
            draggable: false,
            opacity: None,
            transparent: None,
            background_color: None,
            click_through: false,
            clamp: true,
            theme: None,
            appearance: None,
        }
    }
}

/// A window's size, before it's known whether media dimensions, an explicit default, or a text
/// measurement should drive it.
pub enum WindowSizeBehaviour {
    ResizeWithMedia {
        width: u32,
        height: u32,
    },
    UseDefaults {
        width: u32,
        height: u32,
    },
    MeasureText {
        text: String,
        font: TextFont,
        font_size: FontSize,
        outline_width: f32,
    },
}

/// A `SpawnWindowOpts` resolved against a monitor snapshot. Sizes, anchor math and clamping are
/// pure computation, so they happen here, on the Lua thread -- but a monitor's *current* absolute
/// position can only be read on the main thread (winit's `MonitorHandle` isn't `Send`, and can
/// change between this resolution and the window's actual, possibly much later, creation if
/// media is still decoding). So this carries `monitor_id` rather than a resolved position;
/// `LewdwareApp::finalize_window_opts` does that last step, fresh, right before the real window
/// is built.
#[derive(Debug, Clone)]
pub struct PopupSpawnOpts {
    pub monitor_id: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub outer_width: u32,
    pub outer_height: u32,
    pub gpu: bool,
    pub transparent: bool,
    pub force_opaque: bool,
    pub opacity: f32,
    pub click_through: bool,
    pub decorations: bool,
    pub title: Option<String>,
    pub closeable: bool,
    pub draggable: bool,
    pub background_color: Option<Color>,
    /// The palette the look is drawn in, still *unresolved*: `auto` depends on runtime state only
    /// the main thread can read, so — like `monitor_id` above — this carries the request and
    /// `WindowState::new` resolves it once the window exists. Safe to defer precisely because
    /// appearance never changes the metrics the sizing below is computed from.
    pub appearance: AppearanceChoice,
    /// The named look this window's chrome and widgets are drawn with, already concrete: the
    /// mode's own choice where it made one, and the user's setting otherwise. Its metrics are
    /// what the sizing above is computed from, which is why — unlike `appearance` — this one
    /// cannot be deferred past window creation.
    pub theme: Theme,
}

impl PopupSpawnOpts {
    /// Fails only on a window that could never be drawn: an explicitly requested width or height
    /// that resolves to zero or less. Sizes the engine derives itself (from media dimensions or a
    /// text measurement) are not checked here — those are the engine's own business, and
    /// `Header::new` clamps a degenerate one rather than failing a spawn the mode did not ask for.
    pub fn resolve(
        spawn_opts: SpawnWindowOpts,
        size_behaviour: WindowSizeBehaviour,
        monitor: &Monitor,
        chrome: ChromeDefaults,
        gpu_available: bool,
        mut gpu: bool,
        transparent: bool,
    ) -> Result<Self, InvalidWindowSize> {
        if !gpu_available {
            gpu = false;
        }

        // Before anything else: a zero-sized window has no content area, no drawable header, and
        // no way for the user to close it. Better a Lua error at the call site than a window that
        // exists but cannot be seen or dismissed.
        check_requested_size(&spawn_opts, monitor)?;
        let transparent = transparent && gpu;
        let force_opaque = spawn_opts.transparent == Some(false);

        let monitor_width = monitor.width;
        let monitor_height = monitor.height;

        let (width, height) = match size_behaviour {
            WindowSizeBehaviour::ResizeWithMedia { width, height } => calculate_media_popup_size(
                spawn_opts.width,
                spawn_opts.height,
                spawn_opts.scale,
                width,
                height,
                monitor_width,
                monitor_height,
            ),
            WindowSizeBehaviour::UseDefaults { width, height } => (
                spawn_opts
                    .width
                    .map(|w| w.to_pixels(monitor_width).max(0) as u32)
                    .unwrap_or(width),
                spawn_opts
                    .height
                    .map(|h| h.to_pixels(monitor_height).max(0) as u32)
                    .unwrap_or(height),
            ),
            WindowSizeBehaviour::MeasureText {
                text,
                font,
                font_size,
                outline_width,
            } => calculate_text_popup_size(
                spawn_opts.width.clone(),
                spawn_opts.height.clone(),
                &text,
                font,
                font_size.to_pixels(monitor_height),
                outline_width,
                monitor_width,
                monitor_height,
            ),
        };

        // Resolved here, once, so that a `native` alias becomes a concrete look before anything
        // downstream — window sizing included — ever asks what platform it is on. A mode that
        // named no theme gets the user's, which is the whole point of that setting.
        let theme = spawn_opts.theme.unwrap_or(chrome.theme).resolve();

        let (mut outer_width, mut outer_height) = (width, height);
        if spawn_opts.decorations {
            let (padding_x, padding_y) = theme.metrics().outer_padding();
            outer_width += padding_x;
            outer_height += padding_y;
        }

        let region = spawn_opts.region;
        let x: i32 = {
            let v = spawn_opts
                .x
                .map(|c| {
                    spawn_opts
                        .anchor
                        .resolve_x(c.to_pixels(monitor_width), outer_width)
                })
                .unwrap_or_else(|| match region {
                    Some(region) => {
                        let (start, span) = region.horizontal(monitor_width);
                        random_position_in(outer_width, start, span)
                    }
                    None => random_position(outer_width, monitor_width),
                });
            if spawn_opts.clamp {
                v.max(0)
                    .min(monitor_width.saturating_sub(outer_width) as i32)
            } else {
                v
            }
        };
        let y: i32 = {
            let v = spawn_opts
                .y
                .map(|c| {
                    spawn_opts
                        .anchor
                        .resolve_y(c.to_pixels(monitor_height), outer_height)
                })
                .unwrap_or_else(|| match region {
                    Some(region) => {
                        let (start, span) = region.vertical(monitor_height);
                        random_position_in(outer_height, start, span)
                    }
                    None => random_position(outer_height, monitor_height),
                });
            if spawn_opts.clamp {
                v.max(0)
                    .min(monitor_height.saturating_sub(outer_height) as i32)
            } else {
                v
            }
        };

        Ok(Self {
            monitor_id: monitor.id,
            x,
            y,
            width,
            height,
            outer_width,
            outer_height,
            gpu,
            transparent,
            force_opaque,
            opacity: spawn_opts.opacity.unwrap_or(1.0),
            click_through: spawn_opts.click_through,
            decorations: spawn_opts.decorations,
            title: spawn_opts.title,
            closeable: spawn_opts.closeable,
            draggable: spawn_opts.draggable,
            background_color: spawn_opts.background_color,
            appearance: spawn_opts.appearance.unwrap_or(chrome.appearance),
            theme,
        })
    }
}

/// A window size a mode asked for that cannot be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidWindowSize {
    /// `"width"` or `"height"`.
    pub axis: &'static str,
    pub pixels: i32,
}

impl std::fmt::Display for InvalidWindowSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "window {} must be greater than zero, got {}",
            self.axis, self.pixels
        )
    }
}

impl std::error::Error for InvalidWindowSize {}

/// Reject an explicitly requested size that resolves to zero or less, including a percentage that
/// rounds down to nothing on a small monitor.
fn check_requested_size(
    spawn_opts: &SpawnWindowOpts,
    monitor: &Monitor,
) -> Result<(), InvalidWindowSize> {
    for (axis, coord, total) in [
        ("width", &spawn_opts.width, monitor.width),
        ("height", &spawn_opts.height, monitor.height),
    ] {
        if let Some(coord) = coord {
            let pixels = coord.to_pixels(total);
            if pixels <= 0 {
                return Err(InvalidWindowSize { axis, pixels });
            }
        }
    }

    Ok(())
}

/// Pick the monitor a popup should spawn on: the one the mode explicitly asked for (from an
/// earlier `lewdware.monitors.list()`/`primary()` call), or a random one from a fresh
/// `list_monitors()` snapshot. Either way, only `.id` is trusted past this point -- see
/// `PopupSpawnOpts`'s doc comment.
fn resolve_monitor<T: EventPoster>(
    spawn_opts: &SpawnWindowOpts,
    request_sender: &RequestSender<T>,
) -> mlua::Result<Monitor> {
    match &spawn_opts.monitor {
        Some(monitor) => Ok(monitor.clone()),
        None => {
            let monitors = request_sender.list_monitors().into_lua_err()?;
            let mut rng = rand::rng();
            monitors
                .choose(&mut rng)
                .cloned()
                .ok_or_else(|| mlua::Error::runtime("no monitors available"))
        }
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFont {
    #[serde(rename = "default")]
    #[default]
    Default,
    #[serde(rename = "mono")]
    Mono,
    #[serde(rename = "display")]
    Display,
    #[serde(rename = "pixel")]
    Pixel,
}

/// What a *standalone* text popup uses when the mode names no size. A dialog's text has a better
/// answer available — the size of the theme it is sitting in — so it resolves its own default at
/// paint time instead; see [`TextStyle::font_size`].
pub const DEFAULT_TEXT_POPUP_FONT_SIZE: FontSize = FontSize::Value(32.0);

fn default_outline_width() -> f32 {
    2.0
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct TextStyle {
    #[serde(default)]
    pub font: TextFont,
    /// `None` means "whatever suits the surface this is drawn on", exactly as `color` does below.
    /// A standalone text popup floats over the desktop at
    /// [`DEFAULT_TEXT_POPUP_FONT_SIZE`], while a dialog's text takes its theme's body size — the
    /// same size the buttons and fields beside it are set in. Left as one fixed 32pt for both,
    /// a dialog's caption came out more than twice the height of its own controls, and appeared
    /// to change size between themes as the face changed underneath it.
    #[serde(default)]
    pub font_size: Option<FontSize>,
    /// `None` means "whatever suits the surface this is drawn on": a dialog's text follows its
    /// theme's palette, so it stays readable in a dark one, while a text popup — which floats over
    /// the desktop with no background of its own — keeps black.
    #[serde(default)]
    pub color: Option<Color>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub outline_color: Option<Color>,
    #[serde(default = "default_outline_width")]
    pub outline_width: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: TextFont::default(),
            font_size: None,
            color: None,
            bold: false,
            align: TextAlign::default(),
            outline_color: None,
            outline_width: default_outline_width(),
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct SpawnTextOpts {
    #[serde(flatten)]
    style: TextStyle,
    #[serde(flatten)]
    window_opts: SpawnWindowOpts,
}

impl FromLua for SpawnTextOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn spawn_text_popup<T: EventPoster>(
    _: &Lua,
    (text, opts): (String, Option<SpawnTextOpts>),
    request_sender: RequestSender<T>,
    windows: Windows<T>,
    chrome: ChromeDefaults,
    gpu_available: bool,
    dev_mode: bool,
) -> mlua::Result<Rc<TextWindow<T>>> {
    let mut opts = opts.unwrap_or_default();

    let monitor = resolve_monitor(&opts.window_opts, &request_sender)?;

    // Unlike other popup types, text defaults to a transparent (GPU-rendered) window, since
    // text is usually meant to float over the desktop rather than sit in an opaque panel.
    let transparent = opts.window_opts.transparent.unwrap_or(true);

    // Apply the text popup's own default, then resolve a percentage to a concrete point size now
    // that the monitor (and so its height, the basis for `FontSize::Percent`) is known. From here
    // on `font_size` is always `Some(FontSize::Value)`.
    let requested = opts.style.font_size.unwrap_or(DEFAULT_TEXT_POPUP_FONT_SIZE);
    opts.style.font_size = Some(FontSize::Value(requested.to_pixels(monitor.height)));
    let outline_width = if opts.style.outline_color.is_some() {
        opts.style.outline_width
    } else {
        0.0
    };

    let window_opts = PopupSpawnOpts::resolve(
        opts.window_opts,
        WindowSizeBehaviour::MeasureText {
            text: text.clone(),
            font: opts.style.font,
            font_size: opts.style.font_size.unwrap_or(DEFAULT_TEXT_POPUP_FONT_SIZE),
            outline_width,
        },
        &monitor,
        chrome,
        gpu_available,
        transparent,
        transparent,
    )
    .into_lua_err()?;

    let props = request_sender.spawn_text(text.clone(), opts.style, window_opts)?;

    let id = props.window_id;

    let window = Rc::new(TextWindow::new(
        props,
        text,
        request_sender.window_sender(id),
        dev_mode,
    ));

    windows
        .try_borrow_mut()
        .into_lua_err()?
        .insert(id, Window::Text(window.clone()));

    Ok(window)
}

#[derive(Serialize, Deserialize, Default)]
pub struct SpawnImageOpts {
    #[serde(flatten)]
    window_opts: SpawnWindowOpts,
}

impl FromLua for SpawnImageOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn spawn_image_popup<T: EventPoster>(
    _: &Lua,
    (image, opts): (Media, Option<SpawnImageOpts>),
    request_sender: RequestSender<T>,
    windows: Windows<T>,
    chrome: ChromeDefaults,
    gpu_available: bool,
    dev_mode: bool,
) -> mlua::Result<Rc<ImageWindow<T>>> {
    let mut opts = opts.unwrap_or_default();

    let (image_width, image_height, media_transparent) = match image.media_data {
        MediaData::Image {
            width,
            height,
            transparent,
        } => (width, height, transparent),
        _ => return Err("`image` is not an image".into_lua_err()),
    };

    if opts.window_opts.transparent.is_none() {
        let needs_transparent = media_transparent
            || opts.window_opts.opacity.is_some_and(|o| o < 1.0)
            || opts.window_opts.background_color.is_some_and(|c| c.a < 1.0);
        if needs_transparent {
            opts.window_opts.transparent = Some(true);
        }
    }

    let monitor = resolve_monitor(&opts.window_opts, &request_sender)?;
    let transparent = opts.window_opts.transparent.unwrap_or(false);
    let window_opts = PopupSpawnOpts::resolve(
        opts.window_opts,
        WindowSizeBehaviour::ResizeWithMedia {
            width: image_width,
            height: image_height,
        },
        &monitor,
        chrome,
        gpu_available,
        transparent,
        transparent,
    )
    .into_lua_err()?;

    // Monitor pick and size resolution happen here, on the Lua thread; only the actual window
    // creation and (slow) decode happen on the main thread, which still acks the spawn — with a
    // fully accurate `WindowProps` — before the decode even starts. See `App::spawn_image`.
    let props = request_sender.spawn_image(image.id, window_opts)?;

    let id = props.window_id;

    let window = Rc::new(ImageWindow::new(
        props,
        image,
        request_sender.window_sender(id),
        dev_mode,
    ));

    windows
        .try_borrow_mut()
        .into_lua_err()?
        .insert(id, Window::Image(window.clone()));

    Ok(window)
}

#[derive(Serialize, Deserialize)]
pub struct SpawnVideoOpts {
    #[serde(rename = "loop")]
    #[serde(default = "return_true")]
    loop_video: bool,
    #[serde(default = "return_true")]
    audio: bool,
    #[serde(default = "return_one")]
    volume: f32,
    #[serde(flatten)]
    window_opts: SpawnWindowOpts,
}

impl Default for SpawnVideoOpts {
    fn default() -> Self {
        Self {
            loop_video: true,
            audio: true,
            volume: 1.0,
            window_opts: Default::default(),
        }
    }
}

fn return_true() -> bool {
    true
}

fn return_one() -> f32 {
    1.0
}

impl FromLua for SpawnVideoOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn spawn_video_popup<T: EventPoster>(
    _: &Lua,
    (video, opts): (Media, Option<SpawnVideoOpts>),
    request_sender: RequestSender<T>,
    windows: Windows<T>,
    chrome: ChromeDefaults,
    gpu_available: bool,
    dev_mode: bool,
) -> mlua::Result<Rc<VideoWindow<T>>> {
    let mut opts = opts.unwrap_or_default();

    let (video_width, video_height, media_transparent) = match video.media_data {
        MediaData::Video {
            width,
            height,
            transparent,
            ..
        } => (width, height, transparent),
        _ => return Err("`video` is not an video".into_lua_err()),
    };

    if opts.window_opts.transparent.is_none() {
        let needs_transparent = media_transparent
            || opts.window_opts.opacity.is_some_and(|o| o < 1.0)
            || opts.window_opts.background_color.is_some_and(|c| c.a < 1.0);
        if needs_transparent {
            opts.window_opts.transparent = Some(true);
        }
    }

    let monitor = resolve_monitor(&opts.window_opts, &request_sender)?;
    let transparent = opts.window_opts.transparent.unwrap_or(false);
    // Always wants GPU rendering (unlike images, not conditional on transparency) -- see the
    // original comment on `App::spawn_video`.
    let window_opts = PopupSpawnOpts::resolve(
        opts.window_opts,
        WindowSizeBehaviour::ResizeWithMedia {
            width: video_width,
            height: video_height,
        },
        &monitor,
        chrome,
        gpu_available,
        true,
        transparent,
    )
    .into_lua_err()?;

    // As with images, monitor pick / size resolution happen here, on the Lua thread; only the
    // actual window creation / decode happen on the main thread -- see `App::spawn_video`.
    let props = request_sender.spawn_video(
        video.id,
        opts.loop_video,
        opts.audio,
        opts.volume,
        window_opts,
    )?;

    let id = props.window_id;

    let window = Rc::new(VideoWindow::new(
        props,
        video,
        request_sender.window_sender(id),
        dev_mode,
    ));

    windows
        .try_borrow_mut()
        .into_lua_err()?
        .insert(id, Window::Video(window.clone()));

    Ok(window)
}

/// A button offered by a `ButtonsElement`. `default` marks the button that pressing Enter in an
/// input element acts as a click on — at most one per dialog, checked in `spawn_dialog`/
/// `update_dialog_element`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DialogButton {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub default: bool,
}

impl FromLua for DialogButton {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

/// One element of a dialog's vertical stack. Mirrors `TextElement`/`ImageElement`/
/// `InputElement`/`ButtonsElement` in the v1 draft — a deliberately closed vocabulary.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum DialogElement<I = Media> {
    #[serde(rename = "text")]
    Text {
        id: Option<String>,
        text: String,
        #[serde(flatten)]
        style: TextStyle,
    },
    #[serde(rename = "image")]
    Image { id: Option<String>, image: I },
    #[serde(rename = "input")]
    Input {
        id: String,
        placeholder: Option<String>,
        initial_value: Option<String>,
    },
    #[serde(rename = "buttons")]
    Buttons {
        id: Option<String>,
        #[serde(default)]
        options: Vec<DialogButton>,
    },
}

impl<I> DialogElement<I> {
    pub fn try_map_image<O, E>(
        self,
        map: impl FnOnce(I) -> Result<O, E>,
    ) -> Result<DialogElement<O>, E> {
        Ok(match self {
            Self::Text { id, text, style } => DialogElement::Text { id, text, style },
            Self::Image { id, image } => DialogElement::Image {
                id,
                image: map(image)?,
            },
            Self::Input {
                id,
                placeholder,
                initial_value,
            } => DialogElement::Input {
                id,
                placeholder,
                initial_value,
            },
            Self::Buttons { id, options } => DialogElement::Buttons { id, options },
        })
    }
}

impl FromLua for DialogElement<Media> {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

/// A partial update to one dialog element, via `DialogWindow:update(id, props)`. Fields not
/// relevant to the target element's type are ignored. `image` updates aren't supported yet (the
/// release plan's cut line already flags dialog image polish as a first-to-cut item) — the
/// element keeps its original image.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DialogElementUpdate {
    pub text: Option<String>,
    pub font: Option<TextFont>,
    pub font_size: Option<FontSize>,
    pub color: Option<Color>,
    pub bold: Option<bool>,
    pub align: Option<TextAlign>,
    pub outline_color: Option<Color>,
    pub outline_width: Option<f32>,
    pub placeholder: Option<String>,
    pub value: Option<String>,
    pub options: Option<Vec<DialogButton>>,
}

impl FromLua for DialogElementUpdate {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn dialog_has_more_than_one_default_button(elements: &[DialogElement]) -> bool {
    elements
        .iter()
        .filter_map(|element| match element {
            DialogElement::Buttons { options, .. } => {
                Some(options.iter().filter(|o| o.default).count())
            }
            _ => None,
        })
        .sum::<usize>()
        > 1
}

#[derive(Serialize, Deserialize)]
struct SpawnDialogOpts {
    elements: Vec<DialogElement>,
    #[serde(flatten)]
    window_opts: SpawnWindowOpts,
}

impl FromLua for SpawnDialogOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn spawn_dialog<T: EventPoster>(
    _: &Lua,
    opts: SpawnDialogOpts,
    request_sender: RequestSender<T>,
    windows: Windows<T>,
    chrome: ChromeDefaults,
    gpu_available: bool,
    dev_mode: bool,
) -> mlua::Result<Rc<DialogWindow<T>>> {
    if dialog_has_more_than_one_default_button(&opts.elements) {
        return Err(mlua::Error::runtime(
            "at most one button in a dialog may be marked `default`",
        ));
    }

    // Validated here (rather than deep in `app.rs`) so the rest of the spawn/decode pipeline can
    // assume every `image` element's `Media` is actually an image.
    for element in &opts.elements {
        if let DialogElement::Image { image, .. } = element {
            let type_name = match image.media_data {
                MediaData::Image { .. } => continue,
                MediaData::Video { .. } => "video",
                MediaData::Audio { .. } => "audio",
            };
            return Err(mlua::Error::runtime(format!(
                "dialog image element's `image` must be an image, not {type_name}"
            )));
        }
    }

    let monitor = resolve_monitor(&opts.window_opts, &request_sender)?;
    let auto_transparent = opts.window_opts.opacity.is_some_and(|o| o < 1.0);
    let transparent = opts.window_opts.transparent.unwrap_or(auto_transparent);
    let window_opts = PopupSpawnOpts::resolve(
        opts.window_opts,
        WindowSizeBehaviour::UseDefaults {
            width: 400,
            height: 400,
        },
        &monitor,
        chrome,
        gpu_available,
        transparent,
        transparent,
    )
    .into_lua_err()?;

    let props = request_sender.spawn_dialog(opts.elements, window_opts)?;

    let id = props.window_id;

    let window = Rc::new(DialogWindow::new(
        props,
        request_sender.window_sender(id),
        dev_mode,
    ));

    windows
        .try_borrow_mut()
        .into_lua_err()?
        .insert(id, Window::Dialog(window.clone()));

    Ok(window)
}

fn set_wallpaper<T: EventPoster>(
    _: &Lua,
    image: Media,
    media_manager: MediaManager,
    request_sender: RequestSender<T>,
) -> mlua::Result<bool> {
    if !matches!(image.media_data, MediaData::Image { .. }) {
        return Err("`image` is not an image".into_lua_err());
    }

    let file = media_manager.get_image_file(image.id).into_lua_err()?;

    request_sender.set_wallpaper(file).into_lua_err()
}

fn reset_wallpaper<T: EventPoster>(
    _: &Lua,
    _: (),
    request_sender: RequestSender<T>,
) -> mlua::Result<()> {
    request_sender.reset_wallpaper().into_lua_err()
}

#[derive(Serialize, Deserialize)]
struct PlayAudioOpts {
    #[serde(rename = "loop")]
    #[serde(default)]
    loop_audio: bool,
    #[serde(default = "return_one")]
    volume: f32,
}

impl Default for PlayAudioOpts {
    fn default() -> Self {
        Self {
            loop_audio: false,
            volume: 1.0,
        }
    }
}

impl FromLua for PlayAudioOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn play_audio<T: EventPoster>(
    _: &Lua,
    (audio, opts): (Media, Option<PlayAudioOpts>),
    request_sender: RequestSender<T>,
    audio_handles: AudioHandles<T>,
    dev_mode: bool,
) -> mlua::Result<Rc<AudioHandle<T>>> {
    let opts = opts.unwrap_or_default();

    if !matches!(audio.media_data, MediaData::Audio { .. }) {
        return Err("`audio` is not a audio".into_lua_err());
    }

    // Decode happens on the main thread now, after the ack — see `App::spawn_audio`.
    let id = request_sender.spawn_audio(audio.id, opts.loop_audio, opts.volume)?;

    let audio_handle = Rc::new(AudioHandle::new(
        id,
        audio,
        request_sender.audio_sender(id),
        dev_mode,
    ));

    audio_handles
        .try_borrow_mut()
        .into_lua_err()?
        .insert(id, audio_handle.clone());

    Ok(audio_handle)
}

fn open_link<T: EventPoster>(
    _: &Lua,
    url: String,
    request_sender: RequestSender<T>,
) -> mlua::Result<bool> {
    request_sender.open_link(url).into_lua_err()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Notification {
    pub summary: Option<String>,
    pub body: String,
}

impl FromLua for Notification {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn show_notification<T: EventPoster>(
    _: &Lua,
    notification: Notification,
    request_sender: RequestSender<T>,
) -> mlua::Result<bool> {
    request_sender
        .show_notification(notification)
        .into_lua_err()
}

fn list_monitors<T: EventPoster>(
    _: &Lua,
    _: (),
    request_sender: RequestSender<T>,
) -> mlua::Result<Vec<Monitor>> {
    request_sender.list_monitors().into_lua_err()
}

fn primary_monitor<T: EventPoster>(
    _: &Lua,
    _: (),
    request_sender: RequestSender<T>,
) -> mlua::Result<Monitor> {
    request_sender.primary_monitor().into_lua_err()
}

fn exit<T: EventPoster>(_: &Lua, _: (), request_sender: RequestSender<T>) -> mlua::Result<()> {
    request_sender.exit().into_lua_err()
}

fn after(_: &Lua, (ms, function): (u64, mlua::Function), dev_mode: bool) -> mlua::Result<Timer> {
    Ok(Timer::new(Duration::from_millis(ms), function, dev_mode))
}

fn every(_: &Lua, (ms, function): (u64, mlua::Function), dev_mode: bool) -> mlua::Result<Interval> {
    Ok(Interval::new(Duration::from_millis(ms), function, dev_mode))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor() -> Monitor {
        Monitor {
            id: 1,
            primary: true,
            width: 1000,
            height: 800,
            scale_factor: 1.0,
        }
    }

    fn resolve_with(opts: SpawnWindowOpts) -> Result<PopupSpawnOpts, InvalidWindowSize> {
        // A fixed, platform-independent pair, so that a test saying nothing about chrome gets the
        // same answer on every machine. Tests about the user's own setting name one explicitly.
        resolve_with_chrome(
            opts,
            ChromeDefaults {
                theme: ThemeChoice::Plain,
                appearance: AppearanceChoice::Light,
            },
        )
    }

    fn resolve_with_chrome(
        opts: SpawnWindowOpts,
        chrome: ChromeDefaults,
    ) -> Result<PopupSpawnOpts, InvalidWindowSize> {
        PopupSpawnOpts::resolve(
            opts,
            WindowSizeBehaviour::UseDefaults {
                width: 200,
                height: 150,
            },
            &monitor(),
            chrome,
            false,
            false,
            false,
        )
    }

    /// The point of the user's setting: a mode that never mentions themes -- the common case, and
    /// every mode written before themes existed -- draws in the look the user picked, with no
    /// cooperation from the mode's author required.
    #[test]
    fn a_window_the_mode_did_not_theme_takes_the_users_choice() {
        let chrome = ChromeDefaults {
            theme: ThemeChoice::Redmond,
            appearance: AppearanceChoice::Dark,
        };
        let resolved = resolve_with_chrome(SpawnWindowOpts::default(), chrome).unwrap();

        assert_eq!(resolved.theme, Theme::Redmond);
        assert_eq!(resolved.appearance, AppearanceChoice::Dark);

        // And it is sized by that theme's metrics, not by the API default's.
        let (pad_x, pad_y) = Theme::Redmond.metrics().outer_padding();
        assert_eq!(resolved.outer_width, resolved.width + pad_x);
        assert_eq!(resolved.outer_height, resolved.height + pad_y);
    }

    /// The other half of that bargain: a mode that *does* name a look gets it, so a window built
    /// to impersonate a specific OS still can, and so naming a theme remains the way to pin the
    /// metrics a mode's own layout arithmetic depends on.
    #[test]
    fn a_theme_the_mode_named_beats_the_users_choice() {
        let chrome = ChromeDefaults {
            theme: ThemeChoice::Redmond,
            appearance: AppearanceChoice::Dark,
        };
        let opts = SpawnWindowOpts {
            theme: Some(ThemeChoice::Aqua),
            appearance: Some(AppearanceChoice::Light),
            ..Default::default()
        };
        let resolved = resolve_with_chrome(opts, chrome).unwrap();

        assert_eq!(resolved.theme, Theme::Aqua);
        assert_eq!(resolved.appearance, AppearanceChoice::Light);
    }

    /// Each axis falls back on its own: a mode fixing the look it draws in has not thereby said
    /// anything about light or dark, and the user's answer to that still stands.
    #[test]
    fn the_two_axes_fall_back_independently() {
        let chrome = ChromeDefaults {
            theme: ThemeChoice::Redmond,
            appearance: AppearanceChoice::Dark,
        };
        let resolved = resolve_with_chrome(
            SpawnWindowOpts {
                theme: Some(ThemeChoice::Aqua),
                ..Default::default()
            },
            chrome,
        )
        .unwrap();

        assert_eq!(resolved.theme, Theme::Aqua);
        assert_eq!(resolved.appearance, AppearanceChoice::Dark);
    }

    /// A theme name this engine has never heard of -- a config written by a newer one -- must not
    /// take chrome down with it, and `plain` would be a poor guess at what the user meant: they
    /// asked for *some* real look.
    #[test]
    fn an_unknown_configured_theme_falls_back_to_the_product_default() {
        let chrome = ChromeDefaults::from_config("some-future-theme", "sepia");

        assert_eq!(chrome, ChromeDefaults::default());
        assert_eq!(chrome.theme, ThemeChoice::Native);
        assert_eq!(chrome.appearance, AppearanceChoice::Auto);
    }

    /// Every name the config app can write must round-trip, or a user's saved choice silently
    /// becomes someone else's.
    #[test]
    fn every_theme_name_reads_back_as_the_choice_it_names() {
        for &choice in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::from_name(choice.name()), Some(choice));
        }
        for &choice in AppearanceChoice::ALL {
            assert_eq!(AppearanceChoice::from_name(choice.name()), Some(choice));
        }
    }

    #[test]
    fn a_theme_is_resolved_before_the_window_is_sized() {
        // `native` is an alias; what reaches the window must already be a concrete look, and the
        // outer size must have been computed from *that* theme's metrics.
        let opts = SpawnWindowOpts {
            theme: Some(ThemeChoice::Native),
            ..Default::default()
        };
        let resolved = resolve_with(opts).unwrap();

        assert!(crate::window::theme::ALL_THEMES.contains(&resolved.theme));

        let (pad_x, pad_y) = resolved.theme.metrics().outer_padding();
        assert_eq!(resolved.outer_width, resolved.width + pad_x);
        assert_eq!(resolved.outer_height, resolved.height + pad_y);
    }

    /// Each theme's own metrics drive the outer size, which is the whole reason the theme has to
    /// be known this early rather than at draw time.
    #[test]
    fn each_theme_sizes_its_window_by_its_own_metrics() {
        for &theme in crate::window::theme::ALL_THEMES {
            let choice = match theme {
                Theme::Plain => ThemeChoice::Plain,
                Theme::Fluent => ThemeChoice::Fluent,
                Theme::Redmond => ThemeChoice::Redmond,
                Theme::Aqua => ThemeChoice::Aqua,
                Theme::Adwaita => ThemeChoice::Adwaita,
                Theme::Breeze => ThemeChoice::Breeze,
                Theme::Platinum => ThemeChoice::Platinum,
                Theme::Cde => ThemeChoice::Cde,
            };

            let resolved = resolve_with(SpawnWindowOpts {
                theme: Some(choice),
                ..Default::default()
            })
            .unwrap();

            let (pad_x, pad_y) = theme.metrics().outer_padding();
            assert_eq!(resolved.theme, theme);
            assert_eq!(resolved.outer_width, resolved.width + pad_x, "{theme:?}");
            assert_eq!(resolved.outer_height, resolved.height + pad_y, "{theme:?}");
        }
    }

    #[test]
    fn an_undecorated_window_has_no_padding_whatever_its_theme() {
        let resolved = resolve_with(SpawnWindowOpts {
            theme: Some(ThemeChoice::Redmond),
            decorations: false,
            ..Default::default()
        })
        .unwrap();

        assert_eq!(resolved.outer_width, resolved.width);
        assert_eq!(resolved.outer_height, resolved.height);
    }

    /// A zero-sized window has no content area, no drawable header and no close button, so asking
    /// for one is a mistake worth reporting rather than a window that cannot be seen or dismissed.
    #[test]
    fn a_zero_or_negative_requested_size_is_rejected() {
        for (width, height, axis) in [
            (Some(Coord::Pixel(0)), None, "width"),
            (None, Some(Coord::Pixel(0)), "height"),
            (Some(Coord::Pixel(-10)), None, "width"),
            (None, Some(Coord::Pixel(-1)), "height"),
            // A percentage that rounds down to nothing is the same mistake, less obviously.
            (Some(Coord::Percent { percent: 0.0 }), None, "width"),
            (Some(Coord::Percent { percent: 0.04 }), None, "width"),
        ] {
            let error = resolve_with(SpawnWindowOpts {
                width,
                height,
                ..Default::default()
            })
            .expect_err("expected a rejection");

            assert_eq!(error.axis, axis);
            assert!(error.pixels <= 0);
        }
    }

    #[test]
    fn a_positive_requested_size_is_accepted() {
        for (width, height) in [
            (Some(Coord::Pixel(1)), None),
            (None, Some(Coord::Pixel(1))),
            (Some(Coord::Percent { percent: 0.1 }), None),
            (Some(Coord::Pixel(400)), Some(Coord::Pixel(300))),
        ] {
            assert!(
                resolve_with(SpawnWindowOpts {
                    width: width.clone(),
                    height: height.clone(),
                    ..Default::default()
                })
                .is_ok(),
                "{width:?} x {height:?} should be allowed"
            );
        }
    }

    /// Sizes the engine derives itself are not rejected: a mode that never asked for a size should
    /// not have a spawn fail because a measurement came out small.
    #[test]
    fn an_engine_derived_size_is_never_rejected() {
        let resolved = PopupSpawnOpts::resolve(
            SpawnWindowOpts::default(),
            WindowSizeBehaviour::UseDefaults {
                width: 0,
                height: 0,
            },
            &monitor(),
            ChromeDefaults::default(),
            false,
            false,
            false,
        );

        assert!(resolved.is_ok());
    }

    /// The error text is what a mode author sees in `lw mode dev`, so it should name the axis and
    /// the value rather than just failing.
    #[test]
    fn the_size_error_names_the_axis_and_value() {
        let error = resolve_with(SpawnWindowOpts {
            width: Some(Coord::Pixel(0)),
            ..Default::default()
        })
        .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("width"), "{message}");
        assert!(message.contains('0'), "{message}");
    }

    /// The Lua-facing spelling of the option, since a mode writes it as a string.
    #[test]
    fn the_theme_option_deserialises_from_its_lua_name() {
        let opts: SpawnWindowOpts =
            serde_json::from_str(r#"{"theme": "native-retro"}"#).expect("should deserialise");
        assert_eq!(opts.theme, Some(ThemeChoice::NativeRetro));

        // Absent stays absent, rather than collapsing into a default here: it is what tells
        // `resolve` to use the user's own setting instead of a look the mode chose.
        let opts: SpawnWindowOpts = serde_json::from_str("{}").expect("should deserialise");
        assert_eq!(opts.theme, None);
        assert_eq!(opts.appearance, None);
    }

    #[test]
    fn draggable_defaults_to_false_and_resolves_when_enabled() {
        let defaults: SpawnWindowOpts =
            serde_json::from_str("{}").expect("should deserialise defaults");
        assert!(!defaults.draggable);

        let opts: SpawnWindowOpts =
            serde_json::from_str(r#"{"draggable": true}"#).expect("should deserialise option");
        assert!(resolve_with(opts).unwrap().draggable);
    }

    /// An unknown name is an error rather than a silent fallback: for a mode author that is a
    /// typo worth surfacing. A mode passing on a name it got from somewhere less trustworthy
    /// checks it against `lewdware.themes` first.
    #[test]
    fn an_unknown_theme_name_is_rejected() {
        let result: Result<SpawnWindowOpts, _> = serde_json::from_str(r#"{"theme": "win7"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn anchor_resolves_each_axis_independently() {
        // A 100x50 window at coordinate (200, 80) on each axis.
        let (coord, width, height) = (200, 100u32, 50u32);

        // Horizontal: left leaves x unadjusted, center subtracts half the width, right
        // subtracts the full width -- regardless of which vertical anchor it's paired with.
        assert_eq!(Anchor::TopLeft.resolve_x(coord, width), 200);
        assert_eq!(Anchor::CenterLeft.resolve_x(coord, width), 200);
        assert_eq!(Anchor::BottomLeft.resolve_x(coord, width), 200);

        assert_eq!(Anchor::TopCenter.resolve_x(coord, width), 150);
        assert_eq!(Anchor::Center.resolve_x(coord, width), 150);
        assert_eq!(Anchor::BottomCenter.resolve_x(coord, width), 150);

        assert_eq!(Anchor::TopRight.resolve_x(coord, width), 100);
        assert_eq!(Anchor::CenterRight.resolve_x(coord, width), 100);
        assert_eq!(Anchor::BottomRight.resolve_x(coord, width), 100);

        // Vertical: same shape, independent of which horizontal anchor it's paired with.
        assert_eq!(Anchor::TopLeft.resolve_y(coord, height), 200);
        assert_eq!(Anchor::TopCenter.resolve_y(coord, height), 200);
        assert_eq!(Anchor::TopRight.resolve_y(coord, height), 200);

        assert_eq!(Anchor::CenterLeft.resolve_y(coord, height), 175);
        assert_eq!(Anchor::Center.resolve_y(coord, height), 175);
        assert_eq!(Anchor::CenterRight.resolve_y(coord, height), 175);

        assert_eq!(Anchor::BottomLeft.resolve_y(coord, height), 150);
        assert_eq!(Anchor::BottomCenter.resolve_y(coord, height), 150);
        assert_eq!(Anchor::BottomRight.resolve_y(coord, height), 150);
    }

    #[test]
    fn anchor_mixed_axes_combine_independently() {
        // "top-center" should behave like TopLeft on y but Center on x -- the actual point of
        // the 9-point grid over the old 3-point diagonal-only version.
        let (coord, size) = (200, 100u32);
        assert_eq!(
            Anchor::TopCenter.resolve_x(coord, size),
            Anchor::Center.resolve_x(coord, size)
        );
        assert_eq!(
            Anchor::TopCenter.resolve_y(coord, size),
            Anchor::TopLeft.resolve_y(coord, size)
        );
    }

    #[test]
    fn anchor_serializes_to_documented_strings() {
        let pairs = [
            (Anchor::TopLeft, "top-left"),
            (Anchor::TopCenter, "top-center"),
            (Anchor::TopRight, "top-right"),
            (Anchor::CenterLeft, "center-left"),
            (Anchor::Center, "center"),
            (Anchor::CenterRight, "center-right"),
            (Anchor::BottomLeft, "bottom-left"),
            (Anchor::BottomCenter, "bottom-center"),
            (Anchor::BottomRight, "bottom-right"),
        ];

        for (anchor, expected) in pairs {
            let json = serde_json::to_string(&anchor).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let parsed: Anchor = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, anchor);
        }
    }
}
