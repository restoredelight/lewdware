use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, channel},
    },
    thread,
    time::Duration,
};

use anyhow::Result;
use async_channel::{Sender, unbounded};
use async_executor::Executor;
use egui::{CollapsingHeader, ScrollArea, Slider};
use futures_lite::future::block_on;
use pack_format::config::Metadata;
use rfd::AsyncFileDialog;
use winit::{
    application::ApplicationHandler, dpi::LogicalSize, event::WindowEvent, window::WindowAttributes,
};

use crate::{
    app::UserEvent,
    config::{AppConfig, file::save_config},
    egui::{EguiWindow, WgpuState},
    utils::read_pack_metadata,
};

pub struct ConfigApp<'a> {
    wgpu_state: Arc<WgpuState>,
    window: Option<Window<'a>>,
    config: Option<AppConfig>,
    closed: bool,
    start: bool,
    executor: Arc<Executor<'static>>,
    shutdown: Sender<()>,
}

fn spawn_executor_thread() -> (Arc<Executor<'static>>, Sender<()>) {
    let executor = Arc::new(Executor::new());
    let ex = executor.clone();
    let (signal, shutdown) = unbounded();

    thread::spawn(move || {
        let _ = block_on(ex.run(shutdown.recv()));
    });

    (executor, signal)
}

impl<'a> ConfigApp<'a> {
    pub fn new(wgpu_state: Arc<WgpuState>, config: AppConfig) -> Self {
        let (executor, shutdown) = spawn_executor_thread();

        Self {
            wgpu_state,
            window: None,
            config: Some(config),
            closed: false,
            start: false,
            executor,
            shutdown,
        }
    }

    pub fn should_start(&self) -> bool {
        self.start
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn into_config(self) -> AppConfig {
        self.config.unwrap()
    }
}

impl<'a> ApplicationHandler<UserEvent> for ConfigApp<'a> {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(config) = self.config.take() {
            let attrs = WindowAttributes::default()
                .with_title("Lewdware config")
                .with_inner_size(LogicalSize::new(480.0, 650.0))
                .with_min_inner_size(LogicalSize::new(400.0, 500.0));

            let window = event_loop.create_window(attrs).unwrap();

            self.window =
                Some(Window::new(&self.wgpu_state, window, self.executor.clone(), config).unwrap());
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        if let Some(window) = self.window.as_mut()
            && window.window.id() == window_id
        {
            match event {
                WindowEvent::RedrawRequested => {
                    window.render().unwrap_or_else(|err| {
                        eprintln!("Error rendering window: {}", err);
                    });
                }
                WindowEvent::CloseRequested => {
                    self.start = true;
                    self.closed = true;
                }
                event => {
                    window.handle_event(&event);
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.window.as_ref().is_some_and(|window| window.closed()) {
            self.start = true;
            self.closed = true;
        }
    }

    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        let window = self.window.take().unwrap();
        let config = window.config;

        if let Err(err) = save_config(&config) {
            eprintln!("Could not save config: {}", err);
        }

        self.config = Some(config);
    }
}

struct Window<'a> {
    pub window: Arc<winit::window::Window>,
    pub config: AppConfig,
    egui_window: EguiWindow<'a>,
    closed: bool,
    executor: Arc<Executor<'static>>,
    file_tx: mpsc::Sender<(PathBuf, Metadata)>,
    file_rx: mpsc::Receiver<(PathBuf, Metadata)>,
    pack_metadata: Option<Metadata>,
}

impl<'a> Window<'a> {
    fn new(
        wgpu_state: &WgpuState,
        window: winit::window::Window,
        executor: Arc<Executor<'static>>,
        config: AppConfig,
    ) -> Result<Self> {
        let window = Arc::new(window);
        let egui_window = EguiWindow::new(wgpu_state, window.clone())?;
        let (file_tx, file_rx) = channel();

        let pack_metadata = config
            .pack_path
            .as_ref()
            .and_then(|path| fs::File::open(path).ok())
            .and_then(|file| read_pack_metadata(file).ok())
            .map(|(_, metadata)| metadata);

        Ok(Self {
            window,
            egui_window,
            config,
            closed: false,
            executor,
            file_tx,
            file_rx,
            pack_metadata,
        })
    }

    fn handle_event(&mut self, event: &WindowEvent) {
        if self.egui_window.handle_event(event) {
            self.window.request_redraw();
        }
    }

