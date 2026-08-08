use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::mpsc;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use crate::app::UserEvent;
use crate::error::LewdwareError;
use crate::lua::{self, Coord, Easing, FadeOpts, ItemId, MoveOpts};
use crate::window::appearance;
use crate::window::decorations::Decorations;
use crate::window::opts::WindowOpts;
use crate::window::redraw::RedrawRequester;
use crate::window::target::RenderTarget;
use crate::window::theme::{Appearance, Theme};

/// Everything about a window that isn't the rendering backend: geometry, the move/fade
/// animations, pointer state, decorations, and the channel back to Lua.
///
/// Paired with a [`RenderTarget`](crate::window::RenderTarget), which owns the surface. The two
/// are kept apart so content types can depend on only the half they need.
pub struct WindowState {
    window: Arc<Window>,
    redraw: RedrawRequester,
    id: ItemId,
    decorations: Decorations,
    inner_size: PhysicalSize<u32>,
    outer_size: PhysicalSize<u32>,
    monitor_position: LogicalPosition<i32>,
    monitor_size: LogicalSize<u32>,
    position: LogicalPosition<i32>,
    lua_event_tx: mpsc::UnboundedSender<lua::Event>,
    current_move: Option<Move>,
    last_move_update: Instant,
    current_fade: Option<Fade>,
    last_fade_update: Instant,
    pub opacity: f32,
    transparent: bool,
    background_color: Option<lua::Color>,
    theme: Theme,
    appearance: Appearance,
    content_hover: bool,
    content_clicked: bool,
}

/// Returned by `WindowState::handle_mouse_up`, since a release can mean two independent things:
/// the close button was activated (decorations), or a qualifying content click just happened.
pub struct MouseUpResult {
    pub should_close: bool,
    pub content_click: bool,
}

struct Move {
    id: u64,
    from: LogicalPosition<i32>,
    to: LogicalPosition<i32>,
    duration: Duration,
    start: Instant,
    easing: Easing,
}

struct Fade {
    id: u64,
    from: f32,
    to: f32,
    duration: Duration,
    start: Instant,
    easing: Easing,
}

