use std::{collections::HashMap, rc::Rc, time::Duration};

use mlua::{ExternalError, ExternalResult, FromLua, IntoLua, Lua, LuaSerdeExt};
use serde::{Deserialize, Serialize};
use shared::mode::OptionValue;

#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let r = (self.r * 255.0).round() as u8;
        let g = (self.g * 255.0).round() as u8;
        let b = (self.b * 255.0).round() as u8;
        let a = (self.a * 255.0).round() as u8;
        if a == 255 {
            serializer.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}"))
        } else {
            serializer.serialize_str(&format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
        }
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        let hex = s
            .strip_prefix('#')
            .ok_or_else(|| serde::de::Error::custom("color must start with '#'"))?;

        fn channel(s: &str) -> Option<f32> {
            u8::from_str_radix(s, 16).ok().map(|v| v as f32 / 255.0)
        }

        match hex.len() {
            6 => Ok(Color {
                r: channel(&hex[0..2])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                g: channel(&hex[2..4])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                b: channel(&hex[4..6])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                a: 1.0,
            }),
            8 => Ok(Color {
                r: channel(&hex[0..2])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                g: channel(&hex[2..4])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                b: channel(&hex[4..6])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
                a: channel(&hex[6..8])
                    .ok_or_else(|| serde::de::Error::custom("invalid hex digit"))?,
            }),
            _ => Err(serde::de::Error::custom(
                "color must be '#rrggbb' or '#rrggbbaa'",
            )),
        }
    }
}

use rand::seq::IndexedRandom;

use crate::{
    lua::{
        AudioHandles, Media, MediaData, MediaType, Window, Windows,
        audio::AudioHandle,
        interval::{Interval, Timer},
        request::RequestSender,
        window::{DialogWindow, ImageWindow, TextWindow, VideoWindow},
    },
    media::{MediaManager, MediaTypes, TagFilter},
    monitor::Monitor,
    utils::{calculate_media_popup_size, calculate_text_popup_size, random_position},
    window::HEADER_HEIGHT,
};

/// Data available once, at mode startup, as opposed to `create_api`'s other parameters, which are
/// live handles used throughout the mode's lifetime.
pub struct ApiOptions {
    pub pack_info: Option<crate::lua::PackInfo>,
    pub config: HashMap<String, OptionValue>,
    pub gpu_available: bool,
    pub dev_mode: bool,
}

pub fn create_api(
    lua: &Lua,
    request_sender: RequestSender,
    media_manager: MediaManager,
    windows: Windows,
    audio_handles: AudioHandles,
    storage: crate::lua::storage::Storage,
    options: ApiOptions,
) -> mlua::Result<()> {
    let ApiOptions {
        pack_info,
        config,
        gpu_available,
        dev_mode,
    } = options;

    let api_table = lua.create_table()?;

    api_table.set("config", config.into_lua(lua)?)?;

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
        Some(QueryMediaOpts { types, tags }) => (
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
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    media_manager
        .random_media(types, tags)
        .map_err(|err| err.into_lua_err())
}

fn random_media(
    lua: &Lua,
    opts: Option<QueryMediaOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (types, tags) = match opts {
        Some(QueryMediaOpts { types, tags }) => (
            types.map_or(MediaTypes::ALL, MediaTypes::from),
            tags.map(TagFilter::from),
        ),
        None => (MediaTypes::ALL, None),
    };

    random_media_type(lua, types, tags, media_manager)
}

fn random_image(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    random_media_type(lua, MediaTypes::IMAGE, tags, media_manager)
}

fn random_video(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    random_media_type(lua, MediaTypes::VIDEO, tags, media_manager)
}

fn random_audio(
    lua: &Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let tags = opts.and_then(|x| x.tags).map(TagFilter::from);

    random_media_type(lua, MediaTypes::AUDIO, tags, media_manager)
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
    #[serde(default)]
    pub anchor: Anchor,
    pub monitor: Option<Monitor>,
    #[serde(default = "return_true")]
    pub decorations: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "return_true")]
    pub closeable: bool,
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
}

impl Default for SpawnWindowOpts {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: None,
            height: None,
            anchor: Anchor::default(),
            monitor: None,
            decorations: true,
            title: None,
            closeable: true,
            opacity: None,
            transparent: None,
            background_color: None,
            click_through: false,
            clamp: true,
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
    pub background_color: Option<Color>,
}

