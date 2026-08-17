use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mlua::{ExternalResult, FromLua, Lua, LuaSerdeExt, UserData, UserDataFields, UserDataMethods};
use serde::{Deserialize, Serialize};

use crate::{
    app::EventPoster,
    lua::{
        DialogElementUpdate, ItemId, Media, WindowProps,
        api::{Anchor, Coord},
        dev_log::log_noop,
        request::WindowRequestSender,
    },
    monitor::Monitor,
};

#[derive(Clone)]
pub enum Window<T: EventPoster> {
    Image(Rc<ImageWindow<T>>),
    Video(Rc<VideoWindow<T>>),
    Dialog(Rc<DialogWindow<T>>),
    Text(Rc<TextWindow<T>>),
}

impl<T: EventPoster> Window<T> {
    pub fn inner_window(&self) -> &InnerWindow<T> {
        match self {
            Window::Image(image) => &image.inner_window,
            Window::Video(video) => &video.inner_window,
            Window::Dialog(dialog) => &dialog.inner_window,
            Window::Text(text) => &text.inner_window,
        }
    }
}

pub struct ImageWindow<T: EventPoster> {
    inner_window: InnerWindow<T>,
    image: Media,
}

impl<T: EventPoster> UserData for ImageWindow<T> {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "image");
        fields.add_field_method_get("image", |_, this| Ok(this.image.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);
    }
}

impl<T: EventPoster> ImageWindow<T> {
    pub fn new(
        props: WindowProps,
        image: Media,
        request_sender: WindowRequestSender<T>,
        dev_mode: bool,
    ) -> Self {
        ImageWindow {
            inner_window: InnerWindow::new(props, request_sender, dev_mode),
            image,
        }
    }
}

pub struct VideoWindow<T: EventPoster> {
    inner_window: InnerWindow<T>,
    video: Media,
}

impl<T: EventPoster> UserData for VideoWindow<T> {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "video");
        fields.add_field_method_get("video", |_, this| Ok(this.video.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);

        methods.add_method("pause", |lua, this, _: ()| {
            let result = this
                .inner_window
                .request_sender
                .pause_video()
                .into_lua_err()?;
            Ok(this
                .inner_window
                .report_noop(lua, "VideoWindow:pause()", result))
        });

        methods.add_method("play", |lua, this, _: ()| {
            let result = this
                .inner_window
                .request_sender
                .play_video()
                .into_lua_err()?;
            Ok(this
                .inner_window
                .report_noop(lua, "VideoWindow:play()", result))
        });

        methods.add_method("set_volume", |lua, this, volume: f32| {
            this.inner_window
                .state
                .try_borrow_mut()
                .into_lua_err()?
                .volume_fade_callback = None;
            let result = this
                .inner_window
                .request_sender
                .set_video_volume(volume)
                .into_lua_err()?;
            Ok(this
                .inner_window
                .report_noop(lua, "VideoWindow:set_volume()", result))
        });

        methods.add_method(
            "fade_volume",
            |lua,
             this,
             (opts, cb): (Option<crate::lua::VolumeFadeOpts>, Option<mlua::Function>)| {
                let id = {
                    let mut state = this.inner_window.state.try_borrow_mut().into_lua_err()?;
                    let id = state.current_volume_fade_id;
                    state.current_volume_fade_id += 1;
                    state.volume_fade_callback = opts
                        .as_ref()
                        .and_then(|_| cb.map(|callback| (id, callback)));
                    id
                };
                let result = this
                    .inner_window
                    .request_sender
                    .fade_video_volume(id, opts)
                    .into_lua_err()?;
                Ok(this
                    .inner_window
                    .report_noop(lua, "VideoWindow:fade_volume()", result))
            },
        );

        methods.add_method("set_loop", |lua, this, loop_video: bool| {
            let result = this
                .inner_window
                .request_sender
                .set_video_loop(loop_video)
                .into_lua_err()?;
            Ok(this
                .inner_window
                .report_noop(lua, "VideoWindow:set_loop()", result))
        });
    }
}