impl WindowState {
    pub fn new(
        window: Arc<Window>,
        opts: &WindowOpts,
        lua_event_tx: mpsc::UnboundedSender<lua::Event>,
        popup_id: ItemId,
        event_loop_proxy: EventLoopProxy<UserEvent>,
        redraw_wakeup_pending: Arc<AtomicBool>,
    ) -> Self {
        // Use opts directly rather than window.inner_size(): request_inner_size() is
        // async on X11, so a recycled pool window still reports its previous size here.
        let scale_factor = window.scale_factor();
        let outer_size: PhysicalSize<u32> =
            LogicalSize::new(opts.popup_opts.outer_width, opts.popup_opts.outer_height)
                .to_physical(scale_factor);
        let inner_size: PhysicalSize<u32> =
            LogicalSize::new(opts.popup_opts.width, opts.popup_opts.height)
                .to_physical(scale_factor);

        let redraw = RedrawRequester::new(window.clone(), event_loop_proxy, redraw_wakeup_pending);

        // Resolved here rather than on the Lua thread because `auto` is runtime state only the
        // main thread can read -- and resolvable this late precisely because appearance never
        // changes the window's metrics, which were already fixed when it was created.
        let appearance = appearance::resolve(opts.popup_opts.appearance, &window);

        let decorations = Decorations::new(
            opts.popup_opts.decorations,
            opts.popup_opts.theme.metrics(),
            opts.popup_opts.theme.chrome(appearance),
            inner_size,
            scale_factor,
            opts.popup_opts.title.clone(),
            opts.popup_opts.closeable,
            redraw.clone(),
        );

        // Every newly-created surface needs one real content frame. This also covers egui's
        // initial repaint request, which occurs before its repaint callback is installed.
        redraw.request_redraw();

        let monitor_position = LogicalPosition::new(
            opts.position.x - opts.popup_opts.x,
            opts.position.y - opts.popup_opts.y,
        );
        let monitor_size = LogicalSize::new(opts.monitor.width, opts.monitor.height);

        Self {
            window,
            redraw,
            id: popup_id,
            decorations,
            inner_size,
            outer_size,
            monitor_position,
            monitor_size,
            position: LogicalPosition::new(opts.popup_opts.x, opts.popup_opts.y),
            lua_event_tx,
            current_move: None,
            last_move_update: Instant::now(),
            current_fade: None,
            last_fade_update: Instant::now(),
            opacity: opts.popup_opts.opacity,
            transparent: opts.popup_opts.transparent,
            background_color: opts.popup_opts.background_color,
            theme: opts.popup_opts.theme,
            appearance,
            content_hover: false,
            content_clicked: false,
        }
    }

    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity;
    }

    pub fn background_color(&self) -> Option<lua::Color> {
        self.background_color
    }

    /// The named look this window is drawn with. Its metrics are already baked into
    /// [`Self::decorations`]; this is for the parts resolved later, such as the egui style a
    /// dialog's widgets use.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// The palette this window resolved to. Already applied to its decorations; this is for the
    /// widget half, which is built later.
    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    pub fn inner_size(&self) -> PhysicalSize<u32> {
        self.inner_size
    }

    pub fn decorations(&self) -> &Decorations {
        &self.decorations
    }

    pub fn decorations_mut(&mut self) -> &mut Decorations {
        &mut self.decorations
    }

    /// Build the decorations' GPU overlay, once the render target is known.
    pub fn attach_decorations(&mut self, target: &RenderTarget) {
        self.decorations
            .attach(target, self.outer_size, self.opacity);
    }

    /// Origin of the content area within the outer window, in physical pixels.
    pub fn inner_offset(&self) -> (u32, u32) {
        self.decorations.content_origin()
    }

    pub fn request_redraw(&self) {
        self.redraw.request_redraw();
    }

    /// Clears and returns this window's dirty flag. Used by the Windows `about_to_wait` path.
    pub fn take_redraw_requested(&self) -> bool {
        self.redraw.take_requested()
    }

    pub fn redraw_requester(&self) -> RedrawRequester {
        self.redraw.clone()
    }

    pub fn start_move(&mut self, id: u64, opts: MoveOpts) -> Result<(), LewdwareError> {
        let scale_factor = self.window.scale_factor();

        let size = self.window.inner_size().to_logical(scale_factor);

        let monitor_size = self.monitor_size;

        let x: Option<i32> = match opts.x {
            Some(Coord::Pixel(x)) => Some(opts.anchor.resolve_x(x, size.width)),
            Some(Coord::Percent { percent }) => Some(opts.anchor.resolve_x(
                ((percent * monitor_size.width as f64) / 100.0).round() as i32,
                size.width,
            )),
            None => None,
        };

        let y: Option<i32> = match opts.y {
            Some(Coord::Pixel(y)) => Some(opts.anchor.resolve_y(y, size.height)),
            Some(Coord::Percent { percent }) => Some(opts.anchor.resolve_y(
                ((percent * monitor_size.height as f64) / 100.0).round() as i32,
                size.height,
            )),
            None => None,
        };

        let clamp = opts.clamp;
        let new_position = if opts.relative {
            LogicalPosition::new(
                if clamp {
                    (self.position.x + x.unwrap_or(0)).max(0)
                } else {
                    self.position.x + x.unwrap_or(0)
                },
                if clamp {
                    (self.position.y + y.unwrap_or(0)).max(0)
                } else {
                    self.position.y + y.unwrap_or(0)
                },
            )
        } else {
            LogicalPosition::new(
                if clamp {
                    x.map(|v| v.max(0)).unwrap_or(self.position.x)
                } else {
                    x.unwrap_or(self.position.x)
                },
                if clamp {
                    y.map(|v| v.max(0)).unwrap_or(self.position.y)
                } else {
                    y.unwrap_or(self.position.y)
                },
            )
        };

        tracing::info!("{:?}", self.position);

        let move_obj = Move {
            id,
            from: self.position,
            to: new_position,
            duration: Duration::from_millis(opts.duration),
            start: Instant::now(),
            easing: opts.easing,
        };

        self.current_move = Some(move_obj);

        Ok(())
    }

    pub fn is_moving(&self) -> bool {
        self.current_move.is_some()
    }

    pub fn update_position(&mut self) {
        if let Some(current_move) = &self.current_move {
            let percent = current_move
                .start
                .elapsed()
                .div_duration_f64(current_move.duration)
                .min(1.0);

            let eased_percent = current_move.easing.apply(percent);

            let new_position = LogicalPosition::new(
                current_move.from.x
                    + ((current_move.to.x as f64 - current_move.from.x as f64) * eased_percent)
                        .round() as i32,
                current_move.from.y
                    + ((current_move.to.y as f64 - current_move.from.y as f64) * eased_percent)
                        .round() as i32,
            );

            let complete = percent >= 1.0;

            // Throttle visual updates to ~30 fps; always apply the final position on completion
            // so the window lands exactly on the wall edge before the next move starts.
            if new_position != self.position
                && (complete || self.last_move_update.elapsed() >= Duration::from_millis(33))
            {
                self.window.set_outer_position(LogicalPosition::new(
                    self.monitor_position.x + new_position.x,
                    self.monitor_position.y + new_position.y,
                ));
                self.position = new_position;
                self.last_move_update = Instant::now();
            }

            if complete {
                if let Err(err) = self.lua_event_tx.send(lua::Event::MoveFinish {
                    id: self.id,
                    move_id: current_move.id,
                    x: self.position.x,
                    y: self.position.y,
                }) {
                    tracing::error!("{err}");
                }

                self.current_move = None;
            }
        }
    }

    pub fn start_fade(&mut self, id: u64, opts: FadeOpts) -> Result<(), LewdwareError> {
        let to = opts.opacity;

        let fade_obj = Fade {
            id,
            from: self.opacity,
            to,
            duration: Duration::from_millis(opts.duration),
            start: Instant::now(),
            easing: opts.easing,
        };

        self.current_fade = Some(fade_obj);

        Ok(())
    }

    pub fn is_fading(&self) -> bool {
        self.current_fade.is_some()
    }

    pub fn update_fade(&mut self) {
        let (new_opacity, is_finished, fade_id) = if let Some(current_fade) = &self.current_fade {
            let percent = current_fade
                .start
                .elapsed()
                .div_duration_f64(current_fade.duration)
                .min(1.0);

            let eased_percent = current_fade.easing.apply(percent);

            let new_opacity = current_fade.from
                + ((current_fade.to - current_fade.from) as f64 * eased_percent) as f32;

            (new_opacity, percent >= 1.0, current_fade.id)
        } else {
            return;
        };

        if new_opacity != self.opacity
            && (is_finished || self.last_fade_update.elapsed() >= Duration::from_millis(33))
        {
            self.set_opacity(new_opacity);
            self.request_redraw();
            self.last_fade_update = Instant::now();
        }

        if is_finished {
            if let Err(err) = self.lua_event_tx.send(lua::Event::FadeFinish {
                id: self.id,
                fade_id,
            }) {
                tracing::error!("{err}");
            }

            self.current_fade = None;
        }
    }

    /// Whether `position` (physical, window-relative) falls within the content area -- excludes
    /// the border and header when `decorations` is on; the whole window otherwise.
    fn in_content_bounds(&self, position: PhysicalPosition<f64>) -> bool {
        let (ox, oy) = self.inner_offset();
        position.x >= ox as f64
            && position.y >= oy as f64
            && position.x < (ox + self.inner_size.width) as f64
            && position.y < (oy + self.inner_size.height) as f64
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        self.decorations.handle_cursor_moved(position);
        self.content_hover = self.in_content_bounds(position);
    }

    pub fn handle_cursor_left(&mut self) {
        self.decorations.handle_cursor_left();
        self.content_hover = false;
        self.content_clicked = false;
    }

    pub fn handle_mouse_down(&mut self) {
        self.decorations.handle_mouse_down();
        if self.content_hover {
            self.content_clicked = true;
        }
    }

    pub fn handle_mouse_up(&mut self) -> MouseUpResult {
        let should_close = self.decorations.handle_mouse_up();

        let content_click = self.content_hover && self.content_clicked;
        self.content_clicked = false;

        MouseUpResult {
            should_close,
            content_click,
        }
    }

    /// Reveal the window for the first (and only) time. Windows are created invisible (or
    /// parked offscreen, on Linux) and shown once here, when their content is ready to display —
    /// this is the moment `Window.spawned`/`Window:on_spawn()` observe from Lua.
    pub fn show(&self) {
        if self
            .lua_event_tx
            .send(lua::Event::WindowSpawned { id: self.id })
            .is_err()
        {
            tracing::debug!("Couldn't send WindowSpawned event: Lua thread has shut down");
        }

        #[cfg(target_os = "linux")]
        {
            // Move back to the correct absolute position before showing.
            self.window.set_outer_position(LogicalPosition::new(
                self.monitor_position.x + self.position.x,
                self.monitor_position.y + self.position.y,
            ));
            self.window.set_visible(true);
            // Recycled (always-mapped) windows are moved with XMoveWindow, which does not
            // restack. Raise explicitly so the window appears above other windows in its layer.
            x11_raise(&self.window);
        }
        #[cfg(not(target_os = "linux"))]
        self.window.set_visible(true);
    }

    pub fn set_title(&mut self, text: Option<String>) {
        self.decorations.set_title(text);
    }

    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    pub fn popup_id(&self) -> ItemId {
        self.id
    }

    pub fn transparent(&self) -> bool {
        self.transparent
    }

    pub fn lua_event_tx(&self) -> &mpsc::UnboundedSender<lua::Event> {
        &self.lua_event_tx
    }

    /// Consume this `WindowState` and return the underlying `Arc<Window>` for pool reuse.
    pub fn into_window(self) -> Arc<Window> {
        self.window.clone()
    }
}

/// Raise `window` to the top of the X11 stacking order without unmapping it.
///
/// `XMoveWindow` (used to park/unpark pooled windows) does not restack, so recycled windows
/// would silently sit below any window mapped since they were last visible. `XRaiseWindow`
/// fixes this without triggering the KWin strut relayout that XMapWindow/XUnmapWindow does.
#[cfg(target_os = "linux")]
fn x11_raise(window: &Window) {
    use winit::raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    };

    let (Ok(wh), Ok(dh)) = (window.window_handle(), window.display_handle()) else {
        return;
    };
    let (RawWindowHandle::Xlib(xlib_win), RawDisplayHandle::Xlib(xlib_dpy)) =
        (wh.as_raw(), dh.as_raw())
    else {
        return;
    };
    let Some(display) = xlib_dpy.display else {
        return;
    };

    let Ok(xlib) = x11_dl::xlib::Xlib::open() else {
        return;
    };
    unsafe {
        (xlib.XRaiseWindow)(display.as_ptr().cast(), xlib_win.window);
        (xlib.XFlush)(display.as_ptr().cast());
    }
}
