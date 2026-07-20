use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, anyhow};
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
    self, AudioAction, DialogElement, FadeOpts, ItemAction, ItemId, LuaRequest, LuaThreadHandle,
    MediaData, MoveOpts, Notification, PackInfo, PopupSpawnOpts, TextStyle, WallpaperMode,
    WindowAction, WindowProps, start_lua_thread,
};
use crate::media::{
    FileOrPath, MediaError, MediaManager, MediaRequirement, RequirementId, ResolvedMedia,
};
use crate::monitor::Monitors;
use crate::wgpu::WgpuState;
use crate::window::{
    DialogWindow, ImageWindow, InnerWindow, TextWindow, VideoWindow, WindowOpts, WindowPool,
    WindowType,
};

mod pending;

use pending::{PendingItem, PendingItemOpts, PendingWindowOpts, Requirement, RequirementState};

/// The main app.
/// * `items`: Every Lua-created popup and audio handle, including items still waiting for media.
///   The key is the Lua-facing [`ItemId`], shared across both kinds.
/// * `window_ids`: Reverse lookup from winit's `WindowId` to `ItemId`, needed to route winit's
///   `window_event` callback back to a ready window item.
/// * `requirement_owners`: Routes each independent decode request back to its owning pending item.
pub struct LewdwareApp {
    running: bool,
    config: Arc<AppConfig>,
    wgpu_state: Option<Arc<WgpuState>>,
    items: HashMap<ItemId, ItemSlot>,
    window_ids: HashMap<WindowId, ItemId>,
    requirement_owners: HashMap<RequirementId, ItemId>,
    next_requirement_id: u64,
    default_wallpaper: Option<String>,
    lua_request_rx: std::sync::mpsc::Receiver<lua::LuaRequest>,
    lua_event_tx: tokio::sync::mpsc::UnboundedSender<lua::Event>,
    _lua_thread_handle: LuaThreadHandle,
    monitors: Monitors,
    window_pool: WindowPool,
    media_manager: MediaManager,
    event_loop_proxy: EventLoopProxy<UserEvent>,
    /// Coalesces Windows redraw wake-ups: while this is true, one `RedrawRequested` user event is
    /// already queued and any further per-window redraw requests can ride on the same event.
    redraw_wakeup_pending: Arc<AtomicBool>,
}

/// One lifecycle slot shared by popups and audio handles.
#[allow(clippy::large_enum_variant)]
enum ItemSlot {
    Pending(PendingItem),
    Window(WindowType),
    Audio(AudioPlayer),
}

pub enum UserEvent {
    Exit,
    LuaRequest,
    #[allow(unused)] RedrawRequested,
    AudioFinish {
        id: ItemId,
    },
    MediaResolved {
        requirement_id: RequirementId,
        result: std::result::Result<ResolvedMedia, MediaError>,
    },
}

pub trait EventPoster: Send + Sync + Clone + 'static {
    fn post_event(&self, event: UserEvent) -> bool;
}

impl EventPoster for EventLoopProxy<UserEvent> {
    fn post_event(&self, event: UserEvent) -> bool {
        self.send_event(event).is_ok()
    }
}

impl LewdwareApp {
    pub fn new(
        wgpu_state: Option<std::sync::Arc<WgpuState>>,
        event_loop_proxy: EventLoopProxy<UserEvent>,
        config: AppConfig,
        dev_mode: bool,
    ) -> anyhow::Result<Self> {
        let config = Arc::new(config);

        let wallpaper = match wallpaper::get() {
            Ok(wallpaper) => Some(wallpaper),
            Err(err) => {
                tracing::warn!("Error getting wallpaper: {}", err);
                None
            }
        };

        tracing::debug!("{:?}", config);

        let pack_path = config
            .pack_path
            .as_ref()
            .context("No pack is configured, but the engine requires one to start")?;

        let wgpu_device = wgpu_state.as_ref().map(|s| s.device.clone());

        let (media_manager, pack_metadata, pack_id) =
            MediaManager::open(pack_path, event_loop_proxy.clone(), wgpu_device)?;

        let (lua_event_tx, lua_request_rx, lua_thread_handle) = start_lua_thread(
            event_loop_proxy.clone(),
            config.clone(),
            media_manager.clone(),
            PackInfo {
                id: pack_id,
                metadata: pack_metadata,
            },
            wgpu_state.is_some(),
            dev_mode,
        );

        let monitors = Monitors::new(config.disabled_monitors.clone());

        Ok(Self {
            running: false,
            config,
            wgpu_state,
            items: HashMap::new(),
            window_ids: HashMap::new(),
            requirement_owners: HashMap::new(),
            next_requirement_id: 0,
            default_wallpaper: wallpaper,
            lua_request_rx,
            lua_event_tx,
            _lua_thread_handle: lua_thread_handle,
            monitors,
            window_pool: WindowPool::new(),
            media_manager,
            event_loop_proxy,
            redraw_wakeup_pending: Arc::new(AtomicBool::new(false)),
        })
    }

