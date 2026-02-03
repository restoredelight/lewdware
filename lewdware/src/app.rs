use std::collections::HashMap;
use std::collections::hash_map::Entry;

use anyhow::anyhow;
use rand::random_range;
use shared::user_config::AppConfig;
use url::{Host, Url};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::MouseButton;
use winit::event_loop::{ControlFlow, EventLoopProxy};
use winit::window::{WindowAttributes, WindowLevel};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use crate::audio::AudioPlayer;
use crate::egui::WgpuState;
use crate::error::{LewdwareError, MonitorError, Result};
use crate::header::HEADER_HEIGHT;
use crate::lua::{
    self, AudioAction, ChoiceWindowOption, LuaRequest, Notification, SpawnWindowOpts,
    WallpaperMode, WindowAction, WindowProps, start_lua_thread,
};
use crate::media::{FileOrPath, ImageData, VideoData};
use crate::monitor::Monitors;
use crate::utils::calculate_media_popup_size;
use crate::window::{
    ChoiceWindow, ImageWindow, InnerWindow, PromptWindow, VideoWindow, WindowType,
};

/// The main app.
/// * `windows`: A map containing all the windows spawned by the app. Since dropping a winit window
///   closes it, we can close windows by removing them from this map.
/// * `default_wallpaper`: Stores the user's default wallpaper, so we can restore it on panic.
pub struct ChaosApp<'a> {
    running: bool,
    config: AppConfig,
    wgpu_state: WgpuState,
    windows: HashMap<WindowId, WindowType<'a>>,
    audio_players: HashMap<u64, AudioPlayer>,
    current_audio_id: u64,
    default_wallpaper: Option<String>,
    lua_request_rx: tokio::sync::mpsc::Receiver<lua::LuaRequest>,
    lua_event_tx: tokio::sync::mpsc::UnboundedSender<lua::Event>,
    monitors: Monitors,
}

enum WindowSizeBehaviour {
    ResizeWithMedia { width: u32, height: u32 },
    UseDefaults { width: u32, height: u32 },
}

#[derive(Debug)]
pub enum UserEvent {
    Exit,
    LuaRequest,
}

impl<'a> ChaosApp<'a> {
    pub fn new(
        wgpu_state: WgpuState,
        event_loop_proxy: EventLoopProxy<UserEvent>,
        config: AppConfig,
    ) -> Result<Self> {
        let wallpaper = match wallpaper::get() {
            Ok(wallpaper) => Some(wallpaper),
            Err(err) => {
                eprintln!("Error getting wallpaper: {}", err);
                None
            }
        };

        println!("{:?}", config);

        let (lua_event_tx, lua_request_rx) = start_lua_thread(
            event_loop_proxy,
            r#"
                x = {}
                while true do
                    table.insert(x, 1)
                end
                local windows = {}
                local interval
                interval = lewdware.every(1000, function()
                    print("Spawning window", #windows + 1)

                    local media = lewdware.media.random({ type = { "image", "video" } })
                    local window
                    if media.type == "image" then
                        window = lewdware.spawn_image_popup(media)
                    elseif media.type == "video" then
                        window = lewdware.spawn_video_popup(media)
                    end

                    table.insert(windows, window)
                    interval:set_duration(0)
                    interval:set_duration(interval.duration * 0.99)
                    print(interval.duration)
                end)
            "#
            .to_string(),
            config.clone(),
        );

        Ok(Self {
            running: false,
            config,
            wgpu_state: wgpu_state,
            windows: HashMap::new(),
            audio_players: HashMap::new(),
            current_audio_id: 0,
            default_wallpaper: wallpaper,
            lua_request_rx,
            lua_event_tx,
            monitors: Monitors::new(),
        })
    }

    fn create_window(
        &mut self,
        opts: SpawnWindowOpts,
        size_behaviour: WindowSizeBehaviour,
        gpu: bool,
        event_loop: &ActiveEventLoop,
    ) -> Result<(InnerWindow<'a>, WindowProps)> {
        let monitor_info = match opts.monitor {
            Some(x) => x,
            None => self.monitors.random(event_loop)?,
        };

        let monitor = self
            .monitors
            .get_handle(monitor_info.id, event_loop)
            .ok_or(MonitorError::MonitorNotFound)?;

        let scale_factor = monitor.scale_factor();
        let monitor_size = monitor.size().to_logical(scale_factor);
        let monitor_position: LogicalPosition<u32> = monitor.position().to_logical(scale_factor);

        let (width, height) = match size_behaviour {
            WindowSizeBehaviour::ResizeWithMedia { width, height } => calculate_media_popup_size(
                opts.width,
                opts.height,
                width,
                height,
                monitor_size.width,
                monitor_size.height,
            ),
            WindowSizeBehaviour::UseDefaults { width, height } => (
                opts.width
                    .map(|width| width.to_pixels(monitor_size.width))
                    .unwrap_or(width),
                opts.height
                    .map(|height| height.to_pixels(monitor_size.height))
                    .unwrap_or(height),
            ),
        };

        let (mut outer_width, mut outer_height) = (width, height);

        if opts.decorations {
            outer_width += 2;
            outer_height += HEADER_HEIGHT + 2;
        }

        let x = opts
            .x
            .map(|coord| {
                opts.anchor
                    .resolve(coord.to_pixels(monitor_size.width), outer_width)
            })
            .unwrap_or_else(|| random_position(outer_width, monitor_size.width));
        let y = opts
            .y
            .map(|coord| {
                opts.anchor
                    .resolve(coord.to_pixels(monitor_size.height), outer_height)
            })
            .unwrap_or_else(|| random_position(outer_height, monitor_size.height));

        let position = LogicalPosition::new(monitor_position.x + x, monitor_position.y + y);

        let mut attrs = WindowAttributes::default()
            .with_title("Chaos Window")
            .with_position(position)
            .with_inner_size(LogicalSize::new(outer_width, outer_height))
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_resizable(false)
            .with_visible(false);

        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::{WindowAttributesExtX11, WindowType};

            attrs = attrs.with_x11_window_type(vec![WindowType::Notification]);
        }

        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowAttributesExtWindows;

            attrs = attrs.with_skip_taskbar(true);
        }

