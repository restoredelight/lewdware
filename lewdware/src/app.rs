use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;

use anyhow::anyhow;
use rand::random_range;
use shared::user_config::AppConfig;
use url::{Host, Url};
use winit::dpi::LogicalPosition;
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
    self, AudioAction, ChoiceWindowOption, FontSize, LuaRequest, LuaThreadHandle, Notification,
    SpawnWindowOpts, TextFont, TextStyle, WallpaperMode, WindowAction, WindowProps,
    start_lua_thread,
};
use crate::media::{FileOrPath, ImageData};
use crate::monitor::Monitors;
use crate::utils::{calculate_media_popup_size, calculate_text_popup_size};
use crate::video::VideoDecoder;
use crate::wgpu::WgpuState;
use crate::window::{
    ChoiceWindow, HEADER_HEIGHT, ImageWindow, InnerWindow, PromptWindow, TextWindow, VideoWindow,
    WindowOpts, WindowPool, WindowType,
};

/// The main app.
/// * `windows`: A map containing all the windows spawned by the app. Since dropping a winit window
///   closes it, we can close windows by removing them from this map.
/// * `default_wallpaper`: Stores the user's default wallpaper, so we can restore it on panic.
pub struct LewdwareApp {
    running: bool,
    _config: Arc<AppConfig>,
    wgpu_state: Option<Arc<WgpuState>>,
    windows: HashMap<WindowId, WindowType>,
    audio_players: HashMap<u64, AudioPlayer>,
    current_audio_id: u64,
    default_wallpaper: Option<String>,
    lua_request_rx: tokio::sync::mpsc::Receiver<lua::LuaRequest>,
    lua_event_tx: tokio::sync::mpsc::UnboundedSender<lua::Event>,
    lua_thread_handle: LuaThreadHandle,
    monitors: Monitors,
    window_pool: WindowPool,
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

#[derive(Debug)]
pub enum UserEvent {
    Exit,
    LuaRequest,
    AudioFinish { id: u64 },
}

impl LewdwareApp {
    pub fn new(
        wgpu_state: Option<std::sync::Arc<WgpuState>>,
        event_loop_proxy: EventLoopProxy<UserEvent>,
        config: AppConfig,
    ) -> Result<Self> {
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

        let (lua_event_tx, lua_request_rx, lua_thread_handle) = start_lua_thread(
            event_loop_proxy,
            config.clone(),
            wgpu_state.as_ref().map(|s| s.device.clone()),
        );

        let monitors = Monitors::new(config.disabled_monitors.clone());

        Ok(Self {
            running: false,
            _config: config,
            wgpu_state: wgpu_state,
            windows: HashMap::new(),
            audio_players: HashMap::new(),
            current_audio_id: 0,
            default_wallpaper: wallpaper,
            lua_request_rx,
            lua_event_tx,
            lua_thread_handle,
            monitors,
            window_pool: WindowPool::new(),
        })
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
    /// [`InnerWindow`]. Returns the window handle and the [`WindowProps`] for Lua.
    fn create_window(
        &mut self,
        opts: WindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<(InnerWindow, WindowProps)> {
        let window = self
            .window_pool
            .acquire(&opts, event_loop)
            .map_err(LewdwareError::WindowError)?;

        let _ = window.set_cursor_hittest(!opts.click_through);

        let window_id = window.id();
        let props = WindowProps {
            window_id,
            width: opts.width,
            height: opts.height,
            outer_width: opts.outer_width,
            outer_height: opts.outer_height,
            x: opts.x,
            y: opts.y,
            monitor: opts.monitor.clone(),
            visible: opts.visible,
        };

        let inner_window = InnerWindow::new(
            window,
            &opts,
            self.wgpu_state.clone(),
            self.lua_event_tx.clone(),
        )
        .map_err(LewdwareError::WindowError)?;

        Ok((inner_window, props))
    }

    /// Release a window back to the pool. Moving offscreen rather than unmapping avoids
    /// the KWin strut relayout freeze on Dock-type windows.
    fn close_window(&mut self, window_type: WindowType) {
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
        data: ImageData,
        opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        tracing::info!("Windows: {}", self.windows.len());
        let transparent = opts.transparent.unwrap_or(false);
        let window_opts = self.resolve_window_opts(
            opts,
            WindowSizeBehaviour::ResizeWithMedia {
                width: data.width(),
                height: data.height(),
            },
            transparent,
            transparent,
            event_loop,
        )?;
        let (inner_window, props) = self.create_window(window_opts, event_loop)?;
        let visible = props.visible;

        let mut image_window =
            ImageWindow::new(inner_window, data).map_err(|err| LewdwareError::WindowError(err))?;

        // Render the image while still offscreen so the compositor has valid pixels before
        // XMoveWindow fires. For CPU (softbuffer) windows, X11 protocol ordering guarantees
        // XShmPutImage is processed before XMoveWindow. For GPU windows, pre_show() submits a
        // clear and blocks until the DRI3 present lands, so XMoveWindow follows in the X11 stream.
        let idx = match image_window.draw() {
            Ok(idx) => idx,
            Err(e) => {
                tracing::warn!("image pre-draw failed: {e}");
                None
            }
        };
        // gpu_sync blocks on the specific submission so XMoveWindow follows the DRI3 present.
        image_window.inner_window.gpu_sync(idx);

        if visible {
            image_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(props.window_id.clone(), WindowType::Image(image_window));

        Ok(props)
    }

    fn spawn_video(
        &mut self,
        video_player: VideoDecoder,
        loop_video: bool,
        opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let auto_transparent =
            video_player.packed_alpha() || opts.opacity.map_or(false, |o| o < 1.0);
        let transparent = opts.transparent.unwrap_or(auto_transparent);
        let window_opts = self.resolve_window_opts(
            opts,
            WindowSizeBehaviour::ResizeWithMedia {
                width: video_player.width() as u32,
                height: video_player.height() as u32,
            },
            true,
            transparent,
            event_loop,
        )?;
        let (window, props) = self.create_window(window_opts, event_loop)?;
        let visible = props.visible;

        window.request_redraw();

        let mut video_window = VideoWindow::new(window, video_player, loop_video)
            .map_err(|err| LewdwareError::WindowError(err))?;

        if visible {
            if let Err(e) = video_window.inner_window.pre_show() {
                tracing::warn!("video pre-show failed: {e}");
            }
            video_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(props.window_id.clone(), WindowType::Video(video_window));

        tracing::info!("{}", self.windows.len());

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
        let auto_transparent = window_opts.opacity.map_or(false, |o| o < 1.0);
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
        let (window, props) = self.create_window(resolved, event_loop)?;
        let visible = props.visible;

        let mut prompt_window = PromptWindow::new(window, text, placeholder, initial_value)
            .map_err(|err| LewdwareError::WindowError(err))?;

        if visible {
            if let Err(e) = prompt_window.inner_window.pre_show() {
                tracing::warn!("prompt pre-show failed: {e}");
            }
            prompt_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(props.window_id.clone(), WindowType::Prompt(prompt_window));

        Ok(props)
    }

    fn spawn_choice(
        &mut self,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
        window_opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let auto_transparent = window_opts.opacity.map_or(false, |o| o < 1.0);
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
        let (window, props) = self.create_window(resolved, event_loop)?;
        let visible = props.visible;

        let mut choice_window = ChoiceWindow::new(window, text, options)
            .map_err(|err| LewdwareError::WindowError(err))?;

        if visible {
            if let Err(e) = choice_window.inner_window.pre_show() {
                tracing::warn!("choice pre-show failed: {e}");
            }
            choice_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(props.window_id.clone(), WindowType::Choice(choice_window));

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

        let (window, props) = self.create_window(resolved, event_loop)?;
        let visible = props.visible;

        let mut text_window =
            TextWindow::new(window, text, style).map_err(|err| LewdwareError::WindowError(err))?;

        if visible {
            if let Err(e) = text_window.inner_window.pre_show() {
                tracing::warn!("choice pre-show failed: {e}");
            }
            text_window.inner_window.set_visible(true);
        }

        self.windows
            .insert(props.window_id.clone(), WindowType::Text(text_window));

        Ok(props)
    }

    fn spawn_audio(&mut self, audio_player: AudioPlayer) -> u64 {
        let id = self.current_audio_id;
        self.current_audio_id += 1;

        audio_player.play();
        self.audio_players.insert(id, audio_player);

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
                data,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_image(data, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnVideo {
                video_player: data,
                loop_video,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_video(data, loop_video, window_opts, event_loop))
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
                audio_player: data,
                tx,
            } => tx.send(self.spawn_audio(data)).is_ok(),
            LuaRequest::SetWallpaper { file, mode, tx } => {
                tx.send(self.set_wallpaper(file, mode)).is_ok()
            }
            LuaRequest::ResetWallpaper { tx } => tx.send(self.reset_wallpaper()).is_ok(),
            LuaRequest::OpenLink { url, tx } => tx.send(self.open_link(url)).is_ok(),
            LuaRequest::ShowNotification { notification, tx } => {
                tx.send(self.show_notification(notification)).is_ok()
            }
            LuaRequest::ListMonitors { tx } => tx.send(self.monitors.list(event_loop)).is_ok(),
            LuaRequest::PrimaryMonitor { tx } => tx
                .send(self.monitors.primary(event_loop).map_err(|err| err.into()))
                .is_ok(),
            LuaRequest::GetMonitor { id, tx } => tx
                .send(self.monitors.get(id, event_loop).map_err(|err| err.into()))
                .is_ok(),
            LuaRequest::RandomMonitor { tx } => tx
                .send(self.monitors.random(event_loop).map_err(|err| err.into()))
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
                            let window_type = entry.remove();
                            self.close_window(window_type);
                            tx.send(()).is_ok()
                        }
                        WindowAction::PauseVideo { tx } => tx
                            .send(match entry.get_mut() {
                                WindowType::Video(video_window) => {
                                    video_window.pause();
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::PlayVideo { tx } => tx
                            .send(match entry.get_mut() {
                                WindowType::Video(video_window) => {
                                    video_window.play();
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::Move { id, tx, opts } => tx
                            .send(entry.get_mut().inner_window_mut().start_move(id, opts))
                            .is_ok(),
                        WindowAction::SetText { tx, text } => tx
                            .send(match entry.get_mut() {
                                WindowType::Prompt(prompt) => {
                                    prompt.set_text(text);
                                    Ok(())
                                }
                                WindowType::Choice(choice) => {
                                    choice.set_text(text);
                                    Ok(())
                                }
                                WindowType::Text(text_window) => match text {
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
                                WindowType::Prompt(prompt) => {
                                    prompt.set_value(value);
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::SetOptions { tx, options } => tx
                            .send(match entry.get_mut() {
                                WindowType::Choice(choice) => {
                                    choice.set_options(options);
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::SetVisible { tx, visible } => tx
                            .send(entry.get().inner_window().set_visible(visible))
                            .is_ok(),
                        WindowAction::SetTitle { tx, title } => tx
                            .send(entry.get_mut().inner_window_mut().set_title(title))
                            .is_ok(),
                        WindowAction::SetOpacity { tx, opacity } => {
                            let window = entry.get_mut();
                            let result = if opacity != 1.0 && !window.inner_window().transparent() {
                                Err(LewdwareError::Internal(
                                    "Cannot change opacity on a non-transparent window. \
                                     Ensure the window is created with transparency enabled: \
                                     use media that has an alpha channel, set `opacity` to a \
                                     value below 1.0, or set `transparent = true`.",
                                ))
                            } else {
                                window.inner_window_mut().set_opacity(opacity);
                                Ok(())
                            };
                            tx.send(result).is_ok()
                        }
                        WindowAction::Fade { id, tx, opts } => {
                            let window = entry.get_mut();
                            let result = if !window.inner_window().transparent() {
                                Err(LewdwareError::Internal(
                                    "Cannot fade a non-transparent window. \
                                     Ensure the window is created with transparency enabled: \
                                     use media that has an alpha channel, set `opacity` to a \
                                     value below 1.0, or set `transparent = true`.",
                                ))
                            } else {
                                window.inner_window_mut().start_fade(id, opts)
                            };
                            tx.send(result).is_ok()
                        }
                    }
                } else {
                    true
                }
            }
            LuaRequest::AudioAction { id, action } => {
                if let Entry::Occupied(entry) = self.audio_players.entry(id) {
                    match action {
                        AudioAction::Pause { tx } => tx.send(entry.get().pause()).is_ok(),
                        AudioAction::Play { tx } => tx.send(entry.get().play()).is_ok(),
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
        if let Entry::Occupied(mut entry) = self.windows.entry(window_id) {
            match entry.get_mut() {
                WindowType::Image(window) => match event {
                    WindowEvent::RedrawRequested => {
                        if let Err(err) = window.draw() {
                            tracing::error!("Error drawing image window: {}", err);
                        }
                    }
                    _ => {}
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
                    let window_type = entry.remove();
                    self.close_window(window_type);
                }
                WindowEvent::CursorMoved { position, .. } => {
                    entry
                        .get_mut()
                        .inner_window_mut()
                        .handle_cursor_moved(position);
                }
                WindowEvent::CursorLeft { .. } => {
                    entry.get_mut().inner_window_mut().handle_cursor_left();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    entry.get_mut().inner_window_mut().handle_mouse_down();
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } => {
                    if entry.get_mut().inner_window_mut().handle_mouse_up() {
                        let window_type = entry.remove();
                        self.close_window(window_type);
                        return;
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
                if self.audio_players.remove(&id).is_some() {
                    if let Err(err) = self.lua_event_tx.send(lua::Event::AudioFinish { id }) {
                        tracing::error!("{err}");
                    }
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut moving_windows = false;
        let mut finished_videos = Vec::new();

        for (id, window) in self.windows.iter_mut() {
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
            if let Some(window_type) = self.windows.remove(&id) {
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
        // Blocks until the Lua thread (and the media manager thread it owns) actually finish,
        // so their temp files (extracted pack index, any in-flight media) get cleaned up via
        // `Drop` instead of being silently killed along with the process when `main` returns.
        self.lua_thread_handle.shutdown();

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