    fn new_requirement(&mut self, media: MediaRequirement) -> Requirement {
        let id = self.next_requirement_id;
        self.next_requirement_id += 1;
        Requirement {
            id: RequirementId(id),
            state: RequirementState::Pending(media),
        }
    }

    fn media_manager(&self) -> MediaManager {
        self.media_manager.clone()
    }

    fn insert_pending_item(
        &mut self,
        id: ItemId,
        opts: PendingItemOpts,
        requirements: Vec<Requirement>,
    ) -> Result<()> {
        let requests = requirements
            .iter()
            .map(|requirement| match &requirement.state {
                RequirementState::Pending(media) => (requirement.id, media.clone()),
                RequirementState::Resolved(_) => {
                    unreachable!("new pending items cannot contain resolved requirements")
                }
            })
            .collect::<Vec<_>>();
        let media_manager = (!requests.is_empty()).then(|| self.media_manager());

        match self.items.entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(ItemSlot::Pending(PendingItem { opts, requirements }));
            }
            Entry::Occupied(_) => return Err(LewdwareError::Internal("Duplicate item id")),
        }

        for (requirement_id, media) in requests {
            self.requirement_owners.insert(requirement_id, id);
            if let Err(err) = media_manager
                .as_ref()
                .expect("media manager exists when requirements are present")
                .resolve(requirement_id, media)
            {
                self.remove_pending_item(id);
                return Err(err.into());
            }
        }
        Ok(())
    }

    fn remove_pending_item(&mut self, id: ItemId) -> Option<PendingItem> {
        let ItemSlot::Pending(item) = self.items.remove(&id)? else {
            return None;
        };
        for requirement in &item.requirements {
            self.requirement_owners.remove(&requirement.id);
        }
        Some(item)
    }

    fn finalize_window_opts(
        &mut self,
        popup_opts: PopupSpawnOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowOpts> {
        let monitor = self
            .monitors
            .list(event_loop)
            .into_iter()
            .find(|m| m.id == popup_opts.monitor_id)
            .ok_or(MonitorError::MonitorNotFound)?;

        let monitor_handle = self
            .monitors
            .get_handle(popup_opts.monitor_id, event_loop)
            .ok_or(MonitorError::MonitorNotFound)?;

        let scale_factor = monitor_handle.scale_factor();
        let monitor_position: LogicalPosition<i32> =
            monitor_handle.position().to_logical(scale_factor);

        let position = LogicalPosition::new(
            monitor_position.x + popup_opts.x,
            monitor_position.y + popup_opts.y,
        );

        Ok(WindowOpts {
            popup_opts,
            monitor,
            position,
        })
    }

    fn create_window(
        &mut self,
        popup_id: ItemId,
        opts: WindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<InnerWindow> {
        let window = self
            .window_pool
            .acquire(&opts, event_loop)
            .map_err(LewdwareError::WindowError)?;

        self.window_ids.insert(window.id(), popup_id);

        let inner_window = InnerWindow::new(
            window,
            &opts,
            self.wgpu_state.clone(),
            self.lua_event_tx.clone(),
            popup_id,
            self.event_loop_proxy.clone(),
            self.redraw_wakeup_pending.clone(),
        )
        .map_err(LewdwareError::WindowError)?;

        Ok(inner_window)
    }

    fn window_props(popup_id: ItemId, opts: &WindowOpts) -> WindowProps {
        WindowProps {
            window_id: popup_id,
            width: opts.popup_opts.width,
            height: opts.popup_opts.height,
            outer_width: opts.popup_opts.outer_width,
            outer_height: opts.popup_opts.outer_height,
            x: opts.popup_opts.x,
            y: opts.popup_opts.y,
            monitor: opts.monitor.clone(),
        }
    }

    fn close_window(&mut self, window_type: WindowType) {
        self.window_ids
            .remove(&window_type.inner_window().window().id());

        self.window_pool.release(window_type.into_inner_window());
    }

    fn spawn_image(
        &mut self,
        id: ItemId,
        media_id: u64,
        opts: PopupSpawnOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        tracing::info!("Items: {}", self.items.len());
        let window_opts = self.finalize_window_opts(opts, event_loop)?;

        let physical_size =
            LogicalSize::new(window_opts.popup_opts.width, window_opts.popup_opts.height)
                .to_physical::<u32>(window_opts.monitor.scale_factor);

        let props = Self::window_props(id, &window_opts);
        let requirement = self.new_requirement(MediaRequirement::Image {
            media_id,
            width: physical_size.width,
            height: physical_size.height,
        });
        self.insert_pending_item(
            id,
            PendingItemOpts::Image {
                window: PendingWindowOpts::new(window_opts),
                image: requirement.id,
            },
            vec![requirement],
        )?;

        Ok(props)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_video(
        &mut self,
        id: ItemId,
        media_id: u64,
        loop_video: bool,
        audio: bool,
        volume: f32,
        opts: PopupSpawnOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let volume = volume * self.config.volume.video;

        let window_opts = self.finalize_window_opts(opts, event_loop)?;

        let loop_video = Arc::new(AtomicBool::new(loop_video));

        let props = Self::window_props(id, &window_opts);
        let requirement = self.new_requirement(MediaRequirement::Video {
            media_id,
            loop_video: loop_video.clone(),
            play_audio: audio,
            volume,
        });
        self.insert_pending_item(
            id,
            PendingItemOpts::Video {
                window: PendingWindowOpts::new(window_opts),
                loop_video: loop_video.clone(),
                paused: false,
                volume,
                video: requirement.id,
            },
            vec![requirement],
        )?;

        Ok(props)
    }

    /// Spawns a dialog. Its image requirements are decoded concurrently; a dialog without image
    /// requirements is built immediately.
    fn spawn_dialog(
        &mut self,
        id: ItemId,
        elements: Vec<DialogElement>,
        window_opts: PopupSpawnOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        let resolved = self.finalize_window_opts(window_opts, event_loop)?;
        let props = Self::window_props(id, &resolved);

        let mut requirements = Vec::new();
        let pending_elements = elements
            .into_iter()
            .map(|element| {
                element.map_image(|image| {
                    let MediaData::Image { width, height, .. } = image.media_data else {
                        unreachable!("dialog images are validated by the Lua API")
                    };
                    let requirement = self.new_requirement(MediaRequirement::Image {
                        media_id: image.id,
                        width,
                        height,
                    });
                    let requirement_id = requirement.id;
                    requirements.push(requirement);
                    requirement_id
                })
            })
            .collect();
        let has_requirements = !requirements.is_empty();

        self.insert_pending_item(
            id,
            PendingItemOpts::Dialog {
                window: PendingWindowOpts::new(resolved),
                elements: pending_elements,
            },
            requirements,
        )?;

        if !has_requirements {
            let item = self
                .remove_pending_item(id)
                .ok_or(LewdwareError::Internal("Pending dialog not found"))?;
            self.finalize_pending_item(id, item, event_loop)?;
        }

        Ok(props)
    }

    fn spawn_text(
        &mut self,
        id: ItemId,
        text: String,
        style: TextStyle,
        window_opts: PopupSpawnOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
        // Font size is already resolved to a concrete `FontSize::Value` on the Lua thread (see
        // `spawn_text_popup`), since that's where the monitor height (the basis for
        // `FontSize::Percent`) is known now.
        let resolved = self.finalize_window_opts(window_opts, event_loop)?;

        let props = Self::window_props(id, &resolved);
        self.insert_pending_item(
            id,
            PendingItemOpts::Text {
                window: PendingWindowOpts::new(resolved),
                text,
                style,
            },
            Vec::new(),
        )?;
        let item = self
            .remove_pending_item(id)
            .ok_or(LewdwareError::Internal("Pending text window not found"))?;
        self.finalize_pending_item(id, item, event_loop)?;

        Ok(props)
    }

    fn spawn_audio(
        &mut self,
        id: ItemId,
        media_id: u64,
        loop_audio: bool,
        volume: f32,
    ) -> Result<ItemId> {
        // See `spawn_video`'s equivalent comment -- same reasoning, `audio` channel instead.
        let volume = volume * self.config.volume.audio;
        let requirement = self.new_requirement(MediaRequirement::Audio {
            item_id: id,
            media_id,
            loop_audio,
            volume,
        });

        self.insert_pending_item(
            id,
            PendingItemOpts::Audio {
                paused: false,
                volume,
                stopped: false,
                audio: requirement.id,
            },
            vec![requirement],
        )?;

        Ok(id)
    }

    fn set_wallpaper(&mut self, file: FileOrPath, mode: Option<WallpaperMode>) -> Result<bool> {
        if !self.config.capabilities.wallpaper {
            return Ok(false);
        }

        let path = file.path().to_str().ok_or(LewdwareError::Internal(
            "Tempfile does not have valid UTF-8 path",
        ))?;

        if wallpaper::set_from_path(path).is_err() {
            return Ok(false);
        }

        if let Some(mode) = mode {
            let mode = match mode {
                WallpaperMode::Center => wallpaper::Mode::Center,
                WallpaperMode::Crop => wallpaper::Mode::Crop,
                WallpaperMode::Fit => wallpaper::Mode::Fit,
                WallpaperMode::Span => wallpaper::Mode::Span,
                WallpaperMode::Stretch => wallpaper::Mode::Stretch,
                WallpaperMode::Tile => wallpaper::Mode::Tile,
            };

            if wallpaper::set_mode(mode).is_err() {
                return Ok(false);
            }
        }

        Ok(true)
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

    fn open_link(&self, url: String) -> Result<bool> {
        if !self.config.capabilities.open_link {
            return Ok(false);
        }

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

        Ok(webbrowser::open(url.as_str()).is_ok())
    }

    fn show_notification(&self, notification: Notification) -> Result<bool> {
        if !self.config.capabilities.notify {
            return Ok(false);
        }

        let mut notification_builder = notify_rust::Notification::new();

        notification_builder.body(&notification.body);

        if let Some(summary) = notification.summary {
            notification_builder.summary(&summary);
        }

        Ok(notification_builder.show().is_ok())
    }

    fn process_lua_request(&mut self, request: LuaRequest, event_loop: &ActiveEventLoop) -> bool {
        if !match request {
            LuaRequest::SpawnImage {
                id,
                media_id,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_image(id, media_id, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnVideo {
                id,
                media_id,
                loop_video,
                audio,
                volume,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_video(
                    id,
                    media_id,
                    loop_video,
                    audio,
                    volume,
                    window_opts,
                    event_loop,
                ))
                .is_ok(),
            LuaRequest::SpawnDialog {
                id,
                elements,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_dialog(id, elements, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnText {
                id,
                text,
                style,
                window_opts,
                tx,
            } => tx
                .send(self.spawn_text(id, text, style, window_opts, event_loop))
                .is_ok(),
            LuaRequest::SpawnAudio {
                id,
                media_id,
                loop_audio,
                volume,
                tx,
            } => tx
                .send(self.spawn_audio(id, media_id, loop_audio, volume))
                .is_ok(),
            LuaRequest::SetWallpaper { file, mode, tx } => {
                tx.send(self.set_wallpaper(file, mode)).is_ok()
            }
            LuaRequest::ResetWallpaper { tx } => {
                self.reset_wallpaper();
                tx.send(())
            }
            .is_ok(),
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
            LuaRequest::ItemAction {
                id,
                action: ItemAction::Window(action),
            } => {
                if let Entry::Occupied(mut entry) = self.items.entry(id) {
                    match action {
                        WindowAction::CloseWindow { tx } => {
                            match entry.remove() {
                                ItemSlot::Window(window_type) => self.close_window(window_type),
                                // No real window was ever created; synthesize the close event
                                // `InnerWindow::Drop` would otherwise have sent.
                                ItemSlot::Pending(item) => {
                                    for requirement in &item.requirements {
                                        self.requirement_owners.remove(&requirement.id);
                                    }
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
                                ItemSlot::Audio(_) => unreachable!("window action for audio item"),
                            }
                            tx.send(()).is_ok()
                        }
                        WindowAction::PauseVideo { tx } => tx
                            .send(match entry.get_mut() {
                                ItemSlot::Window(WindowType::Video(video_window)) => {
                                    video_window.pause();
                                    Ok(())
                                }
                                ItemSlot::Pending(PendingItem {
                                    opts: PendingItemOpts::Video { paused, .. },
                                    ..
                                }) => {
                                    *paused = true;
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::PlayVideo { tx } => tx
                            .send(match entry.get_mut() {
                                ItemSlot::Window(WindowType::Video(video_window)) => {
                                    video_window.play();
                                    Ok(())
                                }
                                ItemSlot::Pending(PendingItem {
                                    opts: PendingItemOpts::Video { paused, .. },
                                    ..
                                }) => {
                                    *paused = false;
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::SetVideoVolume { tx, volume } => {
                            let volume = volume * self.config.volume.video;
                            tx.send(match entry.get_mut() {
                                ItemSlot::Window(WindowType::Video(video_window)) => {
                                    video_window.set_volume(volume);
                                    Ok(())
                                }
                                ItemSlot::Pending(PendingItem {
                                    opts:
                                        PendingItemOpts::Video {
                                            volume: pending_volume,
                                            ..
                                        },
                                    ..
                                }) => {
                                    *pending_volume = volume;
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok()
                        }
                        WindowAction::SetVideoLoop { tx, loop_video } => tx
                            .send(match entry.get_mut() {
                                ItemSlot::Window(WindowType::Video(video_window)) => {
                                    video_window.set_loop(loop_video);
                                    Ok(())
                                }
                                ItemSlot::Pending(PendingItem {
                                    opts:
                                        PendingItemOpts::Video {
                                            loop_video: pending,
                                            ..
                                        },
                                    ..
                                }) => {
                                    pending.store(loop_video, Ordering::Relaxed);
                                    Ok(())
                                }
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::Move { id, tx, opts } => tx
                            .send(match entry.get_mut() {
                                ItemSlot::Window(window_type) => {
                                    window_type.inner_window_mut().start_move(id, opts)
                                }
                                ItemSlot::Pending(item) => item
                                    .opts
                                    .window_mut()
                                    .map(|pending| pending.pending_move = Some((id, opts)))
                                    .ok_or(LewdwareError::Internal("Invalid window type")),
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            })
                            .is_ok(),
                        WindowAction::SetText { tx, text } => tx
                            .send(match entry.get_mut() {
                                ItemSlot::Window(WindowType::Text(text_window)) => match text {
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
                        WindowAction::UpdateDialogElement { tx, id, props } => {
                            let updated = match entry.get_mut() {
                                ItemSlot::Window(WindowType::Dialog(dialog)) => {
                                    dialog.update_element(&id, props)
                                }
                                _ => false,
                            };
                            tx.send(updated).is_ok()
                        }
                        WindowAction::GetDialogValues { tx } => {
                            let values = match entry.get() {
                                ItemSlot::Window(WindowType::Dialog(dialog)) => dialog.values(),
                                _ => HashMap::new(),
                            };
                            tx.send(values).is_ok()
                        }
                        WindowAction::GetDialogValue { id, tx } => {
                            let value = match entry.get() {
                                ItemSlot::Window(WindowType::Dialog(dialog)) => dialog.value(&id),
                                _ => None,
                            };
                            tx.send(value).is_ok()
                        }
                        WindowAction::SetTitle { tx, title } => {
                            match entry.get_mut() {
                                ItemSlot::Window(window_type) => {
                                    window_type.inner_window_mut().set_title(title)
                                }
                                ItemSlot::Pending(item) => {
                                    if let Some(pending) = item.opts.window_mut() {
                                        pending.title = title;
                                    }
                                }
                                _ => {}
                            }
                            tx.send(()).is_ok()
                        }
                        WindowAction::SetOpacity { tx, opacity } => {
                            let result = match entry.get_mut() {
                                ItemSlot::Window(window_type) => {
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
                                ItemSlot::Pending(item) => match item.opts.window_mut() {
                                    Some(pending)
                                        if opacity != 1.0
                                            && !pending.window.popup_opts.transparent =>
                                    {
                                        Err(LewdwareError::Internal(
                                            "Cannot change opacity on a non-transparent window. \
                                             Ensure the window is created with transparency \
                                             enabled: use media that has an alpha channel, set \
                                             `opacity` to a value below 1.0, or set \
                                             `transparent = true`.",
                                        ))
                                    }
                                    Some(pending) => {
                                        pending.opacity = opacity;
                                        Ok(())
                                    }
                                    None => Err(LewdwareError::Internal("Invalid window type")),
                                },
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            };
                            tx.send(result).is_ok()
                        }
                        WindowAction::Fade { id, tx, opts } => {
                            let result = match entry.get_mut() {
                                ItemSlot::Window(window_type) => {
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
                                ItemSlot::Pending(item) => match item.opts.window_mut() {
                                    Some(pending) if !pending.window.popup_opts.transparent => {
                                        Err(LewdwareError::Internal(
                                            "Cannot fade a non-transparent window. Ensure the \
                                             window is created with transparency enabled: use \
                                             media that has an alpha channel, set `opacity` to \
                                             a value below 1.0, or set `transparent = true`.",
                                        ))
                                    }
                                    Some(pending) => {
                                        pending.pending_fade = Some((id, opts));
                                        Ok(())
                                    }
                                    None => Err(LewdwareError::Internal("Invalid window type")),
                                },
                                _ => Err(LewdwareError::Internal("Invalid window type")),
                            };
                            tx.send(result).is_ok()
                        }
                    }
                } else {
                    true
                }
            }
            LuaRequest::ItemAction {
                id,
                action: ItemAction::Audio(action),
            } => {
                if let Entry::Occupied(mut entry) = self.items.entry(id) {
                    match action {
                        AudioAction::Pause { tx } => {
                            match entry.get_mut() {
                                ItemSlot::Audio(player) => player.pause(),
                                ItemSlot::Pending(PendingItem {
                                    opts: PendingItemOpts::Audio { paused, .. },
                                    ..
                                }) => *paused = true,
                                _ => {}
                            }
                            tx.send(()).is_ok()
                        }
                        AudioAction::Play { tx } => {
                            match entry.get_mut() {
                                ItemSlot::Audio(player) => player.play(),
                                ItemSlot::Pending(PendingItem {
                                    opts: PendingItemOpts::Audio { paused, .. },
                                    ..
                                }) => *paused = false,
                                _ => {}
                            }
                            tx.send(()).is_ok()
                        }
                        AudioAction::SetVolume { tx, volume } => {
                            let volume = volume * self.config.volume.audio;
                            match entry.get_mut() {
                                ItemSlot::Audio(player) => player.set_volume(volume),
                                ItemSlot::Pending(PendingItem {
                                    opts:
                                        PendingItemOpts::Audio {
                                            volume: pending_volume,
                                            ..
                                        },
                                    ..
                                }) => *pending_volume = volume,
                                _ => {}
                            }
                            tx.send(()).is_ok()
                        }
                        AudioAction::Stop { tx } => {
                            match entry.get_mut() {
                                ItemSlot::Audio(player) => player.stop(),
                                ItemSlot::Pending(PendingItem {
                                    opts: PendingItemOpts::Audio { stopped, .. },
                                    ..
                                }) => *stopped = true,
                                _ => {}
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
    fn pending_item_failed(&mut self, id: ItemId, context: &str, err: impl std::fmt::Display) {
        tracing::error!("{context}: {err}");
        let Some(item) = self.remove_pending_item(id) else {
            return;
        };
        let event = if item.opts.is_window() {
            lua::Event::WindowClosed { id }
        } else {
            lua::Event::AudioFinish { id }
        };
        if self.lua_event_tx.send(event).is_err() {
            tracing::debug!("Couldn't send pending-item failure event: Lua thread has shut down");
        }
    }

    /// Apply mutations made while a window item's media requirements were pending.
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

    fn handle_media_resolved(
        &mut self,
        requirement_id: RequirementId,
        result: std::result::Result<ResolvedMedia, MediaError>,
        event_loop: &ActiveEventLoop,
    ) {
        let Some(item_id) = self.requirement_owners.remove(&requirement_id) else {
            return;
        };

        let media = match result {
            Ok(media) => media,
            Err(err) => return self.pending_item_failed(item_id, "Failed to resolve media", err),
        };

        let resolved = match self.items.get_mut(&item_id) {
            Some(ItemSlot::Pending(item)) => item.resolve(requirement_id, media),
            _ => return,
        };
        if let Err(err) = resolved {
            return self.pending_item_failed(item_id, "Failed to resolve media", err);
        }

        let complete = matches!(
            self.items.get(&item_id),
            Some(ItemSlot::Pending(item)) if item.is_resolved()
        );
        if complete && let Some(item) = self.remove_pending_item(item_id) {
            let is_window = item.opts.is_window();
            if let Err(err) = self.finalize_pending_item(item_id, item, event_loop) {
                tracing::error!("Failed to finalize pending item: {err}");
                self.window_ids.retain(|_, id| *id != item_id);
                let event = if is_window {
                    lua::Event::WindowClosed { id: item_id }
                } else {
                    lua::Event::AudioFinish { id: item_id }
                };
                let _ = self.lua_event_tx.send(event);
            }
        }
    }

    fn finalize_pending_item(
        &mut self,
        id: ItemId,
        item: PendingItem,
        event_loop: &ActiveEventLoop,
    ) -> Result<()> {
        let (opts, mut requirements) = item.into_resolved()?;
        match opts {
            PendingItemOpts::Audio {
                paused,
                volume,
                stopped,
                audio,
            } => {
                let player = requirements.take_audio(audio)?;
                player.set_volume(volume);
                if stopped {
                    player.stop();
                } else if !paused {
                    player.play();
                }
                self.items.insert(id, ItemSlot::Audio(player));
            }
            PendingItemOpts::Image {
                window:
                    PendingWindowOpts {
                        window,
                        opacity,
                        title,
                        pending_move,
                        pending_fade,
                    },
                image,
            } => {
                let inner_window = self.create_window(id, window, event_loop)?;
                let data = requirements.take_image(image)?;
                let mut image_window =
                    ImageWindow::new(inner_window, data).map_err(LewdwareError::WindowError)?;
                Self::apply_pending_mutations(
                    &mut image_window.inner_window,
                    opacity,
                    title,
                    pending_move,
                    pending_fade,
                );
                let idx = image_window.draw().unwrap_or_else(|err| {
                    tracing::warn!("image pre-draw failed: {err}");
                    None
                });
                image_window.inner_window.gpu_sync(idx);
                image_window.inner_window.show();
                self.items
                    .insert(id, ItemSlot::Window(WindowType::Image(image_window)));
            }
            PendingItemOpts::Video {
                window:
                    PendingWindowOpts {
                        window,
                        opacity,
                        title,
                        pending_move,
                        pending_fade,
                    },
                video,
                paused,
                volume,
                ..
            } => {
                let inner_window = self.create_window(id, window, event_loop)?;
                let decoder = requirements.take_video(video)?;
                let mut video_window =
                    VideoWindow::new(inner_window, decoder).map_err(LewdwareError::WindowError)?;
                Self::apply_pending_mutations(
                    &mut video_window.inner_window,
                    opacity,
                    title,
                    pending_move,
                    pending_fade,
                );
                if paused {
                    video_window.pause();
                }
                video_window.set_volume(volume);
                if let Err(err) = video_window.inner_window.pre_show() {
                    tracing::warn!("video pre-show failed: {err}");
                }
                video_window.inner_window.show();
                self.items
                    .insert(id, ItemSlot::Window(WindowType::Video(video_window)));
            }
            PendingItemOpts::Dialog {
                window:
                    PendingWindowOpts {
                        window,
                        opacity,
                        title,
                        pending_move,
                        pending_fade,
                    },
                elements,
            } => {
                let inner_window = self.create_window(id, window, event_loop)?;
                let mut dialog_window = DialogWindow::new(inner_window, elements, |image| {
                    requirements.take_image(image).map_err(Into::into)
                })
                .map_err(LewdwareError::WindowError)?;
                Self::apply_pending_mutations(
                    &mut dialog_window.inner_window,
                    opacity,
                    title,
                    pending_move,
                    pending_fade,
                );
                if let Err(err) = dialog_window.inner_window.pre_show() {
                    tracing::warn!("dialog pre-show failed: {err}");
                }
                dialog_window.inner_window.show();
                self.items
                    .insert(id, ItemSlot::Window(WindowType::Dialog(dialog_window)));
            }
            PendingItemOpts::Text {
                window:
                    PendingWindowOpts {
                        window,
                        opacity,
                        title,
                        pending_move,
                        pending_fade,
                    },
                text,
                style,
            } => {
                let inner_window = self.create_window(id, window, event_loop)?;
                let mut text_window = TextWindow::new(inner_window, text, style)
                    .map_err(LewdwareError::WindowError)?;
                Self::apply_pending_mutations(
                    &mut text_window.inner_window,
                    opacity,
                    title,
                    pending_move,
                    pending_fade,
                );
                if let Err(err) = text_window.inner_window.pre_show() {
                    tracing::warn!("text pre-show failed: {err}");
                }
                text_window.inner_window.show();
                self.items
                    .insert(id, ItemSlot::Window(WindowType::Text(text_window)));
            }
        }
        Ok(())
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
        let Some(&popup_id) = self.window_ids.get(&window_id) else {
            return;
        };

        if let Entry::Occupied(mut entry) = self.items.entry(popup_id) {
            let ItemSlot::Window(window_type) = entry.get_mut() else {
                return;
            };

            // A compositor/OS-driven redraw can still arrive independently of our requests. If
            // it satisfies a dirty window here, don't render that window a second time when this
            // batch reaches `about_to_wait`.
            if event == WindowEvent::RedrawRequested {
                window_type.inner_window().take_redraw_requested();
            }

            let mut video_finished = false;

            match window_type {
                WindowType::Image(window) => {
                    if event == WindowEvent::RedrawRequested
                        && let Err(err) = window.draw()
                    {
                        tracing::error!("Error drawing image window: {}", err);
                    }
                }
                WindowType::Video(window) => {
                    // Windows drains this request directly in `about_to_wait`; other platforms
                    // retain winit's normal `RedrawRequested` delivery path.
                    if !cfg!(target_os = "windows") && event == WindowEvent::RedrawRequested {
                        match window.update() {
                            Ok(finished) => video_finished = finished,
                            Err(err) => tracing::error!("Error updating video window: {err}"),
                        }
                    }
                }
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
                WindowType::Dialog(window) => match &event {
                    WindowEvent::RedrawRequested => {
                        window.render().unwrap_or_else(|err| {
                            tracing::error!("Error rendering dialog window: {}", err);
                        });
                    }
                    event => {
                        window.handle_event(event);
                    }
                },
            }

            if video_finished {
                let ItemSlot::Window(window_type) = entry.remove() else {
                    unreachable!("only ready video windows can finish")
                };
                self.close_window(window_type);
                return;
            }

            // Global event handling
            match event {
                WindowEvent::CloseRequested => {
                    if let ItemSlot::Window(window_type) = entry.remove() {
                        self.close_window(window_type);
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let ItemSlot::Window(window_type) = entry.get_mut() {
                        window_type.inner_window_mut().handle_cursor_moved(position);
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    if let ItemSlot::Window(window_type) = entry.get_mut() {
                        window_type.inner_window_mut().handle_cursor_left();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    if let ItemSlot::Window(window_type) = entry.get_mut() {
                        window_type.inner_window_mut().handle_mouse_down();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } => {
                    let click_result = match entry.get_mut() {
                        ItemSlot::Window(window_type) => {
                            Some(window_type.inner_window_mut().handle_mouse_up())
                        }
                        ItemSlot::Pending(_) => None,
                        ItemSlot::Audio(_) => None,
                    };

                    if let Some(click_result) = click_result {
                        if click_result.content_click {
                            match entry.get_mut() {
                                // Dialogs only fire `Window:on_click()` for clicks not consumed
                                // by a button/input, which isn't known until the next render pass
                                // (egui processes the click asynchronously) -- see
                                // `mark_pending_content_click`.
                                ItemSlot::Window(WindowType::Dialog(dialog_window)) => {
                                    dialog_window.mark_pending_content_click();
                                }
                                ItemSlot::Window(window_type) => {
                                    let inner_window = window_type.inner_window();
                                    if inner_window
                                        .lua_event_tx()
                                        .send(lua::Event::WindowClicked {
                                            id: inner_window.popup_id(),
                                        })
                                        .is_err()
                                    {
                                        tracing::debug!(
                                            "Couldn't send WindowClicked event: Lua thread has \
                                             shut down"
                                        );
                                    }
                                }
                                ItemSlot::Pending(_) => {}
                                ItemSlot::Audio(_) => {}
                            }
                        }

                        if click_result.should_close
                            && let ItemSlot::Window(window_type) = entry.remove()
                        {
                            self.close_window(window_type);
                        }
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
            UserEvent::RedrawRequested => {
                // Allow requests made while the ensuing batch is being rendered to queue the
                // next wake-up. The per-window flags themselves are drained in `about_to_wait`.
                self.redraw_wakeup_pending.store(false, Ordering::Release);
            }
            UserEvent::AudioFinish { id } => {
                let removed = if matches!(self.items.get(&id), Some(ItemSlot::Pending(_))) {
                    self.remove_pending_item(id).is_some()
                } else {
                    self.items.remove(&id).is_some()
                };
                if removed && let Err(err) = self.lua_event_tx.send(lua::Event::AudioFinish { id })
                {
                    tracing::error!("{err}");
                }
            }
            UserEvent::MediaResolved {
                requirement_id,
                result,
            } => {
                self.handle_media_resolved(requirement_id, result, event_loop);
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut poll = false;
        let mut finished_videos = Vec::new();

        for (id, slot) in self.items.iter_mut() {
            let ItemSlot::Window(window) = slot else {
                continue;
            };

            if matches!(window, WindowType::Video(_)) {
                window.inner_window().request_redraw();
                poll = true;
            }

            let redraw_requested = window.inner_window().take_redraw_requested();

            // Avoid the Win32 `WM_PAINT`/`RedrawRequested` path for every kind of popup, not just
            // video. Each window is rendered at most once per event-loop batch, even if several
            // callers dirtied it before this callback.
            if cfg!(target_os = "windows") && redraw_requested {
                match window {
                    WindowType::Image(image_window) => {
                        if let Err(err) = image_window.draw() {
                            tracing::error!("Error drawing image window: {err}");
                        }
                    }
                    WindowType::Text(text_window) => {
                        if let Err(err) = text_window.render() {
                            tracing::error!("Error rendering text window: {err}");
                        }
                    }
                    WindowType::Dialog(dialog_window) => {
                        if let Err(err) = dialog_window.render() {
                            tracing::error!("Error rendering dialog window: {err}");
                        }
                    }
                    WindowType::Video(video_window) => match video_window.update() {
                        Ok(true) => finished_videos.push(*id),
                        Ok(false) => {}
                        Err(err) => tracing::error!("Error updating video window: {err}"),
                    },
                }
            }

            if window.inner_window().is_moving() {
                window.inner_window_mut().update_position();
                poll = true;
            }
            if window.inner_window().is_fading() {
                window.inner_window_mut().update_fade();
                poll = true;
            }
        }

        for id in finished_videos {
            if let Some(ItemSlot::Window(window_type)) = self.items.remove(&id) {
                self.close_window(window_type);
            }
        }

        if poll {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}