    fn render(&mut self) -> Result<()> {
        if let Ok((path, metadata)) = self.file_rx.try_recv() {
            self.config.pack_path = Some(path);
            self.pack_metadata = Some(metadata);
        }

        self.egui_window.redraw(|ctx| {
            // ctx.set_visuals(egui::Visuals::light());

            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add_space(10.0);
                ui.heading("Chaos Config");

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(12.0);

                ScrollArea::vertical().show(ui, |ui| {
                    CollapsingHeader::new("⚙️ General")
                        .default_open(true)
                        .show(ui, |ui| {
                            ui.group(|ui| {
                                ui.set_min_height(40.0);
                                ui.vertical_centered_justified(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("📦 Media Pack:");
                                        ui.add_space(8.0);

                                        if let Some(metadata) = self.pack_metadata.as_ref() {
                                            ui.label(
                                                egui::RichText::new(&metadata.name)
                                                    .color(egui::Color32::from_rgb(0, 120, 0))
                                                    .strong(),
                                            );
                                        } else {
                                            ui.label(
                                                egui::RichText::new("No pack selected")
                                                    .color(egui::Color32::GRAY)
                                                    .italics(),
                                            );
                                        }

                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.button("📁 Browse...").clicked() {
                                                    let file_tx = self.file_tx.clone();

                                                    self.executor
                                                        .spawn(async move {
                                                            let result = AsyncFileDialog::new()
                                                                .add_filter("Media packs", &["md"])
                                                                .pick_file()
                                                                .await;

                                                            if let Some(result) = result {
                                                                let path = result.path();

                                                                let mut file =
                                                                    fs::File::open(result.path())
                                                                        .unwrap();

                                                                let (_, metadata) =
                                                                    read_pack_metadata(&mut file)
                                                                        .unwrap();

                                                                file_tx
                                                                    .send((
                                                                        path.to_path_buf(),
                                                                        metadata,
                                                                    ))
                                                                    .unwrap();
                                                            }
                                                        })
                                                        .detach();
                                                }
                                            },
                                        );
                                    });
                                });
                            });

                            ui.add_space(12.0);

                            // Better slider layouts with labels
                            ui.horizontal(|ui| {
                                ui.label("⏱️ Popup frequency:");
                                ui.add_space(8.0);
                            });
                            ui.add(
                                Slider::from_get_set(0.1..=10.0, |value| {
                                    if let Some(value) = value {
                                        self.config.popup_frequency =
                                            Duration::from_secs_f64(value.max(0.0));
                                    }
                                    self.config.popup_frequency.as_secs_f64()
                                })
                                .clamping(egui::SliderClamping::Never)
                                .step_by(0.1)
                                .suffix(" seconds")
                                .min_decimals(1)
                                .max_decimals(2),
                            );

                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                ui.label("⏳ Max popup duration:");
                                ui.add_space(8.0);
                            });

                            ui.horizontal(|ui| {
                                if let Some(duration) = self.config.max_popup_duration {
                                    ui.add(
                                        Slider::from_get_set(1.0..=600.0, |value| {
                                            if let Some(value) = value {
                                                self.config.max_popup_duration =
                                                    Some(Duration::from_secs(value as u64));
                                            }
                                            duration.as_secs() as f64
                                        })
                                        .suffix(" seconds"),
                                    );

                                    ui.add_space(8.0);
                                    if ui.button("❌ Clear").clicked() {
                                        self.config.max_popup_duration = None;
                                    }
                                } else {
                                    ui.label(
                                        egui::RichText::new("No limit set")
                                            .color(egui::Color32::GRAY)
                                            .italics(),
                                    );
                                    ui.add_space(8.0);
                                    if ui.button("➕ Set Limit").clicked() {
                                        self.config.max_popup_duration =
                                            Some(Duration::from_secs(60));
                                    }
                                }
                            });

                            ui.add_space(8.0);

                            ui.horizontal(|ui| {
                                ui.label("🎬 Max videos:");
                                ui.add_space(8.0);
                            });
                            ui.add(
                                Slider::new(&mut self.config.max_videos, 0..=100).suffix(" videos"),
                            );

                            ui.add_space(8.0);
                        });

                    ui.add_space(8.0);

                    CollapsingHeader::new("🔊 Audio")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add_space(8.0);

                            ui.checkbox(&mut self.config.video_audio, "🎵 Play audio from videos");
                            ui.add_space(4.0);
                            ui.checkbox(&mut self.config.audio, "🎶 Play background audio");