impl<T: EventPoster> VideoWindow<T> {
    pub fn new(
        props: WindowProps,
        video: Media,
        request_tx: WindowRequestSender<T>,
        dev_mode: bool,
    ) -> Self {
        VideoWindow {
            inner_window: InnerWindow::new(props, request_tx, dev_mode),
            video,
        }
    }
}

pub struct DialogWindow<T: EventPoster> {
    inner_window: InnerWindow<T>,
    state: RefCell<DialogWindowState>,
}

struct DialogWindowState {
    select_callbacks: Vec<mlua::Function>,
    submit_callbacks: Vec<mlua::Function>,
}

impl DialogWindowState {
    fn new() -> Self {
        Self {
            select_callbacks: Vec::new(),
            submit_callbacks: Vec::new(),
        }
    }
}

impl<T: EventPoster> UserData for DialogWindow<T> {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "dialog");
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);

        // Fires on selecting a button (by click or, for the `default` one, Enter in an input) --
        // distinct from the physical `Window:on_click()` content click, which fires only for
        // clicks not consumed by an interactive element (see `DialogWindow::mark_pending_content_click`).
        methods.add_method("on_select", |_, this, cb: mlua::Function| {
            this.state
                .try_borrow_mut()
                .into_lua_err()?
                .select_callbacks
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
        // meaningful reply for a still-open dialog with no input elements. Unlike `value()`
        // below, nil here is unambiguous (always means "closed"), so it's still worth a dev-mode
        // no-op log.
        methods.add_method("values", |lua, this, _: ()| {
            let result = this.inner_window.request_sender.get_dialog_values()?;
            this.inner_window
                .report_noop(lua, "DialogWindow:values()", result.is_some());
            Ok(result)
        });

        // Nil for a closed window OR an id that isn't a live input element -- both just mean
        // "no value to report" from the caller's perspective, and (unlike `values()`) an
        // unrecognised id is an entirely normal case, not a dead-object issue -- so this isn't
        // logged as a no-op.
        methods.add_method("value", |_, this, id: String| {
            Ok(this.inner_window.request_sender.get_dialog_value(id)?)
        });

        methods.add_method(
            "update",
            |lua, this, (id, props): (String, DialogElementUpdate)| {
                let result = this
                    .inner_window
                    .request_sender
                    .update_dialog_element(id, props)?;
                Ok(this
                    .inner_window
                    .report_noop(lua, "DialogWindow:update()", result))
            },
        );
    }
}

impl<T: EventPoster> DialogWindow<T> {
    pub fn new(props: WindowProps, request_sender: WindowRequestSender<T>, dev_mode: bool) -> Self {
        Self {
            inner_window: InnerWindow::new(props, request_sender, dev_mode),
            state: RefCell::new(DialogWindowState::new()),
        }
    }

    pub fn on_select(
        &self,
        button_id: String,
        values: HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let callbacks = {
            let state = self.state.try_borrow()?;
            state.select_callbacks.clone()
        };

        for cb in callbacks {
            if let Err(err) = cb.call::<()>((button_id.clone(), values.clone())) {
                tracing::error!("{err}");
            }
        }

        Ok(())
    }

