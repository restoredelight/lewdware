use std::{cell::RefCell, rc::Rc};

use mlua::{
    ExternalResult, FromLua, IntoLua, LuaSerdeExt, SerializeOptions, UserData, UserDataFields,
    UserDataMethods,
};
use serde::{Deserialize, Serialize};
use winit::window::WindowId;

use crate::{
    lua::{
        Media, WindowProps,
        api::{Anchor, Coord},
        request::WindowRequestSender,
    },
    monitor::Monitor,
};

#[derive(Clone)]
pub enum Window {
    Image(Rc<ImageWindow>),
    Video(Rc<VideoWindow>),
    Prompt(Rc<PromptWindow>),
    Choice(Rc<ChoiceWindow>),
}

impl Window {
    pub fn inner_window(&self) -> &InnerWindow {
        match self {
            Window::Image(image) => &image.inner_window,
            Window::Video(video) => &video.inner_window,
            Window::Prompt(prompt) => &prompt.inner_window,
            Window::Choice(choice) => &choice.inner_window,
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

        methods.add_async_method("pause", async |_, this, _: ()| {
            this.inner_window
                .request_sender
                .pause_video()
                .await
                .into_lua_err()?;

            Ok(())
        });

        methods.add_async_method("play", async |_, this, _: ()| {
            this.inner_window
                .request_sender
                .play_video()
                .await
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

pub struct PromptWindow {
    inner_window: InnerWindow,
    state: RefCell<PromptWindowState>,
}

struct PromptWindowState {
    text: Option<String>,
    value: String,
    submit_callbacks: Vec<mlua::Function>,
}

impl PromptWindowState {
    fn new(text: Option<String>, value: String) -> Self {
        Self {
            text,
            value,
            submit_callbacks: Vec::new(),
        }
    }
}

impl UserData for PromptWindow {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "prompt");

        fields.add_field_method_get("text", |_, this| {
            Ok(this.state.try_borrow().into_lua_err()?.text.clone())
        });
        fields.add_field_method_get("value", |_, this| {
            Ok(this.state.try_borrow().into_lua_err()?.value.clone())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);

        methods.add_method("on_submit", |_, this, cb: mlua::Function| {
            this.state
                .try_borrow_mut()
                .into_lua_err()?
                .submit_callbacks
                .push(cb);

            Ok(())
        });

        methods.add_async_method("set_text", async |_, this, text: Option<String>| {
            this.inner_window
                .request_sender
                .set_text(text.clone())
                .await?;

            this.state.try_borrow_mut().into_lua_err()?.text = text;

            Ok(())
        });

        methods.add_async_method("set_value", async |_, this, value: Option<String>| {
            this.inner_window
                .request_sender
                .set_value(value.clone())
                .await?;

            this.state.try_borrow_mut().into_lua_err()?.value = value.unwrap_or_default();

            Ok(())
        });
    }
}

impl PromptWindow {
    pub fn new(
        props: WindowProps,
        text: Option<String>,
        value: String,
        request_sender: WindowRequestSender,
    ) -> Self {
        Self {
            inner_window: InnerWindow::new(props, request_sender),
            state: RefCell::new(PromptWindowState::new(text, value)),
        }
    }

    pub fn on_submit(&self, text: String) -> anyhow::Result<()> {
        let callbacks = {
            let state = self.state.try_borrow()?;
            state.submit_callbacks.clone()
        };

        for cb in callbacks {
            let text = text.clone();

            tokio::task::spawn_local(async move {
                if let Err(err) = cb.call_async::<()>(text).await {
                    tracing::error!("{err}");
                }
            });
        }

        Ok(())
    }
}

pub struct ChoiceWindow {
    inner_window: InnerWindow,
    state: RefCell<ChoiceWindowState>,
}

struct ChoiceWindowState {
    text: Option<String>,
    options: Vec<ChoiceWindowOption>,
    select_callbacks: Vec<mlua::Function>,
}

impl ChoiceWindowState {
    fn new(text: Option<String>, options: Vec<ChoiceWindowOption>) -> Self {
        Self {
            text,
            options,
            select_callbacks: Vec::new(),
        }
    }
}

impl UserData for ChoiceWindow {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "choice");

        fields.add_field_method_get("text", |_, this| {
            Ok(this.state.try_borrow().into_lua_err()?.text.clone())
        });
        fields.add_field_method_get("options", |_, this| {
            Ok(this.state.try_borrow().into_lua_err()?.options.clone())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);

        methods.add_method("on_select", |_, this, cb: mlua::Function| {
            this.state
                .try_borrow_mut()
                .into_lua_err()?
                .select_callbacks
                .push(cb);

            Ok(())
        });

        methods.add_async_method("set_text", async |_, this, text: Option<String>| {
            this.inner_window
                .request_sender
                .set_text(text.clone())
                .await?;

            this.state.try_borrow_mut().into_lua_err()?.text = text;

            Ok(())
        });

        methods.add_async_method(
            "set_options",
            async |_, this, options: Option<Vec<ChoiceWindowOption>>| {
                let options = options.unwrap_or_default();

                this.inner_window
                    .request_sender
                    .set_options(options.clone())
                    .await?;

                this.state.try_borrow_mut().into_lua_err()?.options = options;

                Ok(())
            },
        );
    }
}

impl ChoiceWindow {
    pub fn new(
        props: WindowProps,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
        request_sender: WindowRequestSender,
    ) -> Self {
        Self {
            inner_window: InnerWindow::new(props, request_sender),
            state: RefCell::new(ChoiceWindowState::new(text, options)),
        }
    }

    pub fn on_select(&self, id: String) -> anyhow::Result<()> {
        let callbacks = {
            let state = self.state.try_borrow()?;
            state.select_callbacks.clone()
        };

        for cb in callbacks {
            let id = id.clone();

            tokio::task::spawn_local(async move {
                if let Err(err) = cb.call_async::<()>(id).await {
                    tracing::error!("{err}");
                }
            });
        }

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChoiceWindowOption {
    pub id: String,
    pub label: String,
}

impl IntoLua for ChoiceWindowOption {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        lua.to_value_with(&self, SerializeOptions::new().serialize_none_to_null(false))
    }
}

impl FromLua for ChoiceWindowOption {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub struct InnerWindow {
    id: WindowId,
    width: u32,
    height: u32,
    outer_width: u32,
    outer_height: u32,
    state: RefCell<InnerWindowState>,
    monitor: Monitor,
    request_sender: WindowRequestSender,
}

struct InnerWindowState {
    x: u32,
    y: u32,
    visible: bool,
    closed: bool,
    close_callbacks: Vec<mlua::Function>,
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

impl HasInnerWindow for PromptWindow {
    fn inner_window(&self) -> &InnerWindow {
        &self.inner_window
    }
}

impl HasInnerWindow for ChoiceWindow {
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
            state: RefCell::new(InnerWindowState::new(props.x, props.y, props.visible)),
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
        fields.add_field_method_get("visible", |_, this| {
            Ok(this
                .inner_window()
                .state
                .try_borrow()
                .into_lua_err()?
                .visible)
        });
    }

    fn add_methods<T: HasInnerWindow + 'static, M: UserDataMethods<T>>(methods: &mut M) {
        methods.add_async_method("close", async |_, this, _: ()| {
            let inner_window = this.inner_window();

            inner_window.request_sender.close().await.into_lua_err()?;

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

        methods.add_async_method(
            "move",
            async |_, this, (opts, cb): (Option<MoveOpts>, Option<mlua::Function>)| {
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
                    .await
                    .into_lua_err()?;

                Ok(())
            },
        );

        methods.add_async_method(
            "fade",
            async |_, this, (opts, cb): (Option<FadeOpts>, Option<mlua::Function>)| {
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
                    .await
                    .into_lua_err()?;

                Ok(())
            },
        );

        methods.add_async_method("set_visible", async |_, this, visible: bool| {
            this.inner_window()
                .request_sender
                .set_visible(visible)
                .await
                .into_lua_err()?;

            this.inner_window()
                .state
                .try_borrow_mut()
                .into_lua_err()?
                .visible = visible;

            Ok(())
        });

        methods.add_async_method("set_title", async |_, this, title: Option<String>| {
            this.inner_window()
                .request_sender
                .set_title(title)
                .await
                .into_lua_err()?;

            Ok(())
        });

        methods.add_async_method("set_opacity", async |_, this, opacity: f32| {
            this.inner_window()
                .request_sender
                .set_opacity(opacity)
                .await
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
            tokio::task::spawn_local(async move {
                if let Err(err) = cb.call_async::<()>(()).await {
                    tracing::error!("{err}");
                }
            });
        }

        Ok(())
    }

    pub fn on_move_finished(&self, move_id: u64) -> anyhow::Result<()> {
        let cb = {
            let mut state = self.state.try_borrow_mut()?;

            match state.move_callback.take() {
                Some((id, cb)) if move_id == id => Some(cb),
                _ => None,
            }
        };

        if let Some(cb) = cb {
            tokio::task::spawn_local(async move {
                if let Err(err) = cb.call_async::<()>(()).await {
                    tracing::error!("{err}");
                }
            });
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

        if let Some(cb) = cb {
            tokio::task::spawn_local(async move {
                if let Err(err) = cb.call_async::<()>(()).await {
                    tracing::error!("{err}");
                }
            });
        }

        Ok(())
    }
}

impl InnerWindowState {
    fn new(x: u32, y: u32, visible: bool) -> Self {
        Self {
            x,
            y,
            visible,
            closed: false,
            close_callbacks: Vec::new(),
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
