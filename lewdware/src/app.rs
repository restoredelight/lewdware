use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::thread;

use anyhow::{Context, anyhow};
use rand::random_range;
use shared::user_config::AppConfig;
use url::{Host, Url};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::MouseButton;
use winit::event_loop::{ControlFlow, EventLoopProxy};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use crate::audio::AudioPlayer;
use crate::error::{LewdwareError, MonitorError, Result};
use crate::lua::{
    self, AudioAction, ChoiceWindowOption, FadeOpts, FontSize, LuaRequest, LuaThreadHandle,
    MoveOpts, Notification, PopupId, SpawnWindowOpts, TextFont, TextStyle, WallpaperMode,
    WindowAction, WindowProps, start_lua_thread,
};
use crate::media::{FileOrPath, ImageData, MediaError, MediaManager};
use crate::monitor::Monitors;
use crate::utils::{calculate_media_popup_size, calculate_text_popup_size};
use crate::video::VideoDecoder;
use crate::wgpu::WgpuState;
use crate::window::{
    ChoiceWindow, HEADER_HEIGHT, ImageWindow, InnerWindow, PromptWindow, TextWindow, VideoWindow,
    WindowOpts, WindowPool, WindowType,
};

/// The main app.
/// * `windows`: A map containing all the popups spawned by the app, keyed by the Lua-facing
///   [`PopupId`] rather than winit's `WindowId` — a popup may exist (and be addressable from
///   Lua) before its real winit window does, while its media is still resolving. Since dropping
///   a winit window closes it, we can close ready windows by removing them from this map.
/// * `window_ids`: Reverse lookup from winit's `WindowId` to `PopupId`, needed to route winit's
///   `window_event` callback (which only knows the real `WindowId`) back to a `PopupSlot`.
/// * `default_wallpaper`: Stores the user's default wallpaper, so we can restore it on panic.
pub struct LewdwareApp {
    running: bool,
    _config: Arc<AppConfig>,
    wgpu_state: Option<Arc<WgpuState>>,
    windows: HashMap<PopupId, PopupSlot>,
    window_ids: HashMap<WindowId, PopupId>,
    next_popup_id: u64,
    audio_players: HashMap<u64, AudioSlot>,
    current_audio_id: u64,
    default_wallpaper: Option<String>,
    lua_request_rx: std::sync::mpsc::Receiver<lua::LuaRequest>,
    lua_event_tx: tokio::sync::mpsc::UnboundedSender<lua::Event>,
    lua_thread_handle: LuaThreadHandle,
    monitors: Monitors,
    window_pool: WindowPool,
    /// The main thread's own clone of the `MediaManager`, opened directly in [`LewdwareApp::new`]
    /// (a clone is also handed to the Lua thread) and used to resolve popup media asynchronously
    /// (see `spawn_image`/`spawn_video`/`spawn_audio`). Always `Some` outside of `Drop` — `new()`
    /// bails out before constructing `Self` if the pack fails to open. Wrapped in `Option` only
    /// so `Drop` can take it (dropping our clone so the manager thread's request channel can
    /// close) before joining `media_manager_handle`, same idiom as `LuaThreadHandle`'s own
    /// fields.
    media_manager: Option<MediaManager>,
    media_manager_handle: Option<thread::JoinHandle<()>>,
}

/// A popup that has been acked to Lua (its [`PopupId`] and [`WindowProps`] are already fixed)
/// but whose real winit window hasn't been created yet, because its media is still being
/// resolved on a background thread. See `spawn_image`/`spawn_video` and the `*Resolved`
/// `UserEvent` handlers.
// `WindowType` is legitimately larger than `PendingWindow` (it owns GPU/CPU render buffers);
// boxing it would mean pattern-matching through a `Box` at every one of the many call sites
// below, for no real benefit — popups aren't allocated often enough for this to matter.
#[allow(clippy::large_enum_variant)]
enum PopupSlot {
    Pending(PendingWindow),
    Ready(WindowType),
}

struct PendingWindow {
    kind: PendingKind,
    /// Already fully resolved — position/size/monitor/transparency never depended on the
    /// decoded media, only its (already-known) native dimensions.
    opts: WindowOpts,
    visible: bool,
    opacity: f32,
    title: Option<String>,
    pending_move: Option<(u64, MoveOpts)>,
    pending_fade: Option<(u64, FadeOpts)>,
    /// Only meaningful for `PendingKind::Video`.
    video_paused: bool,
}

enum PendingKind {
    Image,
    Video { loop_video: bool },
}

/// An audio slot, analogous to [`PopupSlot`] but with no window involved.
enum AudioSlot {
    Pending { paused: bool },
    Ready(AudioPlayer),
}

enum WindowSizeBehaviour {
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
        border_width: f32,
    },
}

pub enum UserEvent {
    Exit,
    LuaRequest,
    AudioFinish { id: u64 },
    ImageResolved { id: PopupId, result: std::result::Result<ImageData, MediaError> },
    VideoResolved { id: PopupId, result: std::result::Result<VideoDecoder, MediaError> },
    AudioResolved { id: u64, result: std::result::Result<AudioPlayer, MediaError> },
}

