use std::cell::RefCell;

use mlua::{ExternalResult, Lua, UserData, UserDataFields, UserDataMethods};

use crate::{
    app::EventPoster,
    lua::{AudioHandles, ItemId, Media, dev_log::log_noop, request::AudioRequestSender},
};

pub struct AudioHandle<T: EventPoster> {
    id: ItemId,
    audio: Media,
    request_sender: AudioRequestSender<T>,
    audio_handles: AudioHandles<T>,
    state: RefCell<AudioState>,
    dev_mode: bool,
}

struct AudioState {
    /// True once playback has ended -- because the (non-looping) track finished, decoding turned
    /// out to be impossible (e.g. no audio device), or `stop()` was called. Never becomes false
    /// again; every method below becomes a no-op returning `false` once this is set, rather than
    /// erroring the way most `Window` methods still do on a dead object (see `AudioRequestSender`/
    /// `WindowRequestSender` request-layer plumbing for that older convention) -- this is meant to
    /// be the model those eventually move to as well.
    finished: bool,
    finish_callbacks: Vec<mlua::Function>,
}

impl AudioState {
    fn new() -> Self {
        Self {
            finished: false,
            finish_callbacks: Vec::new(),
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
                this.request_sender.set_volume(volume).into_lua_err()
            })
        });

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
        audio_handles: AudioHandles<T>,
        dev_mode: bool,
    ) -> Self {
        Self {
            id,
            audio,
            request_sender,
            state: RefCell::new(AudioState::new()),
            audio_handles,
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
}

impl<T: EventPoster> Drop for AudioHandle<T> {
    fn drop(&mut self) {
        match self.audio_handles.try_borrow_mut() {
            Ok(mut handles) => {
                handles.remove(&self.id);
            }
            Err(err) => {
                tracing::error!("Couldn't borrow audio_handles: {err}");
            }
        }
    }
}
