use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mlua::{
    ExternalError, ExternalResult, FromLua, LuaSerdeExt, UserData, UserDataFields, UserDataMethods,
};
use serde::{Deserialize, Serialize};

use crate::{
    error::LewdwareError,
    lua::{
        DialogElementUpdate, Media, PopupId, WindowProps,
        api::{Anchor, Coord},
        request::WindowRequestSender,
    },
    monitor::Monitor,
};

#[derive(Clone)]
pub enum Window {
    Image(Rc<ImageWindow>),
    Video(Rc<VideoWindow>),
    Dialog(Rc<DialogWindow>),
    Text(Rc<TextWindow>),
}

impl Window {
    pub fn inner_window(&self) -> &InnerWindow {
        match self {
            Window::Image(image) => &image.inner_window,
            Window::Video(video) => &video.inner_window,
            Window::Dialog(dialog) => &dialog.inner_window,
            Window::Text(text) => &text.inner_window,
        }
    }
}

pub struct ImageWindow {
    inner_window: InnerWindow,
    image: Media,
}

impl UserData for ImageWindow {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "image");
        fields.add_field_method_get("image", |_, this| Ok(this.image.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);
    }
}

impl ImageWindow {
    pub fn new(props: WindowProps, image: Media, request_sender: WindowRequestSender) -> Self {
        ImageWindow {
            inner_window: InnerWindow::new(props, request_sender),
            image,
        }
    }
}

pub struct VideoWindow {
    inner_window: InnerWindow,
    video: Media,
}

impl UserData for VideoWindow {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "video");
        fields.add_field_method_get("video", |_, this| Ok(this.video.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);

        methods.add_method("pause", |_, this, _: ()| {
            this.inner_window
                .request_sender
                .pause_video()
                .into_lua_err()?;

            Ok(())
        });

        methods.add_method("play", |_, this, _: ()| {
            this.inner_window
                .request_sender
                .play_video()
                .into_lua_err()?;

            Ok(())
        });

        methods.add_method("set_volume", |_, this, volume: f32| {
            this.inner_window
                .request_sender
                .set_video_volume(volume)
                .into_lua_err()?;

            Ok(())
        });
    }
}

impl VideoWindow {
    pub fn new(props: WindowProps, video: Media, request_tx: WindowRequestSender) -> Self {
        VideoWindow {
            inner_window: InnerWindow::new(props, request_tx),
            video,
        }
    }
}

pub struct DialogWindow {
    inner_window: InnerWindow,
    state: RefCell<DialogWindowState>,
}

struct DialogWindowState {
    click_callbacks: Vec<mlua::Function>,
    submit_callbacks: Vec<mlua::Function>,
}

impl DialogWindowState {
    fn new() -> Self {
        Self {
            click_callbacks: Vec::new(),
            submit_callbacks: Vec::new(),
        }
    }
}

impl UserData for DialogWindow {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "dialog");
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);

        methods.add_method("on_click", |_, this, cb: mlua::Function| {
            this.state
                .try_borrow_mut()
                .into_lua_err()?
                .click_callbacks
                .push(cb);

            Ok(())
        });

        methods.add_method("on_submit", |_, this, cb: mlua::Function| {
            this.state
                .try_borrow_mut()
                .into_lua_err()?
                .submit_callbacks
                .push(cb);

            Ok(())
        });

        // Nil if the window is closed -- documented as a deliberate exception to the usual
        // no-op-returns-false convention, since "no values" (an empty table) is itself a
        // meaningful reply for a still-open dialog with no input elements.
        methods.add_method("values", |_, this, _: ()| {
            Ok(this.inner_window.request_sender.get_dialog_values()?)
        });

        // Nil for a closed window OR an id that isn't a live input element -- both just mean
        // "no value to report" from the caller's perspective.
        methods.add_method("value", |_, this, id: String| {
            Ok(this.inner_window.request_sender.get_dialog_value(id)?)
        });

        methods.add_method(
            "update",
            |_, this, (id, props): (String, DialogElementUpdate)| {
                Ok(this
                    .inner_window
                    .request_sender
                    .update_dialog_element(id, props)?)
            },
        );
    }
}

