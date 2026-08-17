use std::cell::RefCell;

use mlua::{ExternalResult, FromLua, Lua, LuaSerdeExt, UserData, UserDataFields, UserDataMethods};
use serde::{Deserialize, Serialize};

use crate::{
    app::EventPoster,
    lua::{ItemId, Media, dev_log::log_noop, request::AudioRequestSender, window::Easing},
};

#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy)]
pub struct VolumeFadeOpts {
    pub volume: f32,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub easing: Easing,
}

impl FromLua for VolumeFadeOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub struct AudioHandle<T: EventPoster> {
    id: ItemId,
    audio: Media,
    request_sender: AudioRequestSender<T>,
    state: RefCell<AudioState>,
    dev_mode: bool,
}

struct AudioState {
    finished: bool,
    finish_callbacks: Vec<mlua::Function>,
    current_volume_fade_id: u64,
    volume_fade_callback: Option<(u64, mlua::Function)>,
}

impl AudioState {
    fn new() -> Self {
        Self {
            finished: false,
            finish_callbacks: Vec::new(),
            current_volume_fade_id: 0,
            volume_fade_callback: None,
        }
    }
}

impl<T: EventPoster> UserData for AudioHandle<T> {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id.0));
        fields.add_field_method_get("audio", |_, this| Ok(this.audio.clone()));
        fields.add_field_method_get("finished", |_, this| {
            Ok(this.state.try_borrow().into_lua_err()?.finished)
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("on_finish", |_, this, cb: mlua::Function| {
            this.state
                .try_borrow_mut()
                .into_lua_err()?
                .finish_callbacks
                .push(cb);

            Ok(())
        });

        methods.add_method("pause", |lua, this, _: ()| {
            this.guarded(lua, "AudioHandle:pause()", || {
                this.request_sender.pause().into_lua_err()
            })
        });

        methods.add_method("play", |lua, this, _: ()| {
            this.guarded(lua, "AudioHandle:play()", || {
                this.request_sender.play().into_lua_err()
            })
        });

        methods.add_method("set_volume", |lua, this, volume: f32| {
            this.guarded(lua, "AudioHandle:set_volume()", || {
                this.state
                    .try_borrow_mut()
                    .into_lua_err()?
                    .volume_fade_callback = None;
                this.request_sender.set_volume(volume).into_lua_err()
            })
        });

        methods.add_method(
            "fade_volume",
            |lua, this, (opts, cb): (Option<VolumeFadeOpts>, Option<mlua::Function>)| {
                this.guarded(lua, "AudioHandle:fade_volume()", || {
                    let id = {
                        let mut state = this.state.try_borrow_mut().into_lua_err()?;
                        let id = state.current_volume_fade_id;
                        state.current_volume_fade_id += 1;
                        state.volume_fade_callback = opts
                            .as_ref()
                            .and_then(|_| cb.map(|callback| (id, callback)));
                        id
                    };
                    this.request_sender.fade_volume(id, opts).into_lua_err()
                })
            },
        );

        methods.add_method("stop", |lua, this, _: ()| {
            this.guarded(lua, "AudioHandle:stop()", || {
                this.request_sender.stop().into_lua_err()?;
                this.state.try_borrow_mut().into_lua_err()?.finished = true;
                Ok(())
            })
        });
    }
}

impl<T: EventPoster> AudioHandle<T> {
    pub fn new(
        id: ItemId,
        audio: Media,
        request_sender: AudioRequestSender<T>,
        dev_mode: bool,
    ) -> Self {
        Self {
            id,
            audio,
            request_sender,
            state: RefCell::new(AudioState::new()),
            dev_mode,
        }
    }

    /// Runs `f` unless already `finished`, returning whether it ran -- the shared shape behind
    /// every method above except `on_finish` (a registration, not an action) and the `finished`
    /// field itself.
    fn guarded(
        &self,
        lua: &Lua,
        what: &str,
        f: impl FnOnce() -> mlua::Result<()>,
    ) -> mlua::Result<bool> {
        if self.state.try_borrow().into_lua_err()?.finished {
            if self.dev_mode {
                log_noop(lua, what);
            }
            return Ok(false);
        }
        f()?;
        Ok(true)
    }

    /// Called when playback terminates, including a non-looping track ending, decoding turning
    /// out to be impossible, or an explicit `stop()`.
    pub fn on_finish(&self) -> anyhow::Result<()> {
        let callbacks = {
            let mut state = self.state.try_borrow_mut()?;
            state.finished = true;
            state.finish_callbacks.clone()
        };

        for cb in callbacks {
            if let Err(err) = cb.call::<()>(()) {
                tracing::error!("{err}");
            }
        }

        Ok(())
    }

    pub fn on_volume_fade_finished(&self, fade_id: u64) -> anyhow::Result<()> {
        let callback = {
            let mut state = self.state.try_borrow_mut()?;
            if state
                .volume_fade_callback
                .as_ref()
                .is_some_and(|(id, _)| *id == fade_id)
            {
                state
                    .volume_fade_callback
                    .take()
                    .map(|(_, callback)| callback)
            } else {
                None
            }
        };

        if let Some(callback) = callback
            && let Err(err) = callback.call::<()>(())
        {
            tracing::error!("{err}");
        }
        Ok(())
    }
}
