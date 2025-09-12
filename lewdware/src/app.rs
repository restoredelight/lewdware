use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Result, anyhow};
use async_channel::{Receiver, Sender, TryRecvError};
use futures_lite::future::block_on;
use notify_rust::Notification;
use pack_format::config::Metadata;
use rand::seq::IndexedRandom;
use rand::{random_bool, random_range};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::MouseButton;
use winit::event_loop::{ControlFlow, EventLoop};
use winit::monitor::MonitorHandle;
use winit::window::WindowLevel;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{WindowAttributes, WindowId},
};

use crate::audio::AudioPlayer;
use crate::config::AppConfig;
use crate::egui::WgpuState;
use crate::media::{self, Media, MediaRequest, MediaResponse, spawn_media_manager_thread};
use crate::transition::TransitionManager;
use crate::window::{ImageWindow, PromptWindow, VideoWindow};

pub struct ChaosApp<'a> {
    config: AppConfig,
    metadata: Metadata,
    wgpu_state: WgpuState,
    windows: HashMap<WindowId, Window<'a>>,
    running: Arc<AtomicBool>,
    last_spawn: Instant,
    audio_player: Option<AudioPlayer>,
    media_tx: Sender<MediaRequest>,
    media_rx: Receiver<MediaResponse>,
    loading_audio: bool,
    loading_prompt: bool,
    tags: Option<Vec<String>>,
    transition_manager: Option<TransitionManager>,
    wallpaper: Option<String>,
}