        let window = event_loop
            .create_window(attrs)
            .map_err(|err| LewdwareError::WindowError(err.into()))?;

        // If we don't do this, then on Windows, the window flashes on screen with decorations and the
        // wrong size/position, before going to the correct place. I think this is because winit
        // creates the window, then sends requests to change the size, position and borders.
        // See https://github.com/rust-windowing/winit/issues/4116
        window.set_visible(true);

        let props = WindowProps {
            window_id: window.id(),
            width,
            height,
            x,
            y,
            outer_width,
            outer_height,
            monitor: monitor_info,
        };

        let inner_window = InnerWindow::new(
            window,
            &self.wgpu_state,
            opts.decorations,
            gpu,
            LogicalPosition::new(x, y),
            self.lua_event_tx.clone(),
        )
        .map_err(|err| LewdwareError::WindowError(err))?;

        Ok((inner_window, props))
    }

    fn spawn_image(
        &mut self,
        data: ImageData,
        opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let (window, props) = self.create_window(
            opts,
            WindowSizeBehaviour::ResizeWithMedia {
                width: data.width(),
                height: data.height(),
            },
            false,
            event_loop,
        )?;

        window.request_redraw();

        let image_window =
            ImageWindow::new(window, data).map_err(|err| LewdwareError::WindowError(err))?;

        self.windows
            .insert(props.window_id.clone(), WindowType::Image(image_window));

        Ok(props)
    }

    fn spawn_video(
        &mut self,
        data: VideoData,
        loop_video: bool,
        audio: bool,
        opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let (window, props) = self.create_window(
            opts,
            WindowSizeBehaviour::ResizeWithMedia { width: data.width, height: data.height },
            true,
            event_loop,
        )?;

        window.request_redraw();

        let video_window = VideoWindow::new(
            window,
            data,
            props.width,
            props.height,
            audio,
            loop_video
        )
        .map_err(|err| LewdwareError::WindowError(err))?;

        self.windows
            .insert(props.window_id.clone(), WindowType::Video(video_window));

        Ok(props)
    }

    fn spawn_prompt(
        &mut self,
        title: Option<String>,
        text: Option<String>,
        placeholder: Option<String>,
        initial_value: Option<String>,
        window_opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let (window, props) = self.create_window(
            window_opts,
            WindowSizeBehaviour::UseDefaults {
                width: 400,
                height: 400,
            },
            false,
            event_loop,
        )?;

        let prompt_window = PromptWindow::new(
            window,
            title,
            text,
            placeholder,
            initial_value,
        )
        .map_err(|err| LewdwareError::WindowError(err))?;

        self.windows
            .insert(props.window_id.clone(), WindowType::Prompt(prompt_window));

        Ok(props)
    }

    fn spawn_choice(
        &mut self,
        title: Option<String>,
        text: Option<String>,
        options: Vec<ChoiceWindowOption>,
        window_opts: SpawnWindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let (window, props) = self.create_window(
            window_opts,
            WindowSizeBehaviour::UseDefaults {
                width: 400,
                height: 400,
            },
            false,
            event_loop,
        )?;

        let prompt_window = ChoiceWindow::new(
            window,
            title,
            text,
            options,
        )
        .map_err(|err| LewdwareError::WindowError(err))?;

        self.windows
            .insert(props.window_id.clone(), WindowType::Choice(prompt_window));

        Ok(props)
    }

    fn spawn_audio(&mut self, data: FileOrPath, loop_audio: bool) -> u64 {
        let audio_player = AudioPlayer::new(data, loop_audio);

        let id = self.current_audio_id;
        self.current_audio_id += 1;

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

    fn open_link(&self, url: String) -> Result<()> {
        let url = Url::parse(&url).map_err(|err| LewdwareError::OpenLinkError(err.into()))?;

        if url.scheme() != "https" {
            return Err(LewdwareError::OpenLinkError(anyhow!("Only https:// links are permitted")));
        }

        if !matches!(url.host(), Some(Host::Domain(_))) {
            return Err(LewdwareError::OpenLinkError(anyhow!("IP addresses are not allowed")));
        }

        if !url.username().is_empty() || url.password().is_some() {
            return Err(LewdwareError::OpenLinkError(anyhow!("URLs cannot contain a username or password")));
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

    // fn pause_video(window: Window) -> Result<()> {
    //     match window {
    //         Window::Video(video) => video
    //     }
    // }

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
                data,
                loop_video,
                audio,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_video(
                    data,
                    loop_video,
                    audio,
                    window_opts,
                    event_loop,
                ))
                .is_ok(),
            LuaRequest::SpawnPrompt {
                title,
                text,
                placeholder,
                initial_value,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_prompt(
                    title,
                    text,
                    placeholder,
                    initial_value,
                    window_opts,
                    event_loop,
                ))
                .is_ok(),
            LuaRequest::SpawnChoice {
                title,
                text,
                options,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_choice(title, text, options, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnAudio {
                data,
                loop_audio,
                tx,
            } => tx.send(self.spawn_audio(data, loop_audio)).is_ok(),
            LuaRequest::SetWallpaper { file, mode, tx } => {
                tx.send(self.set_wallpaper(file, mode)).is_ok()
            }
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
                            entry.remove();
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
                        WindowAction::SetTitle { tx, title } => tx
                            .send(match entry.get_mut() {
                                WindowType::Prompt(prompt) => {
                                    prompt.set_title(title);
                                    Ok(())
                                }
                                WindowType::Choice(choice) => {
                                    choice.set_title(title);
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
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
            eprintln!("Couldn't send response");
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

impl<'a> ApplicationHandler<UserEvent> for ChaosApp<'a> {
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
                    WindowEvent::RedrawRequested => window.draw().unwrap_or_else(|err| {
                        eprintln!("Error drawing image window: {}", err);
                    }),
                    _ => {}
                },
                WindowType::Video(window) => match event {
                    WindowEvent::RedrawRequested => match window.update() {
                        Err(err) => {
                            eprintln!("Error updating video window: {err}");
                        }
                        Ok(true) => {
                            entry.remove();
                            return;
                        }
                        Ok(false) => {}
                    },
                    _ => {}
                },
                WindowType::Prompt(window) => match &event {
                    WindowEvent::RedrawRequested => {
                        window.render().unwrap_or_else(|err| {
                            eprintln!("Error rendering prompt window: {}", err);
                        });
                    }
                    event => {
                        window.handle_event(event);
                    }
                },
                WindowType::Choice(window) => match &event {
                    WindowEvent::RedrawRequested => {
                        window.render().unwrap_or_else(|err| {
                            eprintln!("Error rendering prompt window: {}", err);
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
                    entry.remove();
                },
                WindowEvent::CursorMoved { position, .. } => {
                    entry.get_mut().inner_window_mut().handle_cursor_moved(position);
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
                        entry.remove();
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
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut moving_windows = false;
        for window in self.windows.values_mut() {
            match window {
                WindowType::Video(_) => {
                    window.inner_window().request_redraw();
                }
                _ => {}
            }

            if window.inner_window().is_moving() {
                window.inner_window_mut().update_position();
                moving_windows = true;
            }
        }

        if moving_windows {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

impl Drop for ChaosApp<'_> {
    fn drop(&mut self) {
        if let Some(wallpaper) = &self.default_wallpaper {
            if let Err(err) = wallpaper::set_from_path(wallpaper) {
                eprintln!("Error setting wallpaper back to default: {}", err);
            }
        } else {
            eprintln!("No default wallpaper found; leaving wallpaper as is");
        }
    }
}

// fn window_props(window: &winit::window::Window) -> Result<WindowProps> {
//     let scale_factor = window.scale_factor();
//     let size = window.inner_size().to_logical(scale_factor);
//     let position = window.inner_position()?.to_logical(scale_factor);
//
//     Ok(WindowProps {
//         width: size.width,
//         height: size.height,
//         x: position.x,
//         y: position.y,
//         window_id: window.id(),
//     })
// }

fn random_position(window_size: u32, total_size: u32) -> u32 {
    if window_size > total_size {
        0
    } else {
        random_range(0..=(total_size - window_size))
    }
}
