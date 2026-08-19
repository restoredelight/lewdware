use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context;
use shared::user_config::AppConfig;
use winit::event::MouseButton;
use winit::event_loop::{ControlFlow, EventLoopProxy};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use crate::audio::AudioPlayer;
use crate::error::{LewdwareError, Result};
use crate::lua::{
    self, ItemId, LuaThreadHandle, PackInfo, start_lua_thread,
};
use crate::media::{
    ExtractedFile, MediaError, MediaManager, MediaRequirement, RequirementId, ResolvedMedia,
};
use crate::monitor::Monitors;
use crate::wgpu::WgpuState;
use crate::window::{Popup, WindowPool};

mod actions;
mod pending;
mod spawn;
mod wallpaper;

use pending::{PendingItem, PendingItemOpts, Requirement, RequirementState};

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
    default_wallpaper: shared::wallpaper::Snapshot,
    /// The image file the desktop wallpaper currently points at, held for exactly as long as it
    /// is the wallpaper.
    ///
    /// Media inside a `.lwpack` has to be extracted before anything outside the engine can open
    /// it, and that extraction is a `NamedTempFile` -- which deletes itself when dropped. Every
    /// wallpaper backend stores a *path*, not the pixels, and reads it back later (Plasma on the
    /// next repaint, GNOME from a settings key), so letting the temp file drop after `set` left
    /// the desktop pointing at a file that no longer existed. Nothing failed and nothing was
    /// logged; the wallpaper just never visibly changed.
    wallpaper_file: Option<ExtractedFile>,
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

/// `MediaResolved` carries a [`ResolvedMedia`], which is deliberately unboxed — see its own note.
#[allow(clippy::large_enum_variant)]
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

        // Captured up front so `lewdware.wallpaper.reset()` and the end-of-episode reset have
        // something to go back to. If this desktop can't report its wallpaper, the user's chosen
        // restore image stands in; without one the snapshot is `Unsupported` and `set_wallpaper`
        // declines rather than making a change we can't undo.
        let wallpaper = shared::wallpaper::snapshot(config.wallpaper.fallback_image());

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

        let monitors = Monitors::new(
            config.disabled_monitors.clone(),
            config.monitor_regions.clone(),
        );

        Ok(Self {
            running: false,
            config,
            wgpu_state,
            items: HashMap::new(),
            window_ids: HashMap::new(),
            requirement_owners: HashMap::new(),
            next_requirement_id: 0,
            default_wallpaper: wallpaper,
            wallpaper_file: None,
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

    pub(crate) fn new_requirement(&mut self, media: MediaRequirement) -> Requirement {
        let id = self.next_requirement_id;
        self.next_requirement_id += 1;
        Requirement {
            id: RequirementId(id),
            state: RequirementState::Pending(media),
        }
    }

    pub(crate) fn insert_pending_item(
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

    pub(crate) fn remove_pending_item(&mut self, id: ItemId) -> Option<PendingItem> {
        let ItemSlot::Pending(item) = self.items.remove(&id)? else {
            return None;
        };
        for requirement in &item.requirements {
            self.requirement_owners.remove(&requirement.id);
        }
        Some(item)
    }

    pub(crate) fn remove_item(&mut self, id: ItemId) {
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

            if event == WindowEvent::RedrawRequested {
                popup.state.take_redraw_requested();
            }

            if event == WindowEvent::RedrawRequested {
                match popup.render() {
                    Ok(outcome) => {
                        if outcome.finished {
                            self.remove_item(popup_id);
                            return;
                        }
                    }
                    Err(err) => tracing::error!("Error rendering window: {err}"),
                }
            } else {
                popup.handle_event(&event);
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
                WindowEvent::Moved(position) => {
                    if let ItemSlot::Window(popup) = entry.get_mut() {
                        popup.state.handle_window_moved(position);
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
        let mut finished_audio_fades = Vec::new();
        let mut finished_video_fades = Vec::new();

        for (id, item) in self.items.iter_mut() {
            if let ItemSlot::Audio(player) = item {
                if player.is_fading_volume() {
                    if let Some(fade_id) = player.update_volume_fade() {
                        finished_audio_fades.push((*id, fade_id));
                    }
                    poll = true;
                }
                continue;
            }
            let ItemSlot::Window(popup) = item else {
                continue;
            };

            if popup.is_video() {
                popup.state.request_redraw();
                poll = true;
                if popup.is_fading_volume()
                    && let Some(fade_id) = popup.update_volume_fade()
                {
                    finished_video_fades.push((*id, fade_id));
                }
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

        for (id, fade_id) in finished_audio_fades {
            if let Err(err) = self
                .lua_event_tx
                .send(lua::Event::VolumeFadeFinish { id, fade_id })
            {
                tracing::error!("{err}");
            }
        }
        for (id, fade_id) in finished_video_fades {
            if let Err(err) = self
                .lua_event_tx
                .send(lua::Event::VolumeFadeFinish { id, fade_id })
            {
                tracing::error!("{err}");
            }
        }

        if poll {
            event_loop.set_control_flow(ControlFlow::Poll);
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}
