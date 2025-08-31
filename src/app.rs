use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rand::random_range;
use rand::seq::IndexedRandom;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::MouseButton;
use winit::event_loop::ControlFlow;
use winit::monitor::MonitorHandle;
use winit::window::{Window, WindowLevel};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, NamedKey},
    window::{WindowAttributes, WindowId},
};

use crate::media::{Media, MediaManager};
use crate::window::{ImageState, VideoState};

pub struct ChaosApp<'a> {
    image_windows: HashMap<WindowId, ImageState>,
    video_windows: HashMap<WindowId, VideoState<'a>>,
    running: Arc<AtomicBool>,
    last_spawn: Instant,
    spawn_interval: Duration,
    media_manager: MediaManager,
    max_videos: usize,
}

impl<'a> ChaosApp<'a> {
    pub fn new(media_manager: MediaManager, running: Arc<AtomicBool>) -> Self {
        Self {
            image_windows: HashMap::new(),
            video_windows: HashMap::new(),
            running,
            last_spawn: Instant::now(),
            spawn_interval: Duration::from_millis(200),
            media_manager,
            max_videos: 20,
        }
    }

    fn spawn_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.video_windows.len() >= self.max_videos
            && let Some(image) = self.media_manager.get_random_image(None).unwrap()
        {
            let window = create_window(event_loop, image.width(), image.height());

            self.image_windows
                .insert(window.id(), ImageState::new(window, image).unwrap());
        } else if let Some(media) = self.media_manager.get_random_item(None).unwrap() {
            match media {
                Media::Image(image) => {
                    let window = create_window(event_loop, image.width(), image.height());

                    self.image_windows
                        .insert(window.id(), ImageState::new(window, image).unwrap());
                }
                Media::Video(video) => {
                    let window = create_window(event_loop, video.width as u32, video.height as u32);

                    let tempfile = self.media_manager.write_to_temp_file(&video).unwrap();

                    self.video_windows.insert(
                        window.id(),
                        VideoState::new(window, video, tempfile).unwrap(),
                    );
                }
            }
        }
    }
}

fn create_window(event_loop: &ActiveEventLoop, width: u32, height: u32) -> Window {
    let monitor = random_monitor(event_loop);

    let position = if let Some(monitor) = monitor {
        let size = monitor.size();
        let monitor_position = monitor.position();

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
        .with_inner_size(PhysicalSize::new(width, height))
        .with_position(position)
        .with_decorations(true)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_resizable(false);

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::{WindowAttributesExtX11, WindowType};

        attrs = attrs.with_x11_window_type(vec![WindowType::Notification]);
    }

    #[cfg(target_os = "windows")]
    {
        attrs = attrs.with_skip_taskbar(true);
    }

    event_loop.create_window(attrs).unwrap()
}

impl<'a> ApplicationHandler for ChaosApp<'a> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.spawn_window(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.image_windows.remove(&window_id);
                self.video_windows.remove(&window_id);
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
            WindowEvent::RedrawRequested => {
                if let Some(window) = self.image_windows.get_mut(&window_id) {
                    window.draw().unwrap();
                }

                if let Some(window) = self.video_windows.get_mut(&window_id) {
                    window.update().unwrap();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.image_windows.remove(&window_id);
                self.video_windows.remove(&window_id);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.running.load(Ordering::Relaxed) {
            event_loop.exit();
            return;
        }

        if self.last_spawn.elapsed() >= self.spawn_interval {
            self.spawn_window(event_loop);

            self.last_spawn = Instant::now();
        }

        for video in self.video_windows.values() {
            video.window.request_redraw();
        }

        let video_active = !self.video_windows.is_empty();
        if video_active {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                self.last_spawn + self.spawn_interval,
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