                            ui.add_space(8.0);
                        });

                    ui.add_space(8.0);

                    CollapsingHeader::new("🔗 Links & Notifications")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add_space(8.0);

                            ui.checkbox(&mut self.config.open_links, "🌐 Open links");
                            if self.config.open_links {
                                ui.add_space(4.0);
                                ui.indent("links_indent", |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("Link frequency:");
                                        ui.add_space(8.0);
                                    });
                                    ui.add(
                                        Slider::from_get_set(1.0..=60.0, |value| {
                                            if let Some(value) = value {
                                                self.config.link_frequency =
                                                    Duration::from_secs_f64(value.max(0.0));
                                            }
                                            self.config.link_frequency.as_secs_f64()
                                        })
                                        .clamping(egui::SliderClamping::Never)
                                        .suffix(" seconds"),
                                    );
                                });
                            }

                            ui.add_space(8.0);

                            ui.checkbox(&mut self.config.notifications, "📢 Show notifications");
                            if self.config.notifications {
                                ui.add_space(4.0);
                                ui.indent("notifications_indent", |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("Notification frequency:");
                                        ui.add_space(8.0);
                                    });
                                    ui.add(
                                        Slider::from_get_set(1.0..=60.0, |value| {
                                            if let Some(value) = value {
                                                self.config.notification_frequency =
                                                    Duration::from_secs_f64(value.max(0.0));
                                            }
                                            self.config.notification_frequency.as_secs_f64()
                                        })
                                        .clamping(egui::SliderClamping::Never)
                                        .suffix(" seconds"),
                                    );
                                });
                            }

                            ui.checkbox(&mut self.config.prompts, "✏ Show prompts");
                            if self.config.prompts {
                                ui.add_space(4.0);
                                ui.indent("prompts_indent", |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("Prompt frequency:");
                                        ui.add_space(8.0);
                                    });
                                    ui.add(
                                        Slider::from_get_set(10.0..=600.0, |value| {
                                            if let Some(value) = value {
                                                self.config.prompt_frequency =
                                                    Duration::from_secs_f64(value.max(0.0));
                                            }
                                            self.config.prompt_frequency.as_secs_f64()
                                        })
                                        .clamping(egui::SliderClamping::Never)
                                        .suffix(" seconds"),
                                    );
                                });
                            }

                            ui.add_space(8.0);
                        });

                    ui.add_space(8.0);

                    CollapsingHeader::new("🪟 Popups")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add_space(8.0);

                            ui.checkbox(
                                &mut self.config.moving_windows,
                                "🏃 Make popups move around",
                            );
                            if self.config.moving_windows {
                                ui.add_space(4.0);
                                ui.indent("moving_indent", |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("Move chance:");
                                        ui.add_space(8.0);
                                    });
                                    ui.add(
                                        Slider::new(&mut self.config.moving_window_chance, 1..=100)
                                            .suffix("%"),
                                    );
                                });
                            }

                            ui.add_space(8.0);

                            ui.checkbox(
                                &mut self.config.close_button,
                                "❌ Show close button on popups",
                            );
                            if !self.config.close_button {
                                ui.add_space(4.0);
                                ui.indent("close_help", |ui| {
                                    ui.label(
                                        egui::RichText::new(
                                            "💡 Tip: Click anywhere on the popup to close it.",
                                        )
                                        .color(egui::Color32::from_rgb(100, 100, 100))
                                        .italics(),
                                    );
                                });
                            }

                            ui.add_space(8.0);
                        });

                    // Uncomment and enhance the Tags section when ready
                    // ui.add_space(8.0);
                    // CollapsingHeader::new("🏷️ Tags")
                    //     .default_open(false)
                    //     .show(ui, |ui| {
                    //         ui.add_space(8.0);
                    //
                    //         ui.label("Enabled tags:");
                    //         ui.add_space(4.0);
                    //
                    //         // Use a grid layout for better organization
                    //         egui::Grid::new("tags_grid")
                    //             .num_columns(3)
                    //             .spacing([8.0, 4.0])
                    //             .show(ui, |ui| {
                    //                 for (i, tag) in self.all_tags.iter().enumerate() {
                    //                     let mut enabled = self.enabled_tags.contains(tag);
                    //                     if ui.checkbox(&mut enabled, tag).changed() {
                    //                         if enabled {
                    //                             self.enabled_tags.push(tag.clone());
                    //                         } else {
                    //                             self.enabled_tags.retain(|t| t != tag);
                    //                         }
                    //                     }
                    //
                    //                     if (i + 1) % 3 == 0 {
                    //                         ui.end_row();
                    //                     }
                    //                 }
                    //             });
                    //
                    //         ui.add_space(8.0);
                    //     });

                    // Add some bottom padding
                    ui.add_space(20.0);
                });
            });
        })?;

        Ok(())
    }

    fn closed(&self) -> bool {
        self.closed
    }
}