impl LewdwareApp {
    pub fn new(
        wgpu_state: Option<std::sync::Arc<WgpuState>>,
        event_loop_proxy: EventLoopProxy<UserEvent>,
        config: AppConfig,
    ) -> anyhow::Result<Self> {
        let config = Arc::new(config);

        let wallpaper = match wallpaper::get() {
            Ok(wallpaper) => Some(wallpaper),
            Err(err) => {
                tracing::error!("Error getting wallpaper: {}", err);
                None
            }
        };

        tracing::info!("{:?}", config);
        // local video = lewdware.media.random_video()
        // local video_window = lewdware.spawn_video_popup(video, {
        //     width = { percent = 100 },
        //     height = { percent = 100 },
        //     decorations = false,
        //     visible = false,
        // })
        // lewdware.every(10000, function()
        //     video_window:set_visible(true)
        //     lewdware.after(500, function()
        //         video_window:set_visible(false)
        //     end)
        // end)

        let pack_path = config
            .pack_path
            .as_ref()
            .context("No pack is configured, but the engine requires one to start")?;

        let wgpu_device = wgpu_state.as_ref().map(|s| s.device.clone());

        // Opened here rather than on the Lua thread, so the main thread has its own clone to
        // resolve popup media asynchronously (see `spawn_image`/`spawn_video`/`spawn_audio`). A
        // clone is also handed to the Lua thread below.
        let (media_manager, _metadata, media_manager_handle) =
            MediaManager::open(pack_path, event_loop_proxy.clone(), wgpu_device)?;

        let (lua_event_tx, lua_request_rx, lua_thread_handle) =
            start_lua_thread(event_loop_proxy, config.clone(), media_manager.clone());

        let monitors = Monitors::new(config.disabled_monitors.clone());

        Ok(Self {
            running: false,
            _config: config,
            wgpu_state,
            windows: HashMap::new(),
            window_ids: HashMap::new(),
            next_popup_id: 0,
            audio_players: HashMap::new(),
            current_audio_id: 0,
            default_wallpaper: wallpaper,
            lua_request_rx,
            lua_event_tx,
            lua_thread_handle,
            monitors,
            window_pool: WindowPool::new(),
            media_manager: Some(media_manager),
            media_manager_handle: Some(media_manager_handle),
        })
    }

    /// Allocate a new [`PopupId`]. See the doc comment on [`LewdwareApp::windows`] for why this
    /// is a distinct identifier from winit's `WindowId`.
    fn next_popup_id(&mut self) -> PopupId {
        let id = self.next_popup_id;
        self.next_popup_id += 1;
        PopupId(id)
    }

    /// Returns the main thread's `MediaManager` clone. See the doc comment on
    /// [`LewdwareApp::media_manager`] for when this is `None`.
    fn media_manager(&self) -> Result<MediaManager> {
        self.media_manager
            .clone()
            .ok_or(LewdwareError::Internal("Media manager not available"))
    }

    /// Resolve a [`SpawnWindowOpts`] into a fully computed [`WindowOpts`], factoring in the
    /// monitor layout, GPU availability, and size constraints.
    fn resolve_window_opts(
        &mut self,
        spawn_opts: SpawnWindowOpts,
        size_behaviour: WindowSizeBehaviour,
        mut gpu: bool,
        transparent: bool,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowOpts> {
        if self.wgpu_state.is_none() {
            gpu = false;
        }
        let transparent = transparent && gpu;
        let force_opaque = spawn_opts.transparent == Some(false);

        let monitor = match spawn_opts.monitor {
            Some(m) => m,
            None => self.monitors.random(event_loop)?,
        };

        let monitor_handle = self
            .monitors
            .get_handle(monitor.id, event_loop)
            .ok_or(MonitorError::MonitorNotFound)?;

        let scale_factor = monitor_handle.scale_factor();
        let monitor_size = monitor_handle.size().to_logical(scale_factor);
        let monitor_position: LogicalPosition<i32> =
            monitor_handle.position().to_logical(scale_factor);

        let (width, height) = match size_behaviour {
            WindowSizeBehaviour::ResizeWithMedia { width, height } => calculate_media_popup_size(
                spawn_opts.width,
                spawn_opts.height,
                width,
                height,
                monitor_size.width,
                monitor_size.height,
            ),
            WindowSizeBehaviour::UseDefaults { width, height } => (
                spawn_opts
                    .width
                    .map(|w| w.to_pixels(monitor_size.width).max(0) as u32)
                    .unwrap_or(width),
                spawn_opts
                    .height
                    .map(|h| h.to_pixels(monitor_size.height).max(0) as u32)
                    .unwrap_or(height),
            ),
            WindowSizeBehaviour::MeasureText {
                text,
                font,
                font_size,
                border_width,
            } => calculate_text_popup_size(
                spawn_opts.width.clone(),
                spawn_opts.height.clone(),
                &text,
                font,
                font_size.to_pixels(monitor_size.height),
                border_width,
                monitor_size.width,
                monitor_size.height,
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
                        .resolve(c.to_pixels(monitor_size.width), outer_width)
                })
                .unwrap_or_else(|| random_position(outer_width, monitor_size.width));
            if spawn_opts.clamp {
                v.max(0)
                    .min(monitor_size.width.saturating_sub(outer_width) as i32)
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
                        .resolve(c.to_pixels(monitor_size.height), outer_height)
                })
                .unwrap_or_else(|| random_position(outer_height, monitor_size.height));
            if spawn_opts.clamp {
                v.max(0)
                    .min(monitor_size.height.saturating_sub(outer_height) as i32)
            } else {
                v
            }
        };

        let position = LogicalPosition::new(monitor_position.x + x, monitor_position.y + y);

