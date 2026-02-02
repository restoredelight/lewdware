use std::{rc::Rc, time::Duration};

use mlua::{ExternalError, ExternalResult, FromLua, Lua, LuaSerdeExt};
use serde::{Deserialize, Serialize};
use winit::dpi::LogicalSize;

use crate::{
    lua::{
        AudioHandles, Media, MediaData, MediaType, Window, Windows,
        audio::AudioHandle,
        interval::{Interval, Timer},
        request::RequestSender,
        window::{ChoiceWindow, ChoiceWindowOption, ImageWindow, PromptWindow, VideoWindow},
    },
    media::{MediaManager, MediaTypes},
    monitor::Monitor,
    utils::calculate_media_popup_size,
};

pub fn create_api(
    lua: &Lua,
    request_sender: RequestSender,
    media_manager: MediaManager,
    windows: Windows,
    audio_handles: AudioHandles,
) -> mlua::Result<()> {
    let api_table = lua.create_table()?;

    let media_table = lua.create_table()?;

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get",
            lua.create_async_function(move |lua, name| {
                get_media(lua, name, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_image",
            lua.create_async_function(move |lua, name| {
                get_image(lua, name, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_video",
            lua.create_async_function(move |lua, name| {
                get_video(lua, name, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_audio",
            lua.create_async_function(move |lua, name| {
                get_audio(lua, name, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list",
            lua.create_async_function(move |lua, opts| {
                list_media(lua, opts, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_images",
            lua.create_async_function(move |lua, opts| {
                list_images(lua, opts, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_videos",
            lua.create_async_function(move |lua, opts| {
                list_videos(lua, opts, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_audio",
            lua.create_async_function(move |lua, opts| {
                list_audio(lua, opts, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random",
            lua.create_async_function(move |lua, opts| {
                random_media(lua, opts, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_image",
            lua.create_async_function(move |lua, opts| {
                random_image(lua, opts, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_video",
            lua.create_async_function(move |lua, opts| {
                random_video(lua, opts, media_manager.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_audio",
            lua.create_async_function(move |lua, opts| {
                random_audio(lua, opts, media_manager.clone())
            })?,
        )?;
    }

    api_table.set("media", media_table)?;

    {
        let media_manager = media_manager.clone();
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        api_table.set(
            "spawn_image_popup",
            lua.create_async_function(move |lua, args| {
                spawn_image_popup(
                    lua,
                    args,
                    media_manager.clone(),
                    request_sender.clone(),
                    windows.clone(),
                )
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        api_table.set(
            "spawn_video_popup",
            lua.create_async_function(move |lua, args| {
                spawn_video_popup(
                    lua,
                    args,
                    media_manager.clone(),
                    request_sender.clone(),
                    windows.clone(),
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        api_table.set(
            "spawn_prompt",
            lua.create_async_function(move |lua, args| {
                spawn_prompt(lua, args, request_sender.clone(), windows.clone())
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        api_table.set(
            "spawn_choice",
            lua.create_async_function(move |lua, args| {
                spawn_choice(lua, args, request_sender.clone(), windows.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();
        let request_sender = request_sender.clone();

        api_table.set(
            "set_wallpaper",
            lua.create_async_function(move |lua, args| {
                set_wallpaper(lua, args, media_manager.clone(), request_sender.clone())
            })?,
        )?;
    }

    {
        let media_manager = media_manager.clone();
        let request_sender = request_sender.clone();
        let audio_handles = audio_handles.clone();

        api_table.set(
            "play_audio",
            lua.create_async_function(move |lua, args| {
                play_audio(
                    lua,
                    args,
                    media_manager.clone(),
                    request_sender.clone(),
                    audio_handles.clone(),
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "open_link",
            lua.create_async_function(move |lua, url| open_link(lua, url, request_sender.clone()))?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "show_notification",
            lua.create_async_function(move |lua, notification| {
                show_notification(lua, notification, request_sender.clone())
            })?,
        )?;
    }

    let monitors_table = lua.create_table()?;

    {
        let request_sender = request_sender.clone();

        monitors_table.set(
            "list",
            lua.create_async_function(move |lua, args| {
                list_monitors(lua, args, request_sender.clone())
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        monitors_table.set(
            "primary",
            lua.create_async_function(move |lua, args| {
                primary_monitor(lua, args, request_sender.clone())
            })?,
        )?;
    }

    api_table.set("monitors", monitors_table)?;

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "exit",
            lua.create_async_function(move |lua, x| exit(lua, x, request_sender.clone()))?,
        )?;
    }

    api_table.set("after", lua.create_function(after)?)?;

    api_table.set("every", lua.create_function(every)?)?;

    lua.globals().set("lewdware", api_table)?;

    Ok(())
}

async fn get_media_type(
    name: String,
    types: MediaTypes,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    media_manager
        .get_media(name, types)
        .await
        .map_err(|err| err.into_lua_err())
}

async fn get_media(
    _: Lua,
    name: String,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::ALL, media_manager).await
}

async fn get_image(
    _: Lua,
    name: String,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::IMAGE, media_manager).await
}

async fn get_video(
    _: Lua,
    name: String,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::VIDEO, media_manager).await
}

async fn get_audio(
    _: Lua,
    name: String,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    get_media_type(name, MediaTypes::AUDIO, media_manager).await
}

async fn list_media_type(
    types: MediaTypes,
    tags: Option<Vec<String>>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    media_manager
        .list_media(types, tags)
        .await
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

#[derive(Serialize, Deserialize)]
struct QueryMediaOpts {
    #[serde(rename = "type")]
    types: Option<OneOrMore<MediaType>>,
    tags: Option<Vec<String>>,
}

impl FromLua for QueryMediaOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

async fn list_media(
    _: Lua,
    opts: Option<QueryMediaOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let (types, tags) = match opts {
        Some(QueryMediaOpts { types, tags }) => {
            (types.map_or(MediaTypes::ALL, |t| MediaTypes::from(t)), tags)
        }
        None => (MediaTypes::ALL, None),
    };

    list_media_type(types, tags, media_manager).await
}

#[derive(Serialize, Deserialize, Default)]
struct QueryMediaTypeOpts {
    tags: Option<Vec<String>>,
}

impl FromLua for QueryMediaTypeOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

async fn list_images(
    _: Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.map_or(None, |x| x.tags);

    list_media_type(MediaTypes::IMAGE, tags, media_manager).await
}

async fn list_videos(
    _: Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.map_or(None, |x| x.tags);

    list_media_type(MediaTypes::VIDEO, tags, media_manager).await
}

async fn list_audio(
    _: Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Vec<Media>> {
    let tags = opts.map_or(None, |x| x.tags);

    list_media_type(MediaTypes::AUDIO, tags, media_manager).await
}

async fn random_media_type(
    _: Lua,
    types: MediaTypes,
    tags: Option<Vec<String>>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    media_manager
        .random_media(types, tags)
        .await
        .map_err(|err| err.into_lua_err())
}

async fn random_media(
    lua: Lua,
    opts: Option<QueryMediaOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let (types, tags) = match opts {
        Some(QueryMediaOpts { types, tags }) => {
            (types.map_or(MediaTypes::ALL, |t| MediaTypes::from(t)), tags)
        }
        None => (MediaTypes::ALL, None),
    };

    random_media_type(lua, types, tags, media_manager).await
}

async fn random_image(
    lua: Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let tags = opts.map_or(None, |x| x.tags);

    random_media_type(lua, MediaTypes::IMAGE, tags, media_manager).await
}

async fn random_video(
    lua: Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let tags = opts.map_or(None, |x| x.tags);

    random_media_type(lua, MediaTypes::VIDEO, tags, media_manager).await
}

async fn random_audio(
    lua: Lua,
    opts: Option<QueryMediaTypeOpts>,
    media_manager: MediaManager,
) -> mlua::Result<Option<Media>> {
    let tags = opts.map_or(None, |x| x.tags);

    random_media_type(lua, MediaTypes::AUDIO, tags, media_manager).await
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Coord {
    Pixel(u32),
    Percent { percent: f64 },
}

impl Coord {
    pub fn to_pixels(&self, total_size: u32) -> u32 {
        match self {
            Coord::Pixel(x) => *x,
            Coord::Percent { percent } => ((percent * total_size as f64) / 100.0).round() as u32,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub enum Anchor {
    #[serde(rename = "top-left")]
    #[default]
    TopLeft,
    #[serde(rename = "center")]
    Center,
    #[serde(rename = "bottom-right")]
    BottomRight,
}

impl Anchor {
    pub fn resolve(&self, coord: u32, size: u32) -> u32 {
        match self {
            Self::TopLeft => coord,
            Self::Center => coord - (size / 2),
            Self::BottomRight => coord - size,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
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
}

impl Default for SpawnWindowOpts {
    fn default() -> Self {
        Self {
            x: Default::default(),
            y: Default::default(),
            width: Default::default(),
            height: Default::default(),
            anchor: Default::default(),
            monitor: Default::default(),
            decorations: true,
        }
    }
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

async fn spawn_image_popup(
    _: Lua,
    (image, opts): (Media, Option<SpawnImageOpts>),
    media_manager: MediaManager,
    request_sender: RequestSender,
    windows: Windows,
) -> mlua::Result<Rc<ImageWindow>> {
    let mut opts = opts.unwrap_or_default();

    let (image_width, image_height) = match image.media_data {
        MediaData::Image { width, height } => (width, height),
        _ => return Err("`image` is not an image".into_lua_err()),
    };

    let monitor = match &opts.window_opts.monitor {
        Some(monitor) => request_sender
            .get_monitor(monitor.id)
            .await
            .into_lua_err()?,
        None => request_sender.random_monitor().await.into_lua_err()?,
    };

    let (width, height) = calculate_media_popup_size(
        opts.window_opts.width.clone(),
        opts.window_opts.height.clone(),
        image_width,
        image_height,
        monitor.width,
        monitor.height,
    );
    let physical_size = LogicalSize::new(width, height).to_physical(monitor.scale_factor);

    let data = media_manager
        .get_image_data(image.id, physical_size.width, physical_size.height)
        .await
        .into_lua_err()?;

    opts.window_opts.monitor = Some(monitor);
    opts.window_opts.width = Some(Coord::Pixel(width));
    opts.window_opts.height = Some(Coord::Pixel(height));

    let props = request_sender.spawn_image(data, opts.window_opts).await?;

    let id = props.window_id;

    let window = Rc::new(ImageWindow::new(
        props,
        image,
        request_sender.window_sender(id),
    ));

    windows
        .borrow_mut()
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
    #[serde(flatten)]
    window_opts: SpawnWindowOpts,
}

impl Default for SpawnVideoOpts {
    fn default() -> Self {
        Self {
            loop_video: true,
            audio: true,
            window_opts: Default::default(),
        }
    }
}

fn return_true() -> bool {
    true
}

impl FromLua for SpawnVideoOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

async fn spawn_video_popup(
    _: Lua,
    (video, opts): (Media, Option<SpawnVideoOpts>),
    media_manager: MediaManager,
    request_sender: RequestSender,
    windows: Windows,
) -> mlua::Result<Rc<VideoWindow>> {
    let opts = opts.unwrap_or_default();

    if !matches!(video.media_data, MediaData::Video { .. }) {
        return Err("`video` is not a video".into_lua_err());
    }

    let data = media_manager
        .get_video_data(video.id)
        .await
        .into_lua_err()?;

    let props = request_sender
        .spawn_video(data, opts.loop_video, opts.audio, opts.window_opts)
        .await?;

    let id = props.window_id;

    let window = Rc::new(VideoWindow::new(
        props,
        video,
        request_sender.window_sender(id),
    ));

    windows
        .borrow_mut()
        .insert(id, Window::Video(window.clone()));

    Ok(window)
}

#[derive(Serialize, Deserialize, Default)]
struct SpawnPromptOpts {
    title: Option<String>,
    text: Option<String>,
    placeholder: Option<String>,
    initial_value: Option<String>,
    #[serde(flatten)]
    window_opts: SpawnWindowOpts,
}

impl FromLua for SpawnPromptOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

async fn spawn_prompt(
    _: Lua,
    opts: Option<SpawnPromptOpts>,
    request_sender: RequestSender,
    windows: Windows,
) -> mlua::Result<Rc<PromptWindow>> {
    let opts = opts.unwrap_or_default();

    let props = request_sender
        .spawn_prompt(
            opts.title.clone(),
            opts.text.clone(),
            opts.placeholder,
            opts.initial_value.clone(),
            opts.window_opts,
        )
        .await?;

    let id = props.window_id;

    let window = Rc::new(PromptWindow::new(
        props,
        opts.title,
        opts.text,
        opts.initial_value.unwrap_or_default(),
        request_sender.window_sender(id),
    ));

    windows
        .borrow_mut()
        .insert(id, Window::Prompt(window.clone()));

    Ok(window)
}

#[derive(Serialize, Deserialize, Default)]
struct SpawnChoiceOpts {
    title: Option<String>,
    text: Option<String>,
    #[serde(default)]
    options: Vec<ChoiceWindowOption>,
    #[serde(flatten)]
    window_opts: SpawnWindowOpts,
}

impl FromLua for SpawnChoiceOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

async fn spawn_choice(
    _: Lua,
    opts: Option<SpawnChoiceOpts>,
    request_sender: RequestSender,
    windows: Windows,
) -> mlua::Result<Rc<ChoiceWindow>> {
    let opts = opts.unwrap_or_default();

    let props = request_sender
        .spawn_choice(
            opts.title.clone(),
            opts.text.clone(),
            opts.options.clone(),
            opts.window_opts,
        )
        .await?;

    let id = props.window_id;

    let window = Rc::new(ChoiceWindow::new(
        props,
        opts.title,
        opts.text,
        opts.options.clone(),
        request_sender.window_sender(id),
    ));

    windows
        .borrow_mut()
        .insert(id, Window::Choice(window.clone()));

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

async fn set_wallpaper(
    _: Lua,
    (image, opts): (Media, Option<SetWallpaperOpts>),
    media_manager: MediaManager,
    request_sender: RequestSender,
) -> mlua::Result<()> {
    let opts = opts.unwrap_or_default();

    if !matches!(image.media_data, MediaData::Image { .. }) {
        return Err("`image` is not an image".into_lua_err());
    }

    let file = media_manager
        .get_image_file(image.id)
        .await
        .into_lua_err()?;

    request_sender
        .set_wallpaper(file, opts.mode)
        .await
        .into_lua_err()
}

#[derive(Serialize, Deserialize, Default)]
struct PlayAudioOpts {
    #[serde(default)]
    loop_audio: bool,
}

impl FromLua for PlayAudioOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

async fn play_audio(
    _: Lua,
    (audio, opts): (Media, Option<PlayAudioOpts>),
    media_manager: MediaManager,
    request_sender: RequestSender,
    audio_handles: AudioHandles,
) -> mlua::Result<Rc<AudioHandle>> {
    let opts = opts.unwrap_or_default();

    if !matches!(audio.media_data, MediaData::Audio { .. }) {
        return Err("`audio` is not a audio".into_lua_err());
    }

    let data = media_manager
        .get_audio_data(audio.id)
        .await
        .into_lua_err()?;

    let id = request_sender.spawn_audio(data, opts.loop_audio).await?;

    let audio_handle = Rc::new(AudioHandle::new(
        id,
        audio,
        request_sender.audio_sender(id),
        audio_handles.clone(),
    ));

    audio_handles.borrow_mut().insert(id, audio_handle.clone());

    Ok(audio_handle)
}

async fn open_link(_: Lua, url: String, request_sender: RequestSender) -> mlua::Result<()> {
    request_sender.open_link(url).await.into_lua_err()
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

async fn show_notification(
    _: Lua,
    notification: Notification,
    request_sender: RequestSender,
) -> mlua::Result<()> {
    request_sender
        .show_notification(notification)
        .await
        .into_lua_err()
}

async fn list_monitors(_: Lua, _: (), request_sender: RequestSender) -> mlua::Result<Vec<Monitor>> {
    request_sender.list_monitors().await.into_lua_err()
}

async fn primary_monitor(_: Lua, _: (), request_sender: RequestSender) -> mlua::Result<Monitor> {
    request_sender.primary_monitor().await.into_lua_err()
}

async fn exit(_: Lua, _: (), request_sender: RequestSender) -> mlua::Result<()> {
    request_sender.exit().await.into_lua_err()
}

// fn on_spawn(
//     _: &Lua,
//     cb: mlua::Function,
//     callbacks: Rc<RefCell<Vec<mlua::Function>>>,
// ) -> mlua::Result<()> {
//     callbacks.borrow_mut().push(cb);
//
//     Ok(())
// }
//
// async fn spawn_image(
//     _: Lua,
//     name: String,
//     request_tx: RequestSender,
//     windows: Rc<RefCell<HashMap<WindowId, Window>>>,
// ) -> mlua::Result<Window> {
//     let (tx, rx) = oneshot::channel();
//
//     request_tx
//         .send(LuaRequest::SpawnImage { tx, name })
//         .await
//         .map_err(|err| err.into_lua_err())?;
//
//     let props = rx.await.map_err(|err| err.into_lua_err())?;
//
//     let window_id = props.window_id.clone();
//
//     let mut windows = windows.borrow_mut();
//     let window = windows
//         .entry(window_id)
//         .or_insert_with(|| Window::Image(Rc::new(ImageWindow::new(props, request_tx))));
//
//     Ok(window.clone())
// }
//
//
// async fn spawn_video(
//     _: Lua,
//     name: String,
//     request_tx: RequestSender,
//     windows: Rc<RefCell<HashMap<WindowId, Window>>>,
// ) -> mlua::Result<Window> {
//     let (tx, rx) = oneshot::channel();
//
//     request_tx
//         .send(LuaRequest::SpawnVideo { tx, name })
//         .await
//         .map_err(|err| err.into_lua_err())?;
//
//     let props = rx.await.map_err(|err| err.into_lua_err())?;
//
//     let window_id = props.window_id.clone();
//
//     let mut windows = windows.borrow_mut();
//     let window = windows
//         .entry(window_id)
//         .or_insert_with(|| Window::Video(Rc::new(VideoWindow::new(props, request_tx))));
//
//     Ok(window.clone())
// }
//
// async fn spawn_popup(
//     lua: Lua,
//     opts: mlua::Value,
//     request_tx: RequestSender,
//     windows: Rc<RefCell<HashMap<WindowId, Window>>>,
// ) -> mlua::Result<Window> {
//     let opts: RandomPopupOpts = lua.from_value(opts)?;
//     let (tx, rx) = oneshot::channel();
//
//     request_tx
//         .send(LuaRequest::SpawnRandomPopup {
//             tx,
//             popup_type: opts.popup_type,
//             tags: opts.tags,
//         })
//         .await
//         .map_err(|err| err.into_lua_err())?;
//
//     let (popup_type, props) = rx.await.map_err(|err| err.into_lua_err())?;
//
//     let window_id = props.window_id.clone();
//
//     let mut windows = windows.borrow_mut();
//     let window = windows
//         .entry(window_id)
//         .or_insert_with(|| match popup_type {
//             PopupResultType::Image => Window::Image(Rc::new(ImageWindow::new(props, request_tx))),
//             PopupResultType::Video => Window::Video(Rc::new(VideoWindow::new(props, request_tx))),
//         });
//
//     Ok(window.clone())
// }
//
// async fn spawn_prompt(
//     _: Lua,
//     text: String,
//     request_tx: RequestSender,
//     windows: Rc<RefCell<HashMap<WindowId, Window>>>,
// ) -> mlua::Result<Window> {
//     let (tx, rx) = oneshot::channel();
//
//     request_tx
//         .send(LuaRequest::SpawnPrompt {
//             tx,
//             text: text.clone(),
//         })
//         .await
//         .map_err(|err| err.into_lua_err())?;
//
//     let props = rx.await.map_err(|err| err.into_lua_err())?;
//
//     let window_id = props.window_id.clone();
//
//     let mut windows = windows.borrow_mut();
//     let window = windows
//         .entry(window_id)
//         .or_insert_with(|| Window::Prompt(Rc::new(PromptWindow::new(props, text, request_tx))));
//
//     Ok(window.clone())
// }

// async fn open_link(_: Lua, url: String, request_tx: RequestSender) -> mlua::Result<()> {
//     request_tx
//         .send(LuaRequest::OpenLink { url })
//         .await
//         .map_err(|err| err.into_lua_err())
// }

fn after(_: &Lua, (ms, function): (u64, mlua::Function)) -> mlua::Result<Timer> {
    Ok(Timer::new(Duration::from_millis(ms), function))
}

fn every(_: &Lua, (ms, function): (u64, mlua::Function)) -> mlua::Result<Interval> {
    Ok(Interval::new(Duration::from_millis(ms), function))
}

// async fn sleep(_: Lua, ms: u64) -> mlua::Result<()> {
//     tokio::time::sleep(Duration::from_millis(ms)).await;
//
//     Ok(())
// }