impl PopupSpawnOpts {
    pub fn resolve(
        spawn_opts: SpawnWindowOpts,
        size_behaviour: WindowSizeBehaviour,
        monitor: &Monitor,
        gpu_available: bool,
        mut gpu: bool,
        transparent: bool,
    ) -> Self {
        if !gpu_available {
            gpu = false;
        }
        let transparent = transparent && gpu;
        let force_opaque = spawn_opts.transparent == Some(false);

        let monitor_width = monitor.width;
        let monitor_height = monitor.height;

        let (width, height) = match size_behaviour {
            WindowSizeBehaviour::ResizeWithMedia { width, height } => calculate_media_popup_size(
                spawn_opts.width,
                spawn_opts.height,
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

        let (mut outer_width, mut outer_height) = (width, height);
        if spawn_opts.decorations {
            outer_width += 2;
            outer_height += HEADER_HEIGHT + 2;
        }

        let x: i32 = {
            let v = spawn_opts
                .x
                .map(|c| {
                    spawn_opts
                        .anchor
                        .resolve_x(c.to_pixels(monitor_width), outer_width)
                })
                .unwrap_or_else(|| random_position(outer_width, monitor_width));
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
                .unwrap_or_else(|| random_position(outer_height, monitor_height));
            if spawn_opts.clamp {
                v.max(0)
                    .min(monitor_height.saturating_sub(outer_height) as i32)
            } else {
                v
            }
        };

        Self {
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
            background_color: spawn_opts.background_color,
        }
    }
}

/// Pick the monitor a popup should spawn on: the one the mode explicitly asked for (from an
/// earlier `lewdware.monitors.list()`/`primary()` call), or a random one from a fresh
/// `list_monitors()` snapshot. Either way, only `.id` is trusted past this point -- see
/// `PopupSpawnOpts`'s doc comment.
fn resolve_monitor(
    spawn_opts: &SpawnWindowOpts,
    request_sender: &RequestSender,
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

#[derive(Serialize, Deserialize, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    #[serde(rename = "left")]
    Left,
    #[serde(rename = "center")]
    #[default]
    Center,
    #[serde(rename = "right")]
    Right,
}

fn default_text_color() -> Color {
    Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}

fn default_font_size() -> FontSize {
    FontSize::Value(32.0)
}

fn default_outline_width() -> f32 {
    2.0
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct TextStyle {
    #[serde(default)]
    pub font: TextFont,
    #[serde(default = "default_font_size")]
    pub font_size: FontSize,
    #[serde(default = "default_text_color")]
    pub color: Color,
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
            font_size: default_font_size(),
            color: default_text_color(),
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

fn spawn_text_popup(
    _: &Lua,
    (text, opts): (String, Option<SpawnTextOpts>),
    request_sender: RequestSender,
    windows: Windows,
    gpu_available: bool,
    dev_mode: bool,
) -> mlua::Result<Rc<TextWindow>> {
    let mut opts = opts.unwrap_or_default();

    let monitor = resolve_monitor(&opts.window_opts, &request_sender)?;

    // Unlike other popup types, text defaults to a transparent (GPU-rendered) window, since
    // text is usually meant to float over the desktop rather than sit in an opaque panel.
    let transparent = opts.window_opts.transparent.unwrap_or(true);

    // Resolve a percentage font size to a concrete point size now that the monitor (and so its
    // height, the basis for `FontSize::Percent`) is known. From here on `font_size` is always
    // `FontSize::Value`.
    opts.style.font_size = FontSize::Value(opts.style.font_size.to_pixels(monitor.height));
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
            font_size: opts.style.font_size,
            outline_width,
        },
        &monitor,
        gpu_available,
        transparent,
        transparent,
    );

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

fn spawn_image_popup(
    _: &Lua,
    (image, opts): (Media, Option<SpawnImageOpts>),
    request_sender: RequestSender,
    windows: Windows,
    gpu_available: bool,
    dev_mode: bool,
) -> mlua::Result<Rc<ImageWindow>> {
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
        gpu_available,
        transparent,
        transparent,
    );

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

fn spawn_video_popup(
    _: &Lua,
    (video, opts): (Media, Option<SpawnVideoOpts>),
    request_sender: RequestSender,
    windows: Windows,
    gpu_available: bool,
    dev_mode: bool,
) -> mlua::Result<Rc<VideoWindow>> {
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
        gpu_available,
        true,
        transparent,
    );

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
pub enum DialogElement {
    #[serde(rename = "text")]
    Text {
        id: Option<String>,
        text: String,
        #[serde(flatten)]
        style: TextStyle,
    },
    #[serde(rename = "image")]
    Image { id: Option<String>, image: Media },
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

impl FromLua for DialogElement {
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

fn spawn_dialog(
    _: &Lua,
    opts: SpawnDialogOpts,
    request_sender: RequestSender,
    windows: Windows,
    gpu_available: bool,
    dev_mode: bool,
) -> mlua::Result<Rc<DialogWindow>> {
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
        gpu_available,
        transparent,
        transparent,
    );

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

#[derive(Serialize, Deserialize, Debug)]
pub enum WallpaperMode {
    #[serde(rename = "center")]
    Center,
    #[serde(rename = "crop")]
    Crop,
    #[serde(rename = "fit")]
    Fit,
    #[serde(rename = "span")]
    Span,
    #[serde(rename = "stretch")]
    Stretch,
    #[serde(rename = "tile")]
    Tile,
}

#[derive(Serialize, Deserialize, Default)]
struct SetWallpaperOpts {
    mode: Option<WallpaperMode>,
}

impl FromLua for SetWallpaperOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

fn set_wallpaper(
    _: &Lua,
    (image, opts): (Media, Option<SetWallpaperOpts>),
    media_manager: MediaManager,
    request_sender: RequestSender,
) -> mlua::Result<()> {
    let opts = opts.unwrap_or_default();

    if !matches!(image.media_data, MediaData::Image { .. }) {
        return Err("`image` is not an image".into_lua_err());
    }

    let file = media_manager.get_image_file(image.id).into_lua_err()?;

    request_sender.set_wallpaper(file, opts.mode).into_lua_err()
}

fn reset_wallpaper(_: &Lua, _: (), request_sender: RequestSender) -> mlua::Result<()> {
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

fn play_audio(
    _: &Lua,
    (audio, opts): (Media, Option<PlayAudioOpts>),
    request_sender: RequestSender,
    audio_handles: AudioHandles,
    dev_mode: bool,
) -> mlua::Result<Rc<AudioHandle>> {
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
        audio_handles.clone(),
        dev_mode,
    ));

    audio_handles
        .try_borrow_mut()
        .into_lua_err()?
        .insert(id, audio_handle.clone());

    Ok(audio_handle)
}

fn open_link(_: &Lua, url: String, request_sender: RequestSender) -> mlua::Result<()> {
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

fn show_notification(
    _: &Lua,
    notification: Notification,
    request_sender: RequestSender,
) -> mlua::Result<()> {
    request_sender
        .show_notification(notification)
        .into_lua_err()
}

fn list_monitors(_: &Lua, _: (), request_sender: RequestSender) -> mlua::Result<Vec<Monitor>> {
    request_sender.list_monitors().into_lua_err()
}

fn primary_monitor(_: &Lua, _: (), request_sender: RequestSender) -> mlua::Result<Monitor> {
    request_sender.primary_monitor().into_lua_err()
}

fn exit(_: &Lua, _: (), request_sender: RequestSender) -> mlua::Result<()> {
    request_sender.exit().into_lua_err()
}

fn after(
    _: &Lua,
    (ms, function): (u64, mlua::Function),
    dev_mode: bool,
) -> mlua::Result<Timer> {
    Ok(Timer::new(Duration::from_millis(ms), function, dev_mode))
}

fn every(
    _: &Lua,
    (ms, function): (u64, mlua::Function),
    dev_mode: bool,
) -> mlua::Result<Interval> {
    Ok(Interval::new(Duration::from_millis(ms), function, dev_mode))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(Anchor::TopCenter.resolve_x(coord, size), Anchor::Center.resolve_x(coord, size));
        assert_eq!(Anchor::TopCenter.resolve_y(coord, size), Anchor::TopLeft.resolve_y(coord, size));
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
