//! Spawning the three plain popup kinds -- text, image and video -- and the text styling
//! only the first of them needs.

use std::rc::Rc;

use mlua::{ExternalError, ExternalResult, FromLua, Lua, LuaSerdeExt};
use serde::{Deserialize, Serialize};

// The Lua layer's own names for these: they describe a colour and an alignment, and a *theme*
// needs to as well, so they live with the themes and are re-exported here.
pub use shared::theme::{Color, TextAlign};

use super::*;
use crate::{
    app::EventPoster,
    lua::{
        Media, MediaData, Window, Windows,
        request::RequestSender,
        window::{ImageWindow, TextWindow, VideoWindow},
    },
    window::ChromeDefaults,
};

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

pub(super) fn default_outline_width() -> f32 {
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
pub(super) struct SpawnTextOpts {
    #[serde(flatten)]
    pub(super) style: TextStyle,
    #[serde(flatten)]
    pub(super) window_opts: SpawnWindowOpts,
}

impl FromLua for SpawnTextOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub(super) fn spawn_text_popup<T: EventPoster>(
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

pub(super) fn spawn_image_popup<T: EventPoster>(
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

pub(super) fn return_true() -> bool {
    true
}

pub(super) fn return_one() -> f32 {
    1.0
}

impl FromLua for SpawnVideoOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub(super) fn spawn_video_popup<T: EventPoster>(
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
