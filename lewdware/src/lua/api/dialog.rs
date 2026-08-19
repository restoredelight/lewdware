//! Dialog popups: the elements an author can put in one, and spawning it.

use std::rc::Rc;

use mlua::{ExternalResult, FromLua, Lua, LuaSerdeExt};
use serde::{Deserialize, Serialize};

// The Lua layer's own names for these: they describe a colour and an alignment, and a *theme*
// needs to as well, so they live with the themes and are re-exported here.
pub use shared::theme::{Color, TextAlign};

use super::*;
use crate::{
    app::EventPoster,
    lua::{Media, MediaData, Window, Windows, request::RequestSender, window::DialogWindow},
    window::ChromeDefaults,
};

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

pub(super) fn dialog_has_more_than_one_default_button(elements: &[DialogElement]) -> bool {
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
pub(super) struct SpawnDialogOpts {
    pub(super) elements: Vec<DialogElement>,
    #[serde(flatten)]
    pub(super) window_opts: SpawnWindowOpts,
}

impl FromLua for SpawnDialogOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub(super) fn spawn_dialog<T: EventPoster>(
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