enum Window<'a> {
    Image(ImageWindow),
    Video(VideoWindow<'a>),
    Prompt(PromptWindow<'a>),
}

#[derive(Debug)]
pub enum UserEvent {
    MediaResponse,
}

impl<'a> ChaosApp<'a> {
    pub fn new(
        event_loop: &EventLoop<UserEvent>,
        config: AppConfig,
        running: Arc<AtomicBool>,
    ) -> Result<Self> {
        let (media_tx, media_rx, metadata) = spawn_media_manager_thread(event_loop)?;

        let transition = metadata.transition.as_ref().cloned();

        let wallpaper = match wallpaper::get() {
            Ok(wallpaper) => Some(wallpaper),
            Err(err) => {
                eprintln!("Error getting wallpaper: {}", err);
                None
            }
        };

        let wgpu_state = block_on(WgpuState::new());

        Ok(Self {
            config,
            metadata,
            wgpu_state,
            windows: HashMap::new(),
            running,
            last_spawn: Instant::now(),
            audio_player: None,
            media_tx,
            media_rx,
            loading_audio: false,
            loading_prompt: false,
            tags: None,
            transition_manager: transition
                .map(|transition| TransitionManager::new(transition.clone())),
            wallpaper,
        })
    }

    fn process_media_response(
        &mut self,
        response: MediaResponse,
        event_loop: &ActiveEventLoop,
    ) -> Result<()> {
        match response {
            MediaResponse::Media(media) => match media {
                Media::Image(image) => {
                    let window = create_window(event_loop, image.width(), image.height(), false)?;

                    window.request_redraw();

                    self.windows.insert(
                        window.id(),
                        Window::Image(ImageWindow::new(
                            window,
                            image,
                            self.config.close_button,
                            random_bool(self.config.moving_window_chance),
                        )?),
                    );
                }
                Media::Video(video) => {
                    let window =
                        create_window(event_loop, video.width as u32, video.height as u32, false)?;

                    self.windows.insert(
                        window.id(),
                        Window::Video(VideoWindow::new(
                            &self.wgpu_state,
                            window,
                            video,
                            self.config.close_button,
                            self.config.video_audio,
                        )?),
                    );
                }
            },
            MediaResponse::Audio(audio) => {
                self.audio_player = Some(AudioPlayer::new(audio)?);
                self.loading_audio = false;
            }
            MediaResponse::Notification(notification) => self.send_notification(notification),
            MediaResponse::Prompt(prompt) => {
                let window = create_window(event_loop, 400, 400, true)?;

                self.windows.insert(
                    window.id(),
                    Window::Prompt(PromptWindow::new(&self.wgpu_state, window, prompt.prompt)?),
                );

                self.loading_prompt = false;
            }
            MediaResponse::Link(link) => self.open_link(link.link),
        }

        Ok(())
    }

    fn request_audio(&mut self) -> bool {
        self.loading_audio = true;

        let tags = self.get_tags();

        self.media_tx
            .try_send(MediaRequest::RandomAudio { tags })
            .is_ok()
    }

    fn request_random_media(&mut self) -> bool {
        let tags = self.get_tags();

        self.media_tx
            .try_send(MediaRequest::RandomMedia {
                only_images: false,
                // only_images: self
                //     .windows
                //     .values()
                //     .filter(|window| matches!(window, Window::Video(_)))
                //     .count()
                //     >= self.config.max_videos,
                tags: None,
            })
            .is_ok()
    }

    fn request_link(&mut self) {
        let tags = self.get_tags();

        let _ = self.media_tx.try_send(MediaRequest::RandomLink { tags });
    }

    fn request_notification(&mut self) {
        let tags = self.get_tags();

        let _ = self
            .media_tx
            .try_send(MediaRequest::RandomNotification { tags });
    }

    fn request_prompt(&mut self) {
        let tags = self.get_tags();

        if self
            .media_tx
            .try_send(MediaRequest::RandomPrompt { tags })
            .is_ok()
        {
            self.loading_prompt = true;
        }
    }

    fn open_link(&self, link: String) {
        match webbrowser::open(&link) {
            Ok(_) => {}
            Err(err) => {
                eprintln!("Could not open link in web browser: {}", err);
            }
        }
    }

    fn send_notification(&self, notification_info: media::Notification) {
        let mut notification = Notification::new();

        notification.body(&notification_info.body);

        if let Some(summary) = notification_info.summary {
            notification.summary(&summary);
        }

        match notification.show() {
            Ok(_) => {}
            Err(err) => {
                eprintln!("Couldn't show notification: {}", err)
            }
        }
    }

    fn get_tags(&mut self) -> Option<Vec<String>> {
        match self.transition_manager.as_mut() {
            Some(transition_manager) => Some(
                transition_manager
                    .get_tags()
                    .into_iter()
                    .filter(|tag| self.tags.as_ref().is_none_or(|tags| tags.contains(tag)))
                    .collect::<Vec<_>>(),
            ),
            None => self.tags.as_ref().cloned(),
        }
    }
}

fn create_window(
    event_loop: &ActiveEventLoop,
    width: u32,
    height: u32,
    logical_size: bool,
) -> Result<winit::window::Window> {
    let monitor = random_monitor(event_loop);

    let position = if let Some(monitor) = monitor {
        let size = monitor.size();
        let monitor_position = monitor.position();
        let scale_factor = monitor.scale_factor();

        let (width, height) = if logical_size {
            let size = LogicalSize::new(width, height).to_physical(scale_factor);
            (size.width, size.height)
        } else {
            (width, height)
        };

        let position = random_window_position(width, height, size.width, size.height);

        PhysicalPosition::new(
            monitor_position.x as f32 + position.x,
            monitor_position.y as f32 + position.y,
        )
    } else {
        println!("Could not find a monitor, using default resolution");
        random_window_position(width, height, 1920, 1080)
    };

    let mut attrs = WindowAttributes::default()
        .with_title("Chaos Window")
        .with_position(position)
        .with_decorations(true)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_resizable(false);

    if logical_size {
        attrs = attrs.with_inner_size(LogicalSize::new(width, height));
    } else {
        attrs = attrs.with_inner_size(PhysicalSize::new(width, height));
    }

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::{WindowAttributesExtX11, WindowType};

        attrs = attrs.with_x11_window_type(vec![WindowType::Notification]);
    }

    #[cfg(target_os = "windows")]
    {
        attrs = attrs.with_skip_taskbar(true);
    }

    event_loop.create_window(attrs).map_err(|err| anyhow!(err))
}

