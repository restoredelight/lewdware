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
use crate::window::{Popup, RenderOutcome, RenderTarget, WindowOpts, WindowPool, WindowState};

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

#[allow(clippy::large_enum_variant)]
enum ItemSlot {
    Pending(PendingItem),
    Window(Popup),
    Audio(AudioPlayer),
}

pub enum UserEvent {
    Exit,
    LuaRequest,
    #[allow(unused)]
    RedrawRequested,
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

    fn insert_pending_item(
        &mut self,
        id: ItemId,
        opts: PendingItemOpts,
        requirements: Vec<Requirement>,
        event_loop: &ActiveEventLoop,
    ) -> Result<()> {
        let requests = requirements
            .iter()
            .map(|requirement| match &requirement.state {
                RequirementState::Pending(media) => Ok((requirement.id, media.clone())),
                RequirementState::Resolved(_) => Err(LewdwareError::Internal(
                    "new pending items cannot contain resolved requirements",
                )),
            })
            .collect::<Result<Vec<_>>>()?;

        match self.items.entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(ItemSlot::Pending(PendingItem { opts, requirements }));
            }
            Entry::Occupied(_) => return Err(LewdwareError::Internal("Duplicate item id")),
        }

        for (requirement_id, media) in requests {
            self.requirement_owners.insert(requirement_id, id);
            if let Err(err) = self.media_manager.resolve(requirement_id, media) {
                self.remove_pending_item(id);
                return Err(err.into());
            }
        }

        self.try_finalize_item(event_loop, id);

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

    fn remove_item(&mut self, id: ItemId) {
        let Some(item) = self.items.remove(&id) else {
            return;
        };

        if let ItemSlot::Pending(pending) = &item {
            for requirement in &pending.requirements {
                self.requirement_owners.remove(&requirement.id);
            }
        }

        let event = match item {
            ItemSlot::Pending(pending) if pending.opts.is_window() => {
                lua::Event::WindowClosed { id }
            }
            ItemSlot::Window(window) => {
                self.close_window(window);
                lua::Event::WindowClosed { id }
            }
            ItemSlot::Pending(_) | ItemSlot::Audio(_) => lua::Event::AudioFinish { id },
        };

        if self.lua_event_tx.send(event).is_err() {
            tracing::debug!("Couldn't send item completion event: Lua thread has shut down");
        }
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
    ) -> Result<(WindowState, RenderTarget)> {
        let window = self
            .window_pool
            .acquire(&opts, event_loop)
            .map_err(LewdwareError::WindowError)?;

        self.window_ids.insert(window.id(), popup_id);

        let mut state = WindowState::new(
            window.clone(),
            &opts,
            self.lua_event_tx.clone(),
            popup_id,
            self.event_loop_proxy.clone(),
            self.redraw_wakeup_pending.clone(),
        );

        let target = RenderTarget::new(
            window,
            &opts,
            self.wgpu_state.clone(),
            state.redraw_requester(),
        )
        .map_err(LewdwareError::WindowError)?;

        state.attach_decorations(&target);

        Ok((state, target))
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

    fn close_window(&mut self, popup: Popup) {
        self.window_ids.remove(&popup.state.window().id());

        let (state, target) = popup.into_parts();
        self.window_pool.release(state, target);
    }

    fn spawn_image(
        &mut self,
        id: ItemId,
        media_id: u64,
        opts: PopupSpawnOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<WindowProps> {
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
            event_loop,
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
            event_loop,
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
                element.try_map_image(|image| {
                    let MediaData::Image { width, height, .. } = image.media_data else {
                        return Err(LewdwareError::Internal(
                            "dialog images are validated by the Lua API",
                        ));
                    };

                    let requirement = self.new_requirement(MediaRequirement::Image {
                        media_id: image.id,
                        width,
                        height,
                    });

                    let requirement_id = requirement.id;
                    requirements.push(requirement);

                    Ok(requirement_id)
                })
            })
            .collect::<Result<_>>()?;

        self.insert_pending_item(
            id,
            PendingItemOpts::Dialog {
                window: PendingWindowOpts::new(resolved),
                elements: pending_elements,
            },
            requirements,
            event_loop,
        )?;

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
            event_loop,
        )?;

        Ok(props)
    }

    fn spawn_audio(
        &mut self,
        id: ItemId,
        media_id: u64,
        loop_audio: bool,
        volume: f32,
        event_loop: &ActiveEventLoop,
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
                audio: requirement.id,
            },
            vec![requirement],
            event_loop,
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
                .send(self.spawn_audio(id, media_id, loop_audio, volume, event_loop))
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
                            self.remove_item(id);
                            tx.send(()).is_ok()
                        }
                        WindowAction::PauseVideo { tx } => tx
                            .send(match entry.get_mut() {
                                ItemSlot::Window(popup) => {
                                    if popup.pause() {
                                        Ok(())
                                    } else {
                                        Err(LewdwareError::Internal("Invalid window type"))
                                    }
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
                                ItemSlot::Window(popup) => {
                                    if popup.play() {
                                        Ok(())
                                    } else {
                                        Err(LewdwareError::Internal("Invalid window type"))
                                    }
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
                                ItemSlot::Window(popup) => {
                                    if popup.set_volume(volume) {
                                        Ok(())
                                    } else {
                                        Err(LewdwareError::Internal("Invalid window type"))
                                    }
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
                                ItemSlot::Window(popup) => {
                                    if popup.set_loop(loop_video) {
                                        Ok(())
                                    } else {
                                        Err(LewdwareError::Internal("Invalid window type"))
                                    }
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
                                ItemSlot::Window(popup) => popup.state.start_move(id, opts),
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
                                ItemSlot::Window(popup) => match text {
                                    Some(text) => {
                                        if popup.set_text(text) {
                                            Ok(())
                                        } else {
                                            Err(LewdwareError::Internal("Invalid window type"))
                                        }
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
                                ItemSlot::Window(popup) => popup.update_element(&id, props),
                                _ => false,
                            };
                            tx.send(updated).is_ok()
                        }
                        WindowAction::GetDialogValues { tx } => {
                            let values = match entry.get() {
                                ItemSlot::Window(popup) => popup.values(),
                                _ => HashMap::new(),
                            };
                            tx.send(values).is_ok()
                        }
                        WindowAction::GetDialogValue { id, tx } => {
                            let value = match entry.get() {
                                ItemSlot::Window(popup) => popup.value(&id),
                                _ => None,
                            };
                            tx.send(value).is_ok()
                        }
                        WindowAction::SetTitle { tx, title } => {
                            match entry.get_mut() {
                                ItemSlot::Window(popup) => popup.state.set_title(title),
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
                                ItemSlot::Window(popup) => {
                                    if opacity != 1.0 && !popup.state.transparent() {
                                        Err(LewdwareError::Internal(
                                            "Cannot change opacity on a non-transparent window. \
                                             Ensure the window is created with transparency \
                                             enabled: use media that has an alpha channel, set \
                                             `opacity` to a value below 1.0, or set \
                                             `transparent = true`.",
                                        ))
                                    } else {
                                        popup.state.set_opacity(opacity);
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
                                ItemSlot::Window(popup) => {
                                    if !popup.state.transparent() {
                                        Err(LewdwareError::Internal(
                                            "Cannot fade a non-transparent window. Ensure the \
                                             window is created with transparency enabled: use \
                                             media that has an alpha channel, set `opacity` to \
                                             a value below 1.0, or set `transparent = true`.",
                                        ))
                                    } else {
                                        popup.state.start_fade(id, opts)
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
                                ItemSlot::Pending(_) => {}
                                _ => {}
                            }

                            self.remove_item(id);
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
        self.remove_item(id);
    }

    /// Apply mutations made while a window item's media requirements were pending.
    fn apply_pending_mutations(
        state: &mut WindowState,
        opacity: f32,
        title: Option<String>,
        pending_move: Option<(u64, MoveOpts)>,
        pending_fade: Option<(u64, FadeOpts)>,
    ) {
        state.set_opacity(opacity);
        state.set_title(title);
        if let Some((move_id, opts)) = pending_move {
            let _ = state.start_move(move_id, opts);
        }
        if let Some((fade_id, opts)) = pending_fade {
            let _ = state.start_fade(fade_id, opts);
        }
    }

    fn handle_media_resolved(
        &mut self,
        requirement_id: RequirementId,
        result: Result<ResolvedMedia, MediaError>,
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

        self.try_finalize_item(event_loop, item_id);
    }

    fn try_finalize_item(&mut self, event_loop: &ActiveEventLoop, item_id: ItemId) {
        if self.is_resolved(item_id)
            && let Some(item) = self.remove_pending_item(item_id)
        {
            let is_window = item.opts.is_window();
            if let Err(err) = self.finalize_pending_item(item_id, item, event_loop) {
                tracing::error!("Failed to finalize item: {err}");
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

    fn is_resolved(&self, item_id: ItemId) -> bool {
        matches!(
            self.items.get(&item_id),
            Some(ItemSlot::Pending(item)) if item.is_resolved()
        )
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
                audio,
            } => {
                let player = requirements.take_audio(audio)?;
                player.set_volume(volume);
                if !paused {
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
                let (state, target) = self.create_window(id, window, event_loop)?;
                let data = requirements.take_image(image)?;
                let mut popup =
                    Popup::new_image(state, target, data).map_err(LewdwareError::WindowError)?;
                Self::apply_pending_mutations(
                    &mut popup.state,
                    opacity,
                    title,
                    pending_move,
                    pending_fade,
                );
                let outcome = popup.render().unwrap_or_else(|err| {
                    tracing::warn!("image pre-draw failed: {err}");
                    RenderOutcome::default()
                });
                popup.target.gpu_sync(outcome.submission);
                popup.state.show();
                self.items.insert(id, ItemSlot::Window(popup));
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
                let (state, target) = self.create_window(id, window, event_loop)?;
                let decoder = requirements.take_video(video)?;
                let mut popup =
                    Popup::new_video(state, target, decoder).map_err(LewdwareError::WindowError)?;
                Self::apply_pending_mutations(
                    &mut popup.state,
                    opacity,
                    title,
                    pending_move,
                    pending_fade,
                );
                if paused {
                    popup.pause();
                }
                popup.set_volume(volume);
                if let Err(err) = popup.target.pre_show() {
                    tracing::warn!("video pre-show failed: {err}");
                }
                popup.state.show();
                self.items.insert(id, ItemSlot::Window(popup));
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
                let (state, target) = self.create_window(id, window, event_loop)?;
                let mut popup = Popup::new_dialog(state, target, elements, |image| {
                    requirements.take_image(image).map_err(Into::into)
                })
                .map_err(LewdwareError::WindowError)?;
                Self::apply_pending_mutations(
                    &mut popup.state,
                    opacity,
                    title,
                    pending_move,
                    pending_fade,
                );
                if let Err(err) = popup.target.pre_show() {
                    tracing::warn!("dialog pre-show failed: {err}");
                }
                popup.state.show();
                self.items.insert(id, ItemSlot::Window(popup));
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
                let (state, target) = self.create_window(id, window, event_loop)?;
                let mut popup = Popup::new_text(state, target, text, style)
                    .map_err(LewdwareError::WindowError)?;
                Self::apply_pending_mutations(
                    &mut popup.state,
                    opacity,
                    title,
                    pending_move,
                    pending_fade,
                );
                if let Err(err) = popup.target.pre_show() {
                    tracing::warn!("text pre-show failed: {err}");
                }
                popup.state.show();
                self.items.insert(id, ItemSlot::Window(popup));
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
            let ItemSlot::Window(popup) = entry.get_mut() else {
                return;
            };

            // A compositor/OS-driven redraw can still arrive independently of our requests. If
            // it satisfies a dirty window here, don't render that window a second time when this
            // batch reaches `about_to_wait`.
            if event == WindowEvent::RedrawRequested {
                popup.state.take_redraw_requested();
            }

            let mut content_finished = false;

            if event == WindowEvent::RedrawRequested {
                // Video is driven from `about_to_wait` on Windows instead; see the comment there.
                if !(cfg!(target_os = "windows") && popup.is_video()) {
                    match popup.render() {
                        Ok(outcome) => content_finished = outcome.finished,
                        Err(err) => tracing::error!("Error rendering window: {err}"),
                    }
                }
            } else {
                popup.handle_event(&event);
            }

            if content_finished {
                self.remove_item(popup_id);

                return;
            }

            // Global event handling
            match event {
                WindowEvent::CloseRequested => {
                    self.remove_item(popup_id);
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let ItemSlot::Window(popup) = entry.get_mut() {
                        popup.state.handle_cursor_moved(position);
                    }
                }
                WindowEvent::CursorLeft { .. } => {
                    if let ItemSlot::Window(popup) = entry.get_mut() {
                        popup.state.handle_cursor_left();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    button: MouseButton::Left,
                    ..
                } => {
                    if let ItemSlot::Window(popup) = entry.get_mut() {
                        popup.state.handle_mouse_down();
                    }
                }
                WindowEvent::MouseInput {
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } => {
                    let click_result = match entry.get_mut() {
                        ItemSlot::Window(popup) => Some(popup.state.handle_mouse_up()),
                        ItemSlot::Pending(_) => None,
                        ItemSlot::Audio(_) => None,
                    };

                    if let Some(click_result) = click_result {
                        if click_result.content_click
                            && let ItemSlot::Window(popup) = entry.get_mut()
                        {
                            let state = &popup.state;
                            if state
                                .lua_event_tx()
                                .send(lua::Event::WindowClicked {
                                    id: state.popup_id(),
                                })
                                .is_err()
                            {
                                tracing::debug!(
                                    "Couldn't send WindowClicked event: Lua thread has shut down"
                                );
                            }
                        }

                        if click_result.should_close {
                            self.remove_item(popup_id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Custom events, sent by other threads using an event loop proxy.
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
                self.remove_item(id);
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

        for (id, item) in self.items.iter_mut() {
            let ItemSlot::Window(popup) = item else {
                continue;
            };

            if popup.is_video() {
                popup.state.request_redraw();
                poll = true;
            }

            let redraw_requested = popup.state.take_redraw_requested();

            // Handle rendering manually on Windows, rather than through RedrawRequested
            // See https://github.com/rust-windowing/winit/issues/3648
            if cfg!(target_os = "windows") && redraw_requested {
                match popup.render() {
                    Ok(outcome) if outcome.finished => finished_videos.push(*id),
                    Ok(_) => {}
                    Err(err) => tracing::error!("Error rendering window: {err}"),
                }
            }

            if popup.state.is_moving() {
                popup.state.update_position();
                poll = true;
            }

            if popup.state.is_fading() {
                popup.state.update_fade();
                poll = true;
            }
        }

        for id in finished_videos {
            self.remove_item(id);
        }

        if poll {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}