impl DialogWindow {
    pub fn new(props: WindowProps, request_sender: WindowRequestSender) -> Self {
        Self {
            inner_window: InnerWindow::new(props, request_sender),
            state: RefCell::new(DialogWindowState::new()),
        }
    }

    pub fn on_click(&self, button_id: String, values: HashMap<String, String>) -> anyhow::Result<()> {
        let callbacks = {
            let state = self.state.try_borrow()?;
            state.click_callbacks.clone()
        };

        for cb in callbacks {
            if let Err(err) = cb.call::<()>((button_id.clone(), values.clone())) {
                tracing::error!("{err}");
            }
        }

        Ok(())
    }

    pub fn on_submit(&self, element_id: String, values: HashMap<String, String>) -> anyhow::Result<()> {
        let callbacks = {
            let state = self.state.try_borrow()?;
            state.submit_callbacks.clone()
        };

        for cb in callbacks {
            if let Err(err) = cb.call::<()>((element_id.clone(), values.clone())) {
                tracing::error!("{err}");
            }
        }

        Ok(())
    }
}

pub struct TextWindow {
    inner_window: InnerWindow,
    state: RefCell<TextWindowState>,
}

struct TextWindowState {
    text: String,
}

impl UserData for TextWindow {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "text");

        fields.add_field_method_get("text", |_, this| {
            Ok(this.state.try_borrow().into_lua_err()?.text.clone())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);

        methods.add_method("set_text", |_, this, text: String| {
            this.inner_window
                .request_sender
                .set_text(Some(text.clone()))?;

            this.state.try_borrow_mut().into_lua_err()?.text = text;

            Ok(())
        });
    }
}

impl TextWindow {
    pub fn new(props: WindowProps, text: String, request_sender: WindowRequestSender) -> Self {
        Self {
            inner_window: InnerWindow::new(props, request_sender),
            state: RefCell::new(TextWindowState { text }),
        }
    }
}

pub struct InnerWindow {
    id: PopupId,
    width: u32,
    height: u32,
    outer_width: u32,
    outer_height: u32,
    state: RefCell<InnerWindowState>,
    monitor: Monitor,
    request_sender: WindowRequestSender,
}

struct InnerWindowState {
    x: i32,
    y: i32,
    closed: bool,
    close_callbacks: Vec<mlua::Function>,
    spawned: bool,
    spawn_callbacks: Vec<mlua::Function>,
    move_callback: Option<(u64, mlua::Function)>,
    current_move_id: u64,
    fade_callback: Option<(u64, mlua::Function)>,
    current_fade_id: u64,
}

trait HasInnerWindow {
    fn inner_window(&self) -> &InnerWindow;
}

impl HasInnerWindow for ImageWindow {
    fn inner_window(&self) -> &InnerWindow {
        &self.inner_window
    }
}

impl HasInnerWindow for VideoWindow {
    fn inner_window(&self) -> &InnerWindow {
        &self.inner_window
    }
}

impl HasInnerWindow for DialogWindow {
    fn inner_window(&self) -> &InnerWindow {
        &self.inner_window
    }
}

impl HasInnerWindow for TextWindow {
    fn inner_window(&self) -> &InnerWindow {
        &self.inner_window
    }
}

impl InnerWindow {
    pub fn new(props: WindowProps, request_tx: WindowRequestSender) -> Self {
        Self {
            id: props.window_id,
            width: props.width,
            height: props.height,
            outer_width: props.outer_width,
            outer_height: props.outer_height,
            state: RefCell::new(InnerWindowState::new(props.x, props.y)),
            monitor: props.monitor,
            request_sender: request_tx,
        }
    }