    pub fn on_submit(
        &self,
        element_id: String,
        values: HashMap<String, String>,
    ) -> anyhow::Result<()> {
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

pub struct TextWindow<T: EventPoster> {
    inner_window: InnerWindow<T>,
    state: RefCell<TextWindowState>,
}

struct TextWindowState {
    text: String,
}

impl<T: EventPoster> UserData for TextWindow<T> {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        InnerWindow::add_fields(fields);

        fields.add_field("type", "text");

        fields.add_field_method_get("text", |_, this| {
            Ok(this.state.try_borrow().into_lua_err()?.text.clone())
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        InnerWindow::add_methods(methods);

        methods.add_method("set_text", |lua, this, text: String| {
            let changed = this
                .inner_window
                .request_sender
                .set_text(Some(text.clone()))?;

            // Only update the local cache if the window was actually open to receive it --
            // otherwise `text` would report a change that never took effect.
            if changed {
                this.state.try_borrow_mut().into_lua_err()?.text = text;
            }

            Ok(this
                .inner_window
                .report_noop(lua, "TextWindow:set_text()", changed))
        });
    }
}

impl<T: EventPoster> TextWindow<T> {
    pub fn new(
        props: WindowProps,
        text: String,
        request_sender: WindowRequestSender<T>,
        dev_mode: bool,
    ) -> Self {
        Self {
            inner_window: InnerWindow::new(props, request_sender, dev_mode),
            state: RefCell::new(TextWindowState { text }),
        }
    }
}

pub struct InnerWindow<T: EventPoster> {
    id: ItemId,
    width: u32,
    height: u32,
    outer_width: u32,
    outer_height: u32,
    state: RefCell<InnerWindowState>,
    monitor: Monitor,
    request_sender: WindowRequestSender<T>,
    dev_mode: bool,
}

struct InnerWindowState {
    x: i32,
    y: i32,
    closed: bool,
    close_callbacks: Vec<mlua::Function>,
    spawned: bool,
    spawn_callbacks: Vec<mlua::Function>,
    click_callbacks: Vec<mlua::Function>,
    move_callback: Option<(u64, mlua::Function)>,
    current_move_id: u64,
    fade_callback: Option<(u64, mlua::Function)>,
    current_fade_id: u64,
    volume_fade_callback: Option<(u64, mlua::Function)>,
    current_volume_fade_id: u64,
}

trait HasInnerWindow<T: EventPoster> {
    fn inner_window(&self) -> &InnerWindow<T>;
}

impl<T: EventPoster> HasInnerWindow<T> for ImageWindow<T> {
    fn inner_window(&self) -> &InnerWindow<T> {
        &self.inner_window
    }
}

impl<T: EventPoster> HasInnerWindow<T> for VideoWindow<T> {
    fn inner_window(&self) -> &InnerWindow<T> {
        &self.inner_window
    }
}

impl<T: EventPoster> HasInnerWindow<T> for DialogWindow<T> {
    fn inner_window(&self) -> &InnerWindow<T> {
        &self.inner_window
    }
}

impl<T: EventPoster> HasInnerWindow<T> for TextWindow<T> {
    fn inner_window(&self) -> &InnerWindow<T> {
        &self.inner_window
    }
}

impl<T: EventPoster> InnerWindow<T> {
    pub fn new(props: WindowProps, request_tx: WindowRequestSender<T>, dev_mode: bool) -> Self {
        Self {
            id: props.window_id,
            width: props.width,
            height: props.height,
            outer_width: props.outer_width,
            outer_height: props.outer_height,
            state: RefCell::new(InnerWindowState::new(props.x, props.y)),
            monitor: props.monitor,
            request_sender: request_tx,
            dev_mode,
        }
    }

    /// Logs a dev-mode warning (with the Lua call site) when `happened` is `false` -- i.e. the
    /// call was a no-op because the window is already closed (execution model rule 3). Returns
    /// `happened` unchanged, so call sites can just wrap their result: `Ok(self.report_noop(lua,
    /// "...", result))`.
    fn report_noop(&self, lua: &Lua, what: &str, happened: bool) -> bool {
        if !happened && self.dev_mode {
            log_noop(lua, what);
        }
        happened
    }

    fn add_fields<U: HasInnerWindow<T>, F: UserDataFields<U>>(fields: &mut F) {
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

    fn add_methods<U: HasInnerWindow<T> + 'static, M: UserDataMethods<U>>(methods: &mut M) {
        methods.add_method("close", |lua, this, _: ()| {
            let closed_now = this.inner_window().request_sender.close().into_lua_err()?;

            // Set eagerly, rather than waiting for the `WindowClosed` event this also triggers
            // (asynchronous -- only processed on a later spin of the Lua thread's event loop): a
            // mode checking `.closed` or calling `on_close()`/`on_spawn()` right after `close()`
            // returns should already see the window as closed, not a stale snapshot from before
            // the request round-trip. `on_close`'s callbacks still fire only once the event is
            // processed, same as before -- this only advances the flag, not callback timing.
            if closed_now {
                this.inner_window()
                    .state
                    .try_borrow_mut()
                    .into_lua_err()?
                    .closed = true;
            }

            Ok(this
                .inner_window()
                .report_noop(lua, "Window:close()", closed_now))
        });

        methods.add_method("on_close", |lua, this, cb: mlua::Function| {
            let mut state = this.inner_window().state.try_borrow_mut().into_lua_err()?;

            if state.closed {
                return Ok(this
                    .inner_window()
                    .report_noop(lua, "Window:on_close()", false));
            }

            state.close_callbacks.push(cb);
            Ok(true)
        });

        // A physical content-area click -- "the user poked this window" (execution model). Fires
        // on every qualifying click, not just the first (unlike `on_spawn`). On `DialogWindow`,
        // only fires for clicks not consumed by a button/input -- see
        // `DialogWindow::mark_pending_content_click` on the rendering side.
        methods.add_method("on_click", |lua, this, cb: mlua::Function| {
            let mut state = this.inner_window().state.try_borrow_mut().into_lua_err()?;

            if state.closed {
                return Ok(this
                    .inner_window()
                    .report_noop(lua, "Window:on_click()", false));
            }

            state.click_callbacks.push(cb);
            Ok(true)
        });

        methods.add_method("on_spawn", |lua, this, cb: mlua::Function| {
            let (closed, spawned) = {
                let state = this.inner_window().state.try_borrow().into_lua_err()?;
                (state.closed, state.spawned)
            };

            if closed {
                return Ok(this
                    .inner_window()
                    .report_noop(lua, "Window:on_spawn()", false));
            }

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

            Ok(true)
        });

        methods.add_method(
            "move",
            |lua, this, (opts, cb): (Option<MoveOpts>, Option<mlua::Function>)| {
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

                let moved = inner_window
                    .request_sender
                    .move_window(id, opts)
                    .into_lua_err()?;

                Ok(inner_window.report_noop(lua, "Window:move()", moved))
            },
        );

        methods.add_method(
            "fade",
            |lua, this, (opts, cb): (Option<FadeOpts>, Option<mlua::Function>)| {
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

                let faded = inner_window
                    .request_sender
                    .fade_window(id, opts)
                    .into_lua_err()?;

                Ok(inner_window.report_noop(lua, "Window:fade()", faded))
            },
        );

        methods.add_method("set_title", |lua, this, title: Option<String>| {
            let result = this
                .inner_window()
                .request_sender
                .set_title(title)
                .into_lua_err()?;
            Ok(this
                .inner_window()
                .report_noop(lua, "Window:set_title()", result))
        });

        methods.add_method("set_opacity", |lua, this, opacity: f32| {
            let result = this
                .inner_window()
                .request_sender
                .set_opacity(opacity)
                .into_lua_err()?;
            Ok(this
                .inner_window()
                .report_noop(lua, "Window:set_opacity()", result))
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

    /// Called for every qualifying content click (see `Window:on_click()`'s registration) -- no
    /// "already fired" guard, unlike `on_spawn`, since a window can be clicked any number of
    /// times.
    pub fn on_click(&self) -> anyhow::Result<()> {
        let callbacks = {
            let state = self.state.try_borrow()?;
            state.click_callbacks.clone()
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

    pub fn on_volume_fade_finished(&self, fade_id: u64) -> anyhow::Result<()> {
        let callback = {
            let mut state = self.state.try_borrow_mut()?;
            if state
                .volume_fade_callback
                .as_ref()
                .is_some_and(|(id, _)| *id == fade_id)
            {
                state
                    .volume_fade_callback
                    .take()
                    .map(|(_, callback)| callback)
            } else {
                None
            }
        };
        if let Some(callback) = callback
            && let Err(err) = callback.call::<()>(())
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
            click_callbacks: Vec::new(),
            move_callback: None,
            current_move_id: 0,
            fade_callback: None,
            current_fade_id: 0,
            volume_fade_callback: None,
            current_volume_fade_id: 0,
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
