use std::sync::{Arc, atomic::AtomicBool};

use winit::{
    dpi::{LogicalPosition, LogicalSize},
    event_loop::ActiveEventLoop,
};

use crate::error::{LewdwareError, MonitorError, Result};
use crate::lua::{
    self, DialogElement, FadeOpts, ItemId, MediaData, MoveOpts, PopupSpawnOpts, TextStyle,
    WindowProps,
};
use crate::media::MediaRequirement;
use crate::window::{Popup, RenderOutcome, RenderTarget, WindowOpts, WindowState};

use super::pending::{PendingItem, PendingItemOpts, PendingWindowOpts};
use super::{ItemSlot, LewdwareApp};

impl LewdwareApp {
    pub(super) fn finalize_window_opts(
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

        // The one place the user's region becomes a real screen position. Everything upstream --
        // `PopupSpawnOpts::resolve`, and the mode itself -- worked in coordinates relative to the
        // region, because `Monitors::refresh` handed them the region's size as the monitor's. The
        // origin is read from the same refresh the monitor above came from, so the two agree.
        let (region_x, region_y) = self.monitors.region_origin(popup_opts.monitor_id);

        let position = LogicalPosition::new(
            monitor_position.x + region_x + popup_opts.x,
            monitor_position.y + region_y + popup_opts.y,
        );

        Ok(WindowOpts {
            popup_opts,
            monitor,
            position,
        })
    }

    pub(super) fn create_window(
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

    pub(super) fn window_props(popup_id: ItemId, opts: &WindowOpts) -> WindowProps {
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

    pub(super) fn close_window(&mut self, popup: Popup) {
        self.window_ids.remove(&popup.state.window().id());

        let (state, target) = popup.into_parts();
        self.window_pool.release(state, target);
    }

    pub(super) fn spawn_image(
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
    pub(super) fn spawn_video(
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
                pending_volume_fade: None,
                video: requirement.id,
            },
            vec![requirement],
            event_loop,
        )?;

        Ok(props)
    }

    /// Spawns a dialog. Its image requirements are decoded concurrently; a dialog without image
    /// requirements is built immediately.
    pub(super) fn spawn_dialog(
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

    pub(super) fn spawn_text(
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

    pub(super) fn spawn_audio(
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
                pending_volume_fade: None,
            },
            vec![requirement],
            event_loop,
        )?;

        Ok(id)
    }

    /// A pending popup's media failed to decode, or its real window failed to build. Per
    /// design, this is treated exactly like the popup immediately closing: drop it and fire the
    /// `on_close` callback Lua would otherwise get from a real close.
    pub(super) fn pending_item_failed(
        &mut self,
        id: ItemId,
        context: &str,
        err: impl std::fmt::Display,
    ) {
        tracing::error!("{context}: {err}");
        self.remove_item(id);
    }

    /// Apply mutations made while a window item's media requirements were pending.
    pub(super) fn apply_pending_mutations(
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

    pub(super) fn is_resolved(&self, item_id: ItemId) -> bool {
        matches!(
            self.items.get(&item_id),
            Some(ItemSlot::Pending(item)) if item.is_resolved()
        )
    }

    pub(super) fn try_finalize_item(&mut self, event_loop: &ActiveEventLoop, item_id: ItemId) {
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

    pub(super) fn finalize_pending_item(
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
                pending_volume_fade,
            } => {
                let mut player = requirements.take_audio(audio)?;
                player.set_volume(volume);
                if let Some((id, opts)) = pending_volume_fade {
                    player.start_volume_fade(id, Some(opts));
                }
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
                pending_volume_fade,
                loop_video: _,
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
                if let Some((fade_id, opts)) = pending_volume_fade {
                    popup.start_volume_fade(fade_id, Some(opts));
                }
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