    fn add_fields<T: HasInnerWindow, F: UserDataFields<T>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(u64::from(this.inner_window().id)));
        fields.add_field_method_get("width", |_, this| Ok(this.inner_window().width));
        fields.add_field_method_get("height", |_, this| Ok(this.inner_window().height));
        fields.add_field_method_get("outer_width", |_, this| Ok(this.inner_window().outer_width));
        fields.add_field_method_get("outer_height", |_, this| {
            Ok(this.inner_window().outer_height)
        });
        fields.add_field_method_get("x", |_, this| {
            Ok(this.inner_window().state.try_borrow().into_lua_err()?.x)
        });
        fields.add_field_method_get("y", |_, this| {
            Ok(this.inner_window().state.try_borrow().into_lua_err()?.y)
        });
        fields.add_field_method_get("monitor", |_, this| Ok(this.inner_window().monitor.clone()));
        fields.add_field_method_get("closed", |_, this| {
            Ok(this
                .inner_window()
                .state
                .try_borrow()
                .into_lua_err()?
                .closed)
        });
        fields.add_field_method_get("spawned", |_, this| {
            Ok(this
                .inner_window()
                .state
                .try_borrow()
                .into_lua_err()?
                .spawned)
        });
    }

    fn add_methods<T: HasInnerWindow + 'static, M: UserDataMethods<T>>(methods: &mut M) {
        methods.add_method("close", |_, this, _: ()| {
            let inner_window = this.inner_window();

            match inner_window.request_sender.close() {
                Ok(()) | Err(LewdwareError::WindowNotFound) => {}
                Err(err) => return Err(err.into_lua_err()),
            };

            Ok(())
        });

        methods.add_method("on_close", |_, this, cb: mlua::Function| {
            this.inner_window()
                .state
                .try_borrow_mut()
                .into_lua_err()?
                .close_callbacks
                .push(cb);

            Ok(())
        });

        methods.add_method("on_spawn", |_, this, cb: mlua::Function| {
            let spawned = this
                .inner_window()
                .state
                .try_borrow()
                .into_lua_err()?
                .spawned;

            if spawned {
                // Already spawned: still fire `cb`, but queued rather than inline — a Lewdware
                // function must never call back into Lua synchronously (execution model rule 1).
                tokio::task::spawn_local(async move {
                    if let Err(err) = cb.call::<()>(()) {
                        tracing::error!("{err}");
                    }
                });
            } else {
                this.inner_window()
                    .state
                    .try_borrow_mut()
                    .into_lua_err()?
                    .spawn_callbacks
                    .push(cb);
            }

            Ok(())
        });

        methods.add_method(
            "move",
            |_, this, (opts, cb): (Option<MoveOpts>, Option<mlua::Function>)| {
                let inner_window = this.inner_window();
                let opts = opts.unwrap_or_default();

                let id = {
                    let mut state = inner_window.state.try_borrow_mut().into_lua_err()?;

                    let id = state.current_move_id;
                    state.current_move_id += 1;

                    if let Some(callback) = cb {
                        state.move_callback = Some((id, callback));
                    } else {
                        state.move_callback = None;
                    }

                    id
                };

                inner_window
                    .request_sender
                    .move_window(id, opts)
                    .into_lua_err()?;

                Ok(())
            },
        );

        methods.add_method(
            "fade",
            |_, this, (opts, cb): (Option<FadeOpts>, Option<mlua::Function>)| {
                let inner_window = this.inner_window();
                let opts = opts.unwrap_or_default();

                let id = {
                    let mut state = inner_window.state.try_borrow_mut().into_lua_err()?;

                    let id = state.current_fade_id;
                    state.current_fade_id += 1;

                    if let Some(callback) = cb {
                        state.fade_callback = Some((id, callback));
                    } else {
                        state.fade_callback = None;
                    }

                    id
                };

                inner_window
                    .request_sender
                    .fade_window(id, opts)
                    .into_lua_err()?;

                Ok(())
            },
        );

        methods.add_method("set_title", |_, this, title: Option<String>| {
            this.inner_window()
                .request_sender
                .set_title(title)
                .into_lua_err()?;

            Ok(())
        });

        methods.add_method("set_opacity", |_, this, opacity: f32| {
            this.inner_window()
                .request_sender
                .set_opacity(opacity)
                .into_lua_err()?;

            Ok(())
        });
    }

    pub fn on_close(&self) -> anyhow::Result<()> {
        self.state.try_borrow_mut().into_lua_err()?.closed = true;

        let callbacks = {
            let state = self.state.try_borrow()?;
            state.close_callbacks.clone()
        };

        for cb in callbacks {
            if let Err(err) = cb.call::<()>(()) {
                tracing::error!("{err}");
            }
        }

        Ok(())
    }

    pub fn on_spawn(&self) -> anyhow::Result<()> {
        let callbacks = {
            let mut state = self.state.try_borrow_mut()?;

            if state.spawned {
                // Already fired — a window is only ever shown once.
                return Ok(());
            }

            state.spawned = true;
            state.spawn_callbacks.clone()
        };

        for cb in callbacks {
            if let Err(err) = cb.call::<()>(()) {
                tracing::error!("{err}");
            }
        }

        Ok(())
    }

    pub fn on_move_finished(&self, move_id: u64, x: i32, y: i32) -> anyhow::Result<()> {
        let cb = {
            let mut state = self.state.try_borrow_mut()?;
            state.x = x;
            state.y = y;
            match state.move_callback.take() {
                Some((id, cb)) if move_id == id => Some(cb),
                _ => None,
            }
        };

        if let Some(cb) = cb
            && let Err(err) = cb.call::<()>(())
        {
            tracing::error!("{err}");
        }

        Ok(())
    }

    pub fn on_fade_finished(&self, fade_id: u64) -> anyhow::Result<()> {
        let cb = {
            let mut state = self.state.try_borrow_mut()?;

            match state.fade_callback.take() {
                Some((id, cb)) if fade_id == id => Some(cb),
                _ => None,
            }
        };

        if let Some(cb) = cb
            && let Err(err) = cb.call::<()>(())
        {
            tracing::error!("{err}");
        }

        Ok(())
    }
}

