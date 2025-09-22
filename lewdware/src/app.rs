use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use notify_rust::Notification;
use pack_format::config::{MediaType, Metadata};
use rand::random_bool;
use tempfile::NamedTempFile;
use winit::event::MouseButton;
use winit::event_loop::{ControlFlow, EventLoopProxy};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::WindowId,
};

use crate::audio::AudioPlayer;
use crate::config::AppConfig;
use crate::egui::WgpuState;
use crate::media::{self, Media, MediaManager, MediaResponse, Response};
use crate::transition::TransitionManager;
use crate::utils::create_window;
use crate::window::{ImageWindow, PromptWindow, VideoWindow};

/// The main app.
/// * `windows`: A map containing all the windows spawned by the app. Since dropping a winit window
///   closes it, we can close windows by removing them from this map.
/// * `default_wallpaper`: Stores the user's default wallpaper, so we can restore it on panic.
/// * `wallpaper`: The current wallpaper.
pub struct ChaosApp<'a> {
    state: AppState,
    config: AppConfig,
    metadata: Metadata,
    spawners: Spawners,
    wgpu_state: Arc<WgpuState>,
    windows: HashMap<WindowId, Window<'a>>,
    audio_player: Option<AudioPlayer>,
    media_manager: MediaManager,
    tags: Option<Vec<String>>,
    transition_manager: Option<TransitionManager>,
    default_wallpaper: Option<String>,
    wallpaper: Option<NamedTempFile>,
}

enum AppState {
    Running,
    Paused,
    Hibernating,
}

/// Tracks the timing of all the things the app spawns (popups, notifications, etc.)
struct Spawners {
    media: Spawner,
    audio: Option<Spawner>,
    notification: Option<Spawner>,
    link: Option<Spawner>,
    prompt: Option<Spawner>,
}