        Ok(WindowOpts {
            position,
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
            visible: spawn_opts.visible,
            decorations: spawn_opts.decorations,
            title: spawn_opts.title,
            closeable: spawn_opts.closeable,
            monitor,
            background_color: spawn_opts.background_color,
        })
    }

    /// Acquire a window from the pool (or create one), configure it, and wrap it in an
    /// [`InnerWindow`]. `popup_id` is already known (allocated when the spawn was first acked to
    /// Lua) and is registered here in [`Self::window_ids`] for winit event routing.
    fn create_window(
        &mut self,
        popup_id: PopupId,
        opts: WindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<InnerWindow> {
        let window = self
            .window_pool
            .acquire(&opts, event_loop)
            .map_err(LewdwareError::WindowError)?;

        let _ = window.set_cursor_hittest(!opts.click_through);

        self.window_ids.insert(window.id(), popup_id);

        let inner_window = InnerWindow::new(
            window,
            &opts,
            self.wgpu_state.clone(),
            self.lua_event_tx.clone(),
            popup_id,
        )
        .map_err(LewdwareError::WindowError)?;

        Ok(inner_window)
    }

    /// Build the [`WindowProps`] sent back to Lua as the spawn ack. Fully accurate even before
    /// the real winit window exists — size/position/monitor never depended on decoded media.
    fn window_props(popup_id: PopupId, opts: &WindowOpts) -> WindowProps {
        WindowProps {
            window_id: popup_id,
            width: opts.width,
            height: opts.height,
            outer_width: opts.outer_width,
            outer_height: opts.outer_height,
            x: opts.x,
            y: opts.y,
            monitor: opts.monitor.clone(),
            visible: opts.visible,
        }
    }

    /// Release a window back to the pool. Moving offscreen rather than unmapping avoids
    /// the KWin strut relayout freeze on Dock-type windows.
    fn close_window(&mut self, window_type: WindowType) {
        self.window_ids.remove(&window_type.inner_window().window().id());
        let transparent = window_type.inner_window().transparent();
        // Move offscreen before dropping InnerWindow so the surface is still alive when KWin
        // processes the XMoveWindow. Without this, transparent (wgpu) windows flash black at
        // their visible position between surface drop and the pool's -32000 move.
        window_type
            .inner_window()
            .window()
            .set_outer_position(LogicalPosition::new(-32000i32, -32000i32));
        let arc_window = window_type.into_inner_window().into_arc_window();
        self.window_pool.release(arc_window, transparent);
    }

    fn spawn_image(
        &mut self,
        media_id: u64,
        width: u32,
        height: u32,
        opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        tracing::info!("Windows: {}", self.windows.len());
        let transparent = opts.transparent.unwrap_or(false);
        let window_opts = self.resolve_window_opts(
            opts,
            WindowSizeBehaviour::ResizeWithMedia { width, height },
            transparent,
            transparent,
            event_loop,
        )?;

        let media_manager = self.media_manager()?;
        let popup_id = self.next_popup_id();

        let physical_size = LogicalSize::new(window_opts.width, window_opts.height)
            .to_physical::<u32>(window_opts.monitor.scale_factor);

        // Enqueues the decode and returns immediately — the media manager thread delivers the
        // result later via `UserEvent::ImageResolved`, handled in `handle_image_resolved`.
        media_manager.get_image_data(popup_id, media_id, physical_size.width, physical_size.height)?;

        let props = Self::window_props(popup_id, &window_opts);
        self.windows.insert(
            popup_id,
            PopupSlot::Pending(PendingWindow {
                kind: PendingKind::Image,
                visible: window_opts.visible,
                opacity: window_opts.opacity,
                title: window_opts.title.clone(),
                pending_move: None,
                pending_fade: None,
                video_paused: false,
                opts: window_opts,
            }),
        );

        Ok(props)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_video(
        &mut self,
        media_id: u64,
        width: u32,
        height: u32,
        loop_video: bool,
        audio: bool,
        opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        // Whether the window needs transparency is already fully resolved on the Lua side
        // (`spawn_video_popup` knows the media's `transparent` flag from `Media.media_data`
        // without needing a decode), so `opts.transparent` is only `None` here when no
        // transparency is actually needed.
        let transparent = opts.transparent.unwrap_or(false);
        let window_opts = self.resolve_window_opts(
            opts,
            WindowSizeBehaviour::ResizeWithMedia { width, height },
            true,
            transparent,
            event_loop,
        )?;

        let media_manager = self.media_manager()?;
        let popup_id = self.next_popup_id();

        // As with images, this just enqueues the request — see `UserEvent::VideoResolved` /
        // `handle_video_resolved`.
        media_manager.get_video_data(popup_id, media_id, loop_video, audio)?;

        let props = Self::window_props(popup_id, &window_opts);
        self.windows.insert(
            popup_id,
            PopupSlot::Pending(PendingWindow {
                kind: PendingKind::Video { loop_video },
                visible: window_opts.visible,
                opacity: window_opts.opacity,
                title: window_opts.title.clone(),
                pending_move: None,
                pending_fade: None,
                video_paused: false,
                opts: window_opts,
            }),
        );

        Ok(props)
    }

    fn spawn_prompt(
        &mut self,
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
        window_opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let auto_transparent = window_opts.opacity.is_some_and(|o| o < 1.0);
        let transparent = window_opts.transparent.unwrap_or(auto_transparent);
        let resolved = self.resolve_window_opts(
            window_opts,
            WindowSizeBehaviour::UseDefaults {
                width: 400,
                height: 400,
            },
            transparent,
            transparent,
            event_loop,
        )?;
        let popup_id = self.next_popup_id();
        let props = Self::window_props(popup_id, &resolved);
        let window = self.create_window(popup_id, resolved, event_loop)?;
        let visible = props.visible;

        let mut prompt_window = PromptWindow::new(window, text, placeholder, initial_value)
            .map_err(LewdwareError::WindowError)?;

        if visible {
            if let Err(e) = prompt_window.inner_window.pre_show() {
                tracing::warn!("prompt pre-show failed: {e}");
            }
            prompt_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(popup_id, PopupSlot::Ready(WindowType::Prompt(prompt_window)));

        Ok(props)
    }

    fn spawn_choice(
        &mut self,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
        window_opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let auto_transparent = window_opts.opacity.is_some_and(|o| o < 1.0);
        let transparent = window_opts.transparent.unwrap_or(auto_transparent);
        let resolved = self.resolve_window_opts(
            window_opts,
            WindowSizeBehaviour::UseDefaults {
                width: 400,
                height: 400,
            },
            transparent,
            transparent,
            event_loop,
        )?;
        let popup_id = self.next_popup_id();
        let props = Self::window_props(popup_id, &resolved);
        let window = self.create_window(popup_id, resolved, event_loop)?;
        let visible = props.visible;

        let mut choice_window = ChoiceWindow::new(window, text, options)
            .map_err(LewdwareError::WindowError)?;

        if visible {
            if let Err(e) = choice_window.inner_window.pre_show() {
                tracing::warn!("choice pre-show failed: {e}");
            }
            choice_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(popup_id, PopupSlot::Ready(WindowType::Choice(choice_window)));

        Ok(props)
    }

    fn spawn_text(
        &mut self,
        text: String,
        style: TextStyle,
        window_opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        // Unlike other popup types, text defaults to a transparent (GPU-rendered) window, since
        // text is usually meant to float over the desktop rather than sit in an opaque panel.
        let transparent = window_opts.transparent.unwrap_or(true);
        let resolved = self.resolve_window_opts(
            window_opts,
            WindowSizeBehaviour::MeasureText {
                text: text.clone(),
                font: style.font,
                font_size: style.font_size,
                border_width: if style.border_color.is_some() {
                    style.border_width
                } else {
                    0.0
                },
            },
            transparent,
            transparent,
            event_loop,
        )?;
        // Resolve a percentage font size to a concrete point size now that the monitor (and so
        // its height, the basis for `FontSize::Percent`) is known. From here on `font_size` is
        // always `FontSize::Value`.
        let mut style = style;
        style.font_size = FontSize::Value(style.font_size.to_pixels(resolved.monitor.height));

        let popup_id = self.next_popup_id();
        let props = Self::window_props(popup_id, &resolved);
        let window = self.create_window(popup_id, resolved, event_loop)?;
        let visible = props.visible;

        let mut text_window =
            TextWindow::new(window, text, style).map_err(LewdwareError::WindowError)?;

        if visible {
            if let Err(e) = text_window.inner_window.pre_show() {
                tracing::warn!("choice pre-show failed: {e}");
            }
            text_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(popup_id, PopupSlot::Ready(WindowType::Text(text_window)));

        Ok(props)
    }

    fn spawn_audio(&mut self, media_id: u64, loop_audio: bool) -> u64 {
        let id = self.current_audio_id;
        self.current_audio_id += 1;

        // `media_manager()`/`get_audio_data` failing here is only reachable if the pack never
        // opened (no mode ever runs in that case) or the request channel is full — degrade
        // gracefully rather than panic: the id is still returned, but no slot is inserted, so
        // any later action on it is a harmless no-op (same as acting on an already-closed id).
        match self.media_manager().and_then(|media_manager| {
            media_manager.get_audio_data(id, media_id, loop_audio).map_err(Into::into)
        }) {
            Ok(()) => {
                self.audio_players.insert(id, AudioSlot::Pending { paused: false });
            }
            Err(err) => tracing::error!("Failed to request audio decode: {err}"),
        }

        id
    }

    fn set_wallpaper(&mut self, file: FileOrPath, mode: Option<WallpaperMode>) -> Result<()> {
        wallpaper::set_from_path(file.path().to_str().ok_or(LewdwareError::Internal(
            "Tempfile does not have valid UTF-8 path",
        ))?)
        .map_err(|err| LewdwareError::WallpaperError(anyhow!("{err}")))?;

        if let Some(mode) = mode {
            let mode = match mode {
                WallpaperMode::Center => wallpaper::Mode::Center,
                WallpaperMode::Crop => wallpaper::Mode::Crop,
                WallpaperMode::Fit => wallpaper::Mode::Fit,
                WallpaperMode::Span => wallpaper::Mode::Span,
                WallpaperMode::Stretch => wallpaper::Mode::Stretch,
                WallpaperMode::Tile => wallpaper::Mode::Tile,
            };

            wallpaper::set_mode(mode)
                .map_err(|err| LewdwareError::WallpaperError(anyhow!("{err}")))?;
        }

        Ok(())
    }

    fn reset_wallpaper(&self) {
        if let Some(wallpaper) = &self.default_wallpaper {
            if let Err(err) = wallpaper::set_from_path(wallpaper) {
                tracing::error!("Error setting wallpaper back to default: {}", err);
            }
        } else {
            tracing::error!("No default wallpaper found; leaving wallpaper as is");
        }
    }

    fn open_link(&self, url: String) -> Result<()> {
        let url = Url::parse(&url).map_err(|err| LewdwareError::OpenLinkError(err.into()))?;

        if url.scheme() != "https" {
            return Err(LewdwareError::OpenLinkError(anyhow!(
                "Only https:// links are permitted"
            )));
        }

        if !matches!(url.host(), Some(Host::Domain(_))) {
            return Err(LewdwareError::OpenLinkError(anyhow!(
                "IP addresses are not allowed"
            )));
        }

        if !url.username().is_empty() || url.password().is_some() {
            return Err(LewdwareError::OpenLinkError(anyhow!(
                "URLs cannot contain a username or password"
            )));
        }

        webbrowser::open(url.as_str()).map_err(|err| LewdwareError::OpenLinkError(err.into()))
    }

    fn show_notification(&self, notification: Notification) -> Result<()> {
        let mut notification_builder = notify_rust::Notification::new();

        notification_builder.body(&notification.body);

        if let Some(summary) = notification.summary {
            notification_builder.summary(&summary);
        }

        notification_builder.show()?;

        Ok(())
    }

    fn process_lua_request(&mut self, request: LuaRequest, event_loop: &ActiveEventLoop) -> bool {
        if !match request {
            LuaRequest::SpawnImage {
                media_id,
                width,
                height,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_image(media_id, width, height, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnVideo {
                media_id,
                width,
                height,
                loop_video,
                audio,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_video(media_id, width, height, loop_video, audio, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnPrompt {
                text,
                placeholder,
                initial_value,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_prompt(text, placeholder, initial_value, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnChoice {
                text,
                options,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_choice(text, options, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnText {
                text,
                style,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_text(text, style, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnAudio {
                media_id,
                loop_audio,
                tx,
            } => tx.send(self.spawn_audio(media_id, loop_audio)).is_ok(),
            LuaRequest::SetWallpaper { file, mode, tx } => {
                tx.send(self.set_wallpaper(file, mode)).is_ok()
            }
            LuaRequest::ResetWallpaper { tx } => {
                self.reset_wallpaper();
                tx.send(())
            }.is_ok(),
            LuaRequest::OpenLink { url, tx } => tx.send(self.open_link(url)).is_ok(),
            LuaRequest::ShowNotification { notification, tx } => {
                tx.send(self.show_notification(notification)).is_ok()
            }
            LuaRequest::ListMonitors { tx } => tx.send(self.monitors.list(event_loop)).is_ok(),
            LuaRequest::PrimaryMonitor { tx } => tx
                .send(self.monitors.primary(event_loop).map_err(|err| err.into()))
                .is_ok(),
            LuaRequest::Exit { tx } => {
                let _ = tx.send(());
                event_loop.exit();
                return true;
            }
            LuaRequest::WindowAction { id, action } => {
                if let Entry::Occupied(mut entry) = self.windows.entry(id) {
                    match action {
                        WindowAction::CloseWindow { tx } => {
                            match entry.remove() {
                                PopupSlot::Ready(window_type) => self.close_window(window_type),
                                // No real window was ever created; synthesize the close event
                                // `InnerWindow::Drop` would otherwise have sent.
                                PopupSlot::Pending(_) => {
                                    if self
                                        .lua_event_tx
                                        .send(lua::Event::WindowClosed { id })
                                        .is_err()
                                    {
                                        tracing::debug!(
                                            "Couldn't send WindowClosed event: Lua thread has \
                                             shut down"
                                        );
                                    }
                                }
                            }
                            tx.send(()).is_ok()
                        }
                        WindowAction::PauseVideo { tx } => tx
                            .send(match entry.get_mut() {
                                PopupSlot::Ready(WindowType::Video(video_window)) => {
                                    video_window.pause();
                                    Ok(())
                                }
                                PopupSlot::Pending(PendingWindow {
                                    kind: PendingKind::Video { .. },
                                    video_paused,
                                    ..
                                }) => {
                                    *video_paused = true;
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::PlayVideo { tx } => tx
                            .send(match entry.get_mut() {
                                PopupSlot::Ready(WindowType::Video(video_window)) => {
                                    video_window.play();
                                    Ok(())
                                }
                                PopupSlot::Pending(PendingWindow {
                                    kind: PendingKind::Video { .. },
                                    video_paused,
                                    ..
                                }) => {
                                    *video_paused = false;
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::Move { id, tx, opts } => tx
                            .send(match entry.get_mut() {
                                PopupSlot::Ready(window_type) => {
                                    window_type.inner_window_mut().start_move(id, opts)
                                }
                                PopupSlot::Pending(pending) => {
                                    pending.pending_move = Some((id, opts));
                                    Ok(())
                                }
                            })
                            .is_ok(),
                        WindowAction::SetText { tx, text } => tx
                            .send(match entry.get_mut() {
                                PopupSlot::Ready(WindowType::Prompt(prompt)) => {
                                    prompt.set_text(text);
                                    Ok(())
                                }
                                PopupSlot::Ready(WindowType::Choice(choice)) => {
                                    choice.set_text(text);
                                    Ok(())
                                }
                                PopupSlot::Ready(WindowType::Text(text_window)) => match text {
                                    Some(text) => {
                                        text_window.set_text(text);
                                        Ok(())
                                    }
                                    None => Err(LewdwareError::Internal(
                                        "Text windows require non-nil text",
                                    )),
                                },
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::SetValue { tx, value } => tx
                            .send(match entry.get_mut() {
                                PopupSlot::Ready(WindowType::Prompt(prompt)) => {
                                    prompt.set_value(value);
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::SetOptions { tx, options } => tx
                            .send(match entry.get_mut() {
                                PopupSlot::Ready(WindowType::Choice(choice)) => {
                                    choice.set_options(options);
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::SetVisible { tx, visible } => {
                            match entry.get_mut() {
                                PopupSlot::Ready(window_type) => {
                                    window_type.inner_window().set_visible(visible)
                                }
                                PopupSlot::Pending(pending) => pending.visible = visible,
                            }
                            tx.send(()).is_ok()
                        }
                        WindowAction::SetTitle { tx, title } => {
                            match entry.get_mut() {
                                PopupSlot::Ready(window_type) => {
                                    window_type.inner_window_mut().set_title(title)
                                }
                                PopupSlot::Pending(pending) => pending.title = title,
                            }
                            tx.send(()).is_ok()
                        }
                        WindowAction::SetOpacity { tx, opacity } => {
                            let result = match entry.get_mut() {
                                PopupSlot::Ready(window_type) => {
                                    if opacity != 1.0 && !window_type.inner_window().transparent() {
                                        Err(LewdwareError::Internal(
                                            "Cannot change opacity on a non-transparent window. \
                                             Ensure the window is created with transparency \
                                             enabled: use media that has an alpha channel, set \
                                             `opacity` to a value below 1.0, or set \
                                             `transparent = true`.",
                                        ))
                                    } else {
                                        window_type.inner_window_mut().set_opacity(opacity);
                                        Ok(())
                                    }
                                }
                                PopupSlot::Pending(pending) => {
                                    if opacity != 1.0 && !pending.opts.transparent {
                                        Err(LewdwareError::Internal(
                                            "Cannot change opacity on a non-transparent window. \
                                             Ensure the window is created with transparency \
                                             enabled: use media that has an alpha channel, set \
                                             `opacity` to a value below 1.0, or set \
                                             `transparent = true`.",
                                        ))
                                    } else {
                                        pending.opacity = opacity;
                                        Ok(())
                                    }
                                }
                            };
                            tx.send(result).is_ok()
                        }
                        WindowAction::Fade { id, tx, opts } => {
                            let result = match entry.get_mut() {
                                PopupSlot::Ready(window_type) => {
                                    if !window_type.inner_window().transparent() {
                                        Err(LewdwareError::Internal(
                                            "Cannot fade a non-transparent window. Ensure the \
                                             window is created with transparency enabled: use \
                                             media that has an alpha channel, set `opacity` to \
                                             a value below 1.0, or set `transparent = true`.",
                                        ))
                                    } else {
                                        window_type.inner_window_mut().start_fade(id, opts)
                                    }
                                }
                                PopupSlot::Pending(pending) => {
                                    if !pending.opts.transparent {
                                        Err(LewdwareError::Internal(
                                            "Cannot fade a non-transparent window. Ensure the \
                                             window is created with transparency enabled: use \
                                             media that has an alpha channel, set `opacity` to \
                                             a value below 1.0, or set `transparent = true`.",
                                        ))
                                    } else {
                                        pending.pending_fade = Some((id, opts));
                                        Ok(())
                                    }
                                }
                            };
                            tx.send(result).is_ok()
                        }
                    }
                } else {
                    true
                }
            }
            LuaRequest::AudioAction { id, action } => {
                if let Entry::Occupied(mut entry) = self.audio_players.entry(id) {
                    match action {
                        AudioAction::Pause { tx } => {
                            match entry.get_mut() {
                                AudioSlot::Ready(player) => player.pause(),
                                AudioSlot::Pending { paused } => *paused = true,
                            }
                            tx.send(()).is_ok()
                        }
                        AudioAction::Play { tx } => {
                            match entry.get_mut() {
                                AudioSlot::Ready(player) => player.play(),
                                AudioSlot::Pending { paused } => *paused = false,
                            }
                            tx.send(()).is_ok()
                        }
                    }
                } else {
                    true
                }
            }
        } {
            tracing::error!("Couldn't send response");
        }

        false
    }

    fn process_lua_requests(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(request) = self.lua_request_rx.try_recv() {
            if self.process_lua_request(request, event_loop) {
                return;
            }
        }
    }

    /// A pending popup's media failed to decode, or its real window failed to build. Per
    /// design, this is treated exactly like the popup immediately closing: drop it and fire the
    /// `on_close` callback Lua would otherwise get from a real close.
    fn pending_window_failed(&mut self, id: PopupId, context: &str, err: impl std::fmt::Display) {
        tracing::error!("{context}: {err}");
        if self.lua_event_tx.send(lua::Event::WindowClosed { id }).is_err() {
            tracing::debug!("Couldn't send WindowClosed event: Lua thread has shut down");
        }
    }

    /// Apply the mutations Lua made to a popup while it was still pending (see
    /// [`PendingWindow`]) now that its real [`InnerWindow`] exists.
    fn apply_pending_mutations(
        inner_window: &mut InnerWindow,
        opacity: f32,
        title: Option<String>,
        pending_move: Option<(u64, MoveOpts)>,
        pending_fade: Option<(u64, FadeOpts)>,
    ) {
        inner_window.set_opacity(opacity);
        inner_window.set_title(title);
        if let Some((move_id, opts)) = pending_move {
            let _ = inner_window.start_move(move_id, opts);
        }
        if let Some((fade_id, opts)) = pending_fade {
            let _ = inner_window.start_fade(fade_id, opts);
        }
    }

    fn handle_image_resolved(
        &mut self,
        id: PopupId,
        result: std::result::Result<ImageData, MediaError>,
        event_loop: &ActiveEventLoop,
    ) {
        let Entry::Occupied(entry) = self.windows.entry(id) else {
            // Closed while the decode was in flight; nothing to do.
            return;
        };
        if !matches!(entry.get(), PopupSlot::Pending(_)) {
            return;
        }
        let PopupSlot::Pending(pending) = entry.remove() else {
            unreachable!()
        };

        let data = match result {
            Ok(data) => data,
            Err(err) => return self.pending_window_failed(id, "Failed to decode image", err),
        };

        let PendingWindow {
            opts,
            visible,
            opacity,
            title,
            pending_move,
            pending_fade,
            ..
        } = pending;

        let inner_window = match self.create_window(id, opts, event_loop) {
            Ok(w) => w,
            Err(err) => return self.pending_window_failed(id, "Failed to create image window", err),
        };

        let mut image_window = match ImageWindow::new(inner_window, data) {
            Ok(w) => w,
            Err(err) => {
                return self.pending_window_failed(id, "Failed to build image window", err);
            }
        };

        Self::apply_pending_mutations(
            &mut image_window.inner_window,
            opacity,
            title,
            pending_move,
            pending_fade,
        );

        // See `spawn_image`'s original comment: render while still offscreen so the compositor
        // has valid pixels before XMoveWindow fires.
        let idx = match image_window.draw() {
            Ok(idx) => idx,
            Err(e) => {
                tracing::warn!("image pre-draw failed: {e}");
                None
            }
        };
        image_window.inner_window.gpu_sync(idx);

        if visible {
            image_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(id, PopupSlot::Ready(WindowType::Image(image_window)));
    }

    fn handle_video_resolved(
        &mut self,
        id: PopupId,
        result: std::result::Result<VideoDecoder, MediaError>,
        event_loop: &ActiveEventLoop,
    ) {
        let Entry::Occupied(entry) = self.windows.entry(id) else {
            return;
        };
        if !matches!(entry.get(), PopupSlot::Pending(_)) {
            return;
        }
        let PopupSlot::Pending(pending) = entry.remove() else {
            unreachable!()
        };

        let video_player = match result {
            Ok(data) => data,
            Err(err) => return self.pending_window_failed(id, "Failed to decode video", err),
        };

        let PendingWindow {
            kind,
            opts,
            visible,
            opacity,
            title,
            pending_move,
            pending_fade,
            video_paused,
        } = pending;
        let PendingKind::Video { loop_video } = kind else {
            unreachable!("video resolve for a non-video pending slot")
        };

        let window = match self.create_window(id, opts, event_loop) {
            Ok(w) => w,
            Err(err) => return self.pending_window_failed(id, "Failed to create video window", err),
        };

        window.request_redraw();

        let mut video_window = match VideoWindow::new(window, video_player, loop_video) {
            Ok(w) => w,
            Err(err) => {
                return self.pending_window_failed(id, "Failed to build video window", err);
            }
        };

        Self::apply_pending_mutations(
            &mut video_window.inner_window,
            opacity,
            title,
            pending_move,
            pending_fade,
        );

        if video_paused {
            video_window.pause();
        }

        if visible {
            if let Err(e) = video_window.inner_window.pre_show() {
                tracing::warn!("video pre-show failed: {e}");
            }
            video_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(id, PopupSlot::Ready(WindowType::Video(video_window)));
    }

    fn handle_audio_resolved(
        &mut self,
        id: u64,
        result: std::result::Result<AudioPlayer, MediaError>,
    ) {
        let Entry::Occupied(entry) = self.audio_players.entry(id) else {
            return;
        };
        if !matches!(entry.get(), AudioSlot::Pending { .. }) {
            return;
        }
        let AudioSlot::Pending { paused } = entry.remove() else {
            unreachable!()
        };

        match result {
            Ok(audio_player) => {
                if !paused {
                    audio_player.play();
                }
                self.audio_players.insert(id, AudioSlot::Ready(audio_player));
            }
            Err(err) => {
                tracing::error!("Failed to decode audio: {err}");
                if self.lua_event_tx.send(lua::Event::AudioFinish { id }).is_err() {
                    tracing::debug!("Couldn't send AudioFinish event: Lua thread has shut down");
                }
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for LewdwareApp {
    fn resumed(&mut self, _: &ActiveEventLoop) {
        self.running = true;
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Only `Ready` slots ever get a real winit window (and hence a `WindowId` registered
        // here), so a hit always means an `Entry::Occupied` with a `PopupSlot::Ready` inside.
        let Some(&popup_id) = self.window_ids.get(&window_id) else {
            return;
        };

        if let Entry::Occupied(mut entry) = self.windows.entry(popup_id) {
            let PopupSlot::Ready(window_type) = entry.get_mut() else {
                return;
            };

            match window_type {
                WindowType::Image(window) => if event == WindowEvent::RedrawRequested
                    && let Err(err) = window.draw() {
                        tracing::error!("Error drawing image window: {}", err);
                    },
                // Video windows are driven directly from `about_to_wait` instead of through
                // `RedrawRequested` — see the comment there for why.
                WindowType::Video(_) => {}
                WindowType::Prompt(window) => match &event {
                    WindowEvent::RedrawRequested => {
                        window.render().unwrap_or_else(|err| {
                            tracing::error!("Error rendering prompt window: {}", err);
                        });
                    }
                    event => {
                        window.handle_event(event);
                    }
                },
                WindowType::Text(window) => match &event {
                    WindowEvent::RedrawRequested => {
                        window.render().unwrap_or_else(|err| {
                            tracing::error!("Error rendering text window: {}", err);
                        });
                    }
                    event => {
                        window.handle_event(event);
                    }
                },
                WindowType::Choice(window) => match &event {
                    WindowEvent::RedrawRequested => {
                        window.render().unwrap_or_else(|err| {
                            tracing::error!("Error rendering prompt window: {}", err);
                        });
                    }
                    event => {
                        window.handle_event(event);
                    }
                },
            }

            // Global event handling
            match event {
                WindowEvent::CloseRequested => {
                    if let PopupSlot::Ready(window_type) = entry.remove() {
                        self.close_window(window_type);
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let PopupSlot::Ready(window_type) = entry.get_mut() {
                        window_type.inner_window_mut().handle_cursor_moved(position);
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    if let PopupSlot::Ready(window_type) = entry.get_mut() {
                        window_type.inner_window_mut().handle_cursor_left();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    if let PopupSlot::Ready(window_type) = entry.get_mut() {
                        window_type.inner_window_mut().handle_mouse_down();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } => {
                    let should_close = match entry.get_mut() {
                        PopupSlot::Ready(window_type) => {
                            window_type.inner_window_mut().handle_mouse_up()
                        }
                        PopupSlot::Pending(_) => false,
                    };
                    if should_close
                        && let PopupSlot::Ready(window_type) = entry.remove()
                    {
                        self.close_window(window_type);
                    }
                }
                _ => {}
            }
        }
    }

    /// By user events we really mean custom events, which can be sent by code running outside the
    /// main event loop (e.g. on another thread).
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Exit => {
                event_loop.exit();
            }
            UserEvent::LuaRequest => {
                self.process_lua_requests(event_loop);
            }
            UserEvent::AudioFinish { id } => {
                if self.audio_players.remove(&id).is_some()
                    && let Err(err) = self.lua_event_tx.send(lua::Event::AudioFinish { id }) {
                        tracing::error!("{err}");
                    }
            }
            UserEvent::ImageResolved { id, result } => {
                self.handle_image_resolved(id, result, event_loop);
            }
            UserEvent::VideoResolved { id, result } => {
                self.handle_video_resolved(id, result, event_loop);
            }
            UserEvent::AudioResolved { id, result } => {
                self.handle_audio_resolved(id, result);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut moving_windows = false;
        let mut finished_videos = Vec::new();

        for (id, slot) in self.windows.iter_mut() {
            // Pending popups have no `InnerWindow` yet — nothing to animate/update until their
            // media resolves (see the `*Resolved` `UserEvent` handlers).
            let PopupSlot::Ready(window) = slot else {
                continue;
            };

            // Video windows are driven directly here rather than via `request_redraw()` /
            // `RedrawRequested`. On the Win32 backend, winit only reliably delivers
            // `RedrawRequested` to the last couple of windows that requested it within the same
            // `AboutToWait` cycle (https://github.com/rust-windowing/winit/issues/3648), so with
            // 3+ simultaneous video windows the rest would silently stop advancing.
            if let WindowType::Video(video_window) = window {
                match video_window.update() {
                    Ok(true) => finished_videos.push(*id),
                    Ok(false) => {}
                    Err(err) => tracing::error!("Error updating video window: {err}"),
                }
                // Keep polling continuously while any video window exists, since we can no
                // longer rely on `request_redraw()` to wake the loop back up for them.
                moving_windows = true;
            }

            if window.inner_window().is_moving() {
                window.inner_window_mut().update_position();
                moving_windows = true;
            }
            if window.inner_window().is_fading() {
                window.inner_window_mut().update_fade();
                moving_windows = true; // reusing `moving_windows` to mean "animating windows"
            }
        }

        for id in finished_videos {
            if let Some(PopupSlot::Ready(window_type)) = self.windows.remove(&id) {
                self.close_window(window_type);
            }
        }

        if moving_windows {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

impl Drop for LewdwareApp {
    fn drop(&mut self) {
        // Drop our own `MediaManager` clone *before* waiting for the Lua thread: the manager
        // thread's request channel only closes (letting it clean up its temp files and exit)
        // once every clone — ours and the Lua thread's — has been dropped.
        self.media_manager = None;

        // Blocks until the Lua thread actually finishes, so its temp files get a chance to be
        // cleaned up via `Drop` instead of being silently killed along with the process when
        // `main` returns. This also drops the Lua thread's own `MediaManager` clone.
        self.lua_thread_handle.shutdown();

        // Both `MediaManager` clones are gone now, so the media manager thread's request
        // channel has closed and it's finishing up (or already has). Join it so its temp files
        // are cleaned up before the process exits.
        if let Some(handle) = self.media_manager_handle.take()
            && handle.join().is_err()
        {
            tracing::error!("Media manager thread panicked");
        }

        if let Some(wallpaper) = &self.default_wallpaper {
            if let Err(err) = wallpaper::set_from_path(wallpaper) {
                tracing::error!("Error setting wallpaper back to default: {}", err);
            }
        } else {
            tracing::error!("No default wallpaper found; leaving wallpaper as is");
        }
    }
}

fn random_position(window_size: u32, total_size: u32) -> i32 {
    if window_size > total_size {
        0
    } else {
        random_range(0i32..=(total_size - window_size) as i32)
    }
}