impl InnerWindowState {
    fn new(x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            closed: false,
            close_callbacks: Vec::new(),
            spawned: false,
            spawn_callbacks: Vec::new(),
            move_callback: None,
            current_move_id: 0,
            fade_callback: None,
            current_fade_id: 0,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy)]
pub enum Easing {
    #[serde(rename = "linear")]
    #[default]
    Linear,
    #[serde(rename = "ease-in")]
    EaseIn,
    #[serde(rename = "ease-out")]
    EaseOut,
    #[serde(rename = "ease-in-out")]
    EaseInOut,
}

impl Easing {
    pub fn apply(&self, t: f64) -> f64 {
        match self {
            Self::Linear => t,
            // Cubic ease-in
            Self::EaseIn => t * t * t,
            // Cubic ease-out
            Self::EaseOut => {
                let f = t - 1.0;
                f * f * f + 1.0
            }
            // Cubic ease-in-out
            Self::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let f = 2.0 * t - 2.0;
                    0.5 * f * f * f + 1.0
                }
            }
        }
    }
}

fn return_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct MoveOpts {
    pub x: Option<Coord>,
    pub y: Option<Coord>,
    #[serde(default)]
    pub anchor: Anchor,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default)]
    pub relative: bool,
    #[serde(default = "return_true")]
    pub clamp: bool,
}

impl FromLua for MoveOpts {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct FadeOpts {
    pub opacity: f32,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub easing: Easing,
}

impl FromLua for FadeOpts {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}