enum Window<'a> {
    Image(ImageWindow),
    Video(VideoWindow<'a>),
    Prompt(PromptWindow<'a>),
}

/// Tracks the timing and state of something the app spawns (popups, notifications, ...).
struct Spawner {
    last_spawn: Instant,
    original_frequency: Option<Duration>,
    frequency: Option<Duration>,
    lock: bool,
    locked: bool,
}

impl Spawner {
    /// Create a new spawner.
    ///
    /// * `frequency`: How often should we spawn? If [None], always spawn no matter the time of the
    ///   previous spawn.
    fn new(frequency: Option<Duration>, lock: bool) -> Self {
        Self {
            last_spawn: Instant::now(),
            original_frequency: frequency,
            frequency,
            lock,
            locked: false,
        }
    }

    /// Should we spawn?
    fn should_spawn(&self) -> bool {
        if self
            .frequency
            .is_some_and(|frequency| self.last_spawn.elapsed() >= frequency)
            && !self.locked
        {
            return true;
            // if let Some(frequency) = self.original_frequency {
            //     let secs = frequency.as_secs_f64();
            //
            //     let distr = Normal::new(secs, secs / 3.0).unwrap();
            //
            //     let mut sample = distr.sample(&mut rng());
            //
            //     if sample < 0.0 {
            //         sample = 0.0
            //     }
            //
            //     self.frequency = Some(Duration::from_secs_f64(sample));
            // }
        }

        false
    }

    /// Mark the spawner as "locked", which will not allow any spawns until `unlock()` is called.
    /// This is useful for spawners which only allow one of the thing at a time.
    fn spawn(&mut self) {
        self.last_spawn = Instant::now();

        if self.lock {
            self.locked = true;
        }
    }

    fn unlock(&mut self) {
        self.locked = false;
    }
}

#[derive(Debug)]
pub enum UserEvent {
    MediaResponse,
    PanicButtonPressed,
}

impl<'a> ChaosApp<'a> {
    pub fn new(
        wgpu_state: Arc<WgpuState>,
        event_loop_proxy: EventLoopProxy<UserEvent>,
        config: AppConfig,
    ) -> Result<Self> {
        println!("{:?}", config);

        println!("{:?}", config.pack_path);
        let (media_manager, metadata) =
            MediaManager::open(config.pack_path.as_ref().unwrap(), event_loop_proxy)?;

        let transition = metadata.transition.as_ref().cloned();

        let wallpaper = match wallpaper::get() {
            Ok(wallpaper) => Some(wallpaper),
            Err(err) => {
                eprintln!("Error getting wallpaper: {}", err);
                None
            }
        };

        let spawners = Spawners {
            media: Spawner::new(Some(config.popup_frequency), false),
            audio: if config.audio {
                Some(Spawner::new(None, true))
            } else {
                None
            },
            notification: if config.notifications {
                Some(Spawner::new(Some(config.notification_frequency), false))
            } else {
                None
            },
            link: if config.open_links {
                Some(Spawner::new(Some(config.link_frequency), false))
            } else {
                None
            },
            prompt: if config.prompts {
                Some(Spawner::new(Some(config.prompt_frequency), true))
            } else {
                None
            },
        };

        Ok(Self {
            state: AppState::Running,
            config,
            metadata,
            wgpu_state,
            windows: HashMap::new(),
            spawners,
            audio_player: None,
            media_manager,
            tags: None,
            transition_manager: transition
                .map(|transition| TransitionManager::new(transition.clone())),
            default_wallpaper: wallpaper,
            wallpaper: None,
        })
    }

    fn try_spawn(&mut self) {
        if self.spawners.media.should_spawn() {
            let only_images = self
                .windows
                .values()
                .filter(|window| matches!(window, Window::Video(_)))
                .count()
                >= self.config.max_videos;
            let tags = self.get_tags(MediaType::Popups);

            if self
                .media_manager
                .request_media(tags, only_images)
                .is_some()
            {
                self.spawners.media.spawn();
            }
        }

        if let Some(spawner) = &self.spawners.audio
            && spawner.should_spawn()
        {
            let tags = self.get_tags(MediaType::Audio);
            if self.media_manager.request_audio(tags).is_some() {
                self.spawners.audio.as_mut().unwrap().spawn();
            }
        }

        if let Some(spawner) = &self.spawners.notification
            && spawner.should_spawn()
        {
            let tags = self.get_tags(MediaType::Notifications);
            if self.media_manager.request_notification(tags).is_some() {
                self.spawners.notification.as_mut().unwrap().spawn();
            }
        }

        if let Some(spawner) = &self.spawners.link
            && spawner.should_spawn()
        {
            let tags = self.get_tags(MediaType::Links);
            if self.media_manager.request_link(tags).is_some() {
                self.spawners.link.as_mut().unwrap().spawn();
            }
        }

        if let Some(spawner) = &self.spawners.prompt
            && spawner.should_spawn()
        {
            let tags = self.get_tags(MediaType::Prompts);
            if self.media_manager.request_prompt(tags).is_some() {
                self.spawners.prompt.as_mut().unwrap().spawn();
            }
        }
    }

    /// Process a message sent my the media manager.
    fn process_media_message(
        &mut self,
        response: Response,
        event_loop: &ActiveEventLoop,
    ) -> Result<()> {
        match response.response {
            MediaResponse::Media(media) => match media {
                Media::Image(image) => {
                    let window =
                        create_window(event_loop, image.width(), image.height(), false, false)?;

                    window.request_redraw();

                    let move_window = if self.config.moving_windows {
                        random_bool(self.config.moving_window_chance as f64 / 100.0)
                    } else {
                        false
                    };

                    self.windows.insert(
                        window.id(),
                        Window::Image(ImageWindow::new(
                            window,
                            image,
                            self.config.close_button,
                            move_window,
                        )?),
                    );
                }
                Media::Video(video) => {
                    let window = create_window(
                        event_loop,
                        video.width as u32,
                        video.height as u32,
                        false,
                        false,
                    )?;

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

                if let Some(spawner) = self.spawners.audio.as_mut() {
                    spawner.unlock();
                }
            }
            MediaResponse::Notification(notification) => self.send_notification(notification),
            MediaResponse::Prompt(prompt) => {
                let window = create_window(event_loop, 400, 400, true, true)?;

                self.windows.insert(
                    window.id(),
                    Window::Prompt(PromptWindow::new(&self.wgpu_state, window, prompt.prompt)?),
                );

                if let Some(spawner) = self.spawners.prompt.as_mut() {
                    spawner.unlock();
                }
            }
            MediaResponse::Link(link) => self.open_link(link.link),
            MediaResponse::Wallpaper(file) => {
                let path = match file.path().to_str() {
                    Some(path) => path,
                    None => {
                        eprintln!("Could not convert tempfile to UTF-8");
                        return Ok(());
                    }
                };

                wallpaper::set_from_path(path).map_err(|err| anyhow!("{}", err))?;
                self.wallpaper = Some(file);
            }
        }

        Ok(())
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

    /// Get the relevant tags for a specific media type.
    fn get_tags(&self, media_type: MediaType) -> Option<Vec<String>> {
        match &self.transition_manager {
            Some(transition_manager) => transition_manager.get_tags(media_type).map(|tags| {
                tags.into_iter()
                    .filter(|tag| self.tags.as_ref().is_none_or(|tags| tags.contains(tag)))
                    .collect()
            }),
            None => self.tags.as_ref().cloned(),
        }
    }
}

impl<'a> ApplicationHandler<UserEvent> for ChaosApp<'a> {
    fn resumed(&mut self, _: &ActiveEventLoop) {
        let tags = self.get_tags(MediaType::Popups);
        if self.media_manager.request_media(tags, false).is_some() {
            self.spawners.media.spawn();
        }

        if self.spawners.audio.is_some() {
            let tags = self.get_tags(MediaType::Audio);
            if self.media_manager.request_audio(tags).is_some() {
                self.spawners.audio.as_mut().unwrap().spawn();
            }
        }

        let tags = self.get_tags(MediaType::Wallpaper);
        self.media_manager.request_wallpaper(tags);
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
                    event_loop.exit();
                }
                _ => {}
            }
        }
    }

    /// By user events we really mean custom events, which can be sent by code running outside the
    /// main event loop (e.g. on another thread).
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::MediaResponse => {
                while let Some(response) = self.media_manager.try_recv() {
                    self.process_media_message(response, event_loop)
                        .unwrap_or_else(|err| {
                            eprintln!("Error: {}", err);
                        });
                }
            }
            UserEvent::PanicButtonPressed => {
                event_loop.exit();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(duration) = self.config.max_popup_duration {
            self.windows.retain(|_, window| match window {
                Window::Image(window) => window.created.elapsed() <= duration,
                Window::Video(window) => window.created.elapsed() <= duration,
                Window::Prompt(window) => !window.closed(),
            });
        }

        for window in self.windows.values_mut() {
            match window {
                Window::Video(window) => {
                    window.window.request_redraw();
                }
                Window::Prompt(_) => {}
                Window::Image(window) => {
                    if window.moving
                        && let Err(err) = window.update_position()
                    {
                        eprintln!("Error moving window: {}", err);
                    }
                }
            }
        }

        event_loop.set_control_flow(ControlFlow::Poll);

        if self.audio_player.as_ref().is_some_and(|x| x.is_finished())
            && let Some(spawner) = self.spawners.audio.as_mut()
        {
            spawner.unlock();
        }

        // The transition has switched from one stage to another
        if self
            .transition_manager
            .as_mut()
            .is_some_and(|manager| manager.try_switch())
        {
            if self
                .transition_manager
                .as_ref()
                .unwrap()
                .applies_to(&MediaType::Wallpaper)
            {
                let tags = self.get_tags(MediaType::Wallpaper);
                self.media_manager.request_wallpaper(tags);
            }

            if self
                .transition_manager
                .as_ref()
                .unwrap()
                .applies_to(&MediaType::Audio)
            {
                let tags = self.get_tags(MediaType::Audio);
                self.media_manager.request_audio(tags);
            }
        }

        self.try_spawn();
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