impl<'a> ApplicationHandler<UserEvent> for ChaosApp<'a> {
    fn resumed(&mut self, _: &ActiveEventLoop) {
        self.request_random_media();

        self.request_audio();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Entry::Occupied(mut entry) = self.windows.entry(window_id) {
            match entry.get_mut() {
                Window::Image(window) => match event {
                    WindowEvent::CursorMoved { position, .. } => {
                        window.handle_cursor_moved(position);
                    }
                    WindowEvent::CursorLeft { .. } => {
                        window.handle_mouse_left_window();
                    }
                    WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                        ..
                    } => {
                        if window.handle_click() {
                            entry.remove();
                            return;
                        }
                    }
                    WindowEvent::RedrawRequested => window.draw().unwrap_or_else(|err| {
                        eprintln!("Error drawing image window: {}", err);
                    }),
                    _ => {}
                },
                Window::Video(window) => match event {
                    WindowEvent::CursorMoved { position, .. } => {
                        window.handle_cursor_moved(position);
                    }
                    WindowEvent::CursorLeft { .. } => {
                        window.handle_mouse_left_window();
                    }
                    WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                        ..
                    } => {
                        if window.handle_click() {
                            entry.remove();
                            return;
                        }
                    }
                    WindowEvent::RedrawRequested => {
                        if let Err(err) = window.update() {
                            eprintln!("Error updating video window: {}", err);
                        }
                    }
                    _ => {}
                },
                Window::Prompt(window) => match &event {
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
                }
                WindowEvent::KeyboardInput {
                    event:
                        KeyEvent {
                            state: ElementState::Pressed,
                            logical_key: Key::Named(NamedKey::Escape),
                            ..
                        },
                    ..
                } => {
                    self.running.store(false, Ordering::Relaxed);
                    event_loop.exit();
                }
                _ => {}
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::MediaResponse => loop {
                match self.media_rx.try_recv() {
                    Ok(message) => {
                        self.process_media_response(message, event_loop)
                            .unwrap_or_else(|err| {
                                eprintln!("Error: {}", err);
                            });
                    }
                    Err(err) => match err {
                        TryRecvError::Empty => break,
                        TryRecvError::Closed => {
                            eprintln!("Channel disconnected");
                            break;
                        }
                    },
                }
            },
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.running.load(Ordering::Relaxed) {
            event_loop.exit();
            return;
        }

        if self.last_spawn.elapsed() >= self.config.spawn_interval {
            if self.config.prompts
                && random_bool(1.0 / 5.0)
                && !self.loading_prompt
                && !self
                    .windows
                    .values()
                    .any(|window| matches!(window, Window::Prompt(_)))
            {
                self.request_prompt();
            } else {
                self.request_random_media();
            }

            self.last_spawn = Instant::now();

            if self.config.open_links && random_bool(1.0 / 10.0) {
                self.request_link();
            }

            if self.config.notifications && random_bool(1.0 / 10.0) {
                self.request_notification();
            }

            if !self.loading_audio
                && self
                    .audio_player
                    .as_ref()
                    .is_some_and(|player| player.is_finished())
                && random_bool(1.0 / 10.0)
            {
                self.request_audio();
            }
        }

        if let Some(duration) = self.config.window_duration {
            self.windows.retain(|_, window| match window {
                Window::Image(window) => window.created.elapsed() <= duration,
                Window::Video(window) => window.created.elapsed() <= duration && !window.closed(),
                Window::Prompt(window) => !window.closed(),
            });
        }

        let mut poll = false;
        for window in self.windows.values_mut() {
            match window {
                Window::Video(window) => {
                    window.window.request_redraw();
                    poll = true;
                }
                Window::Prompt(_) => {
                    poll = true;
                }
                Window::Image(window) => {
                    if window.moving {
                        if let Err(err) = window.update_position() {
                            eprintln!("Error moving window: {}", err);
                        }

                        poll = true;
                    }
                }
            }
        }

        if poll {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                self.last_spawn + self.config.spawn_interval,
            ));
        }
    }
}

impl Drop for ChaosApp<'_> {
    fn drop(&mut self) {
        if let Some(wallpaper) = &self.wallpaper {
            if let Err(err) = wallpaper::set_from_path(wallpaper) {
                eprintln!("Error setting wallpaper back to default: {}", err);
            }
        } else {
            eprintln!("No default wallpaper found; leaving wallpaper as is");
        }
    }
}

fn random_window_position(
    width: u32,
    height: u32,
    monitor_width: u32,
    monitor_height: u32,
) -> PhysicalPosition<f32> {
    let x = if monitor_width > width {
        random_range(0..=(monitor_width - width))
    } else {
        0
    };
    let y = if monitor_height > height {
        random_range(0..=(monitor_height - height))
    } else {
        0
    };

    PhysicalPosition::new(x as f32, y as f32)
}

fn random_monitor(event_loop: &ActiveEventLoop) -> Option<MonitorHandle> {
    let monitors: Vec<_> = event_loop.available_monitors().collect();

    let mut rng = rand::rng();
    monitors.choose(&mut rng).cloned()
}
