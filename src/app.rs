use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Result, anyhow};
use egui_wgpu::wgpu;
use notify_rust::Notification;
use rand::seq::IndexedRandom;
use rand::{random_bool, random_range};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::MouseButton;
use winit::event_loop::ControlFlow;
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
use crate::media::{Media, MediaManager};
use crate::window::{ImageWindow, PromptWindow, VideoWindow};

pub struct ChaosApp<'a> {
    config: AppConfig,
    wgpu_instance: wgpu::Instance,
    windows: HashMap<WindowId, Window<'a>>,
    running: Arc<AtomicBool>,
    last_spawn: Instant,
    media_manager: MediaManager,
    audio_player: Option<AudioPlayer>,
}

enum Window<'a> {
    Image(ImageWindow),
    Video(VideoWindow<'a>),
    Prompt(PromptWindow<'a>),
}

impl<'a> ChaosApp<'a> {
    pub fn new(media_manager: MediaManager, config: AppConfig, running: Arc<AtomicBool>) -> Self {
        Self {
            config,
            wgpu_instance: wgpu::Instance::new(&wgpu::InstanceDescriptor::default()),
            windows: HashMap::new(),
            running,
            last_spawn: Instant::now(),
            media_manager,
            audio_player: None,
        }
    }

    fn play_audio(&mut self) -> Result<Option<AudioPlayer>> {
        if let Some(audio) = self.media_manager.get_random_audio(None)? {
            Ok(Some(AudioPlayer::new(audio, &mut self.media_manager)?))
        } else {
            println!("No audio files found");
            Ok(None)
        }
    }

    fn spawn_window(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if self
            .windows
            .values()
            .filter(|window| matches!(window, Window::Video(_)))
            .count()
            >= self.config.max_videos
            && let Some(image) = self.media_manager.get_random_image(None)?
        {
            let window = create_window(event_loop, image.width(), image.height(), false)?;

            window.request_redraw();

            self.windows.insert(
                window.id(),
                Window::Image(ImageWindow::new(window, image, self.config.close_button)?),
            );
        } else if let Some(media) = self.media_manager.get_random_item(None)? {
            match media {
                Media::Image(image) => {
                    let window = create_window(event_loop, image.width(), image.height(), false)?;

                    self.windows.insert(
                        window.id(),
                        Window::Image(ImageWindow::new(window, image, self.config.close_button)?),
                    );
                }
                Media::Video(video) => {
                    let window =
                        create_window(event_loop, video.width as u32, video.height as u32, false)?;

                    let tempfile = self.media_manager.write_video_to_temp_file(&video)?;

                    self.windows.insert(
                        window.id(),
                        Window::Video(VideoWindow::new(
                            &self.wgpu_instance,
                            window,
                            video,
                            tempfile,
                            self.config.close_button,
                        )?),
                    );
                }
            }
        }

        Ok(())
    }

    fn spawn_prompt(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let window = create_window(event_loop, 400, 400, true)?;

        self.windows.insert(
            window.id(),
            Window::Prompt(PromptWindow::new(
                &self.wgpu_instance,
                window,
                "I can't stop gooning".to_string(),
            )?),
        );

        Ok(())
    }

    fn open_link(&self) {
        match webbrowser::open("https://censored.booru.org/index.php?page=post&s=list") {
            Ok(_) => {}
            Err(err) => {
                eprintln!("Could not open link in web browser: {}", err);
            }
        }
    }

    fn send_notification(&self) {
        match Notification::new()
            .summary("Kill yourself!")
            .body("Keep gooning~")
            .show()
        {
            Ok(_) => {}
            Err(err) => {
                eprintln!("Couldn't show notification: {}", err)
            }
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

impl<'a> ApplicationHandler for ChaosApp<'a> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.spawn_window(event_loop).unwrap_or_else(|err| {
            eprintln!("Error spawning audio: {}", err);
        });
        self.audio_player = self.play_audio().unwrap_or_else(|err| {
            eprintln!("Error playing audio: {}", err);
            None
        });
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.running.load(Ordering::Relaxed) {
            event_loop.exit();
            return;
        }

        if self.last_spawn.elapsed() >= self.config.spawn_interval {
            if random_bool(1.0 / 5.0)
                && !self
                    .windows
                    .values()
                    .any(|window| matches!(window, Window::Prompt(_)))
            {
                self.spawn_prompt(event_loop).unwrap_or_else(|err| {
                    println!("Error spawning prompt: {}", err);
                });
            } else {
                self.spawn_window(event_loop).unwrap_or_else(|err| {
                    println!("Error spawning window: {}", err);
                });
            }

            self.last_spawn = Instant::now();

            if random_bool(1.0 / 50.0) {
                self.open_link();
            }

            if random_bool(1.0 / 10.0) {
                self.send_notification();
            }
        }

        if self
            .audio_player
            .as_ref()
            .is_some_and(|player| player.is_finished())
        {
            self.audio_player = self.play_audio().unwrap_or_else(|err| {
                println!("Error playing audio: {}", err);
                None
            });
        }

        if let Some(duration) = self.config.window_duration {
            self.windows.retain(|_, window| match window {
                Window::Image(window) => window.created.elapsed() <= duration,
                Window::Video(window) => window.created.elapsed() <= duration && !window.closed(),
                Window::Prompt(window) => !window.closed(),
            });
        }

        let mut poll = false;
        for window in self.windows.values() {
            match window {
                Window::Video(window) => {
                    window.window.request_redraw();
                    poll = true;
                }
                Window::Prompt(_) => {
                    poll = true;
                }
                Window::Image(_) => {}
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
