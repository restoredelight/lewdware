use std::cell::RefCell;

use mlua::{ExternalResult, UserData, UserDataFields, UserDataMethods};

use crate::lua::{AudioHandles, Media, request::AudioRequestSender};

pub struct AudioHandle {
    id: u64,
    audio: Media,
    request_sender: AudioRequestSender,
    audio_handles: AudioHandles,
    state: RefCell<AudioState>,
}

struct AudioState {
    finish_callbacks: Vec<mlua::Function>,
}

impl AudioState {
    fn new() -> Self {
        Self {
            finish_callbacks: Vec::new(),
        }
    }
}

impl UserData for AudioHandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id));
        fields.add_field_method_get("audio", |_, this| Ok(this.audio.clone()));
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

        methods.add_async_method("pause", async |_, this, _: ()| {
            this.request_sender.pause().await.into_lua_err()?;

            Ok(())
        });

        methods.add_async_method("play", async |_, this, _: ()| {
            this.request_sender.play().await.into_lua_err()?;

            Ok(())
        });
    }
}

impl AudioHandle {
    pub fn new(
        id: u64,
        audio: Media,
        request_sender: AudioRequestSender,
        audio_handles: AudioHandles,
    ) -> Self {
        Self {
            id,
            audio,
            request_sender,
            state: RefCell::new(AudioState::new()),
            audio_handles,
        }
    }

    pub fn on_finish(&self) -> anyhow::Result<()> {
        let callbacks = {
            let state = self.state.try_borrow()?;
            state.finish_callbacks.clone()
        };

        for cb in callbacks {
            tokio::task::spawn_local(async move {
                if let Err(err) = cb.call_async::<()>(()).await {
                    tracing::error!("{err}");
                }
            });
        }

        Ok(())
    }
}

impl Drop for AudioHandle {
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
