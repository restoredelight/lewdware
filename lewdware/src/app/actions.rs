use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::atomic::Ordering;

use winit::event_loop::ActiveEventLoop;

use crate::error::{LewdwareError, Result};
use crate::lua::{AudioAction, ItemAction, ItemId, LuaRequest, WindowAction};
use crate::media::{MediaError, RequirementId, ResolvedMedia};

use super::pending::{PendingItem, PendingItemOpts};
use super::{ItemSlot, LewdwareApp};

impl LewdwareApp {
    pub(super) fn process_lua_request(
        &mut self,
        request: LuaRequest,
        event_loop: &ActiveEventLoop,
    ) -> bool {
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
            LuaRequest::SetWallpaper { file, tx } => tx.send(self.set_wallpaper(file)).is_ok(),
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
            } => self.handle_window_action(id, action),
            LuaRequest::ItemAction {
                id,
                action: ItemAction::Audio(action),
            } => self.handle_audio_action(id, action),
        } {
            tracing::error!("Couldn't send response");
        }

        false
    }

    /// Applies one window action to the item `id` names, answering whether its reply channel was
    /// still open. An id with no item left (already closed, or never spawned) is not an error --
    /// Lua holds handles that outlive their windows -- so it answers `true` and does nothing.
    pub(super) fn handle_window_action(&mut self, id: ItemId, action: WindowAction) -> bool {
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
                                    pending_volume_fade,
                                    ..
                                },
                            ..
                        }) => {
                            *pending_volume = volume;
                            *pending_volume_fade = None;
                            Ok(())
                        }
                        _ => Err(LewdwareError::Internal("Invalid window type")),
                    })
                    .is_ok()
                }
                WindowAction::FadeVideoVolume { tx, id, opts } => {
                    let opts = opts.map(|mut opts| {
                        opts.volume *= self.config.volume.video;
                        opts
                    });
                    tx.send(match entry.get_mut() {
                        ItemSlot::Window(popup) => popup
                            .start_volume_fade(id, opts)
                            .then_some(())
                            .ok_or(LewdwareError::Internal("Invalid window type")),
                        ItemSlot::Pending(PendingItem {
                            opts:
                                PendingItemOpts::Video {
                                    pending_volume_fade,
                                    ..
                                },
                            ..
                        }) => {
                            *pending_volume_fade = opts.map(|opts| (id, opts));
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
                            None => {
                                Err(LewdwareError::Internal("Text windows require non-nil text"))
                            }
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
                                if opacity != 1.0 && !pending.window.popup_opts.transparent =>
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

    /// The audio counterpart of [`Self::handle_window_action`], with the same "an id with no item
    /// is not an error" rule.
    pub(super) fn handle_audio_action(&mut self, id: ItemId, action: AudioAction) -> bool {
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
                                    pending_volume_fade,
                                    ..
                                },
                            ..
                        }) => {
                            *pending_volume = volume;
                            *pending_volume_fade = None;
                        }
                        _ => {}
                    }
                    tx.send(()).is_ok()
                }
                AudioAction::FadeVolume { tx, id, opts } => {
                    let opts = opts.map(|mut opts| {
                        opts.volume *= self.config.volume.audio;
                        opts
                    });
                    match entry.get_mut() {
                        ItemSlot::Audio(player) => player.start_volume_fade(id, opts),
                        ItemSlot::Pending(PendingItem {
                            opts:
                                PendingItemOpts::Audio {
                                    pending_volume_fade,
                                    ..
                                },
                            ..
                        }) => *pending_volume_fade = opts.map(|opts| (id, opts)),
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

    pub(super) fn process_lua_requests(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(request) = self.lua_request_rx.try_recv() {
            if self.process_lua_request(request, event_loop) {
                return;
            }
        }
    }

    pub(super) fn handle_media_resolved(
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
}
