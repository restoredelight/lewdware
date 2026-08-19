//! The rest of the API surface: wallpaper, audio, links, notifications, monitors, timers,
//! and shutting the session down.

use std::{rc::Rc, time::Duration};

use mlua::{ExternalError, ExternalResult, FromLua, Lua, LuaSerdeExt};
use serde::{Deserialize, Serialize};

// The Lua layer's own names for these: they describe a colour and an alignment, and a *theme*
// needs to as well, so they live with the themes and are re-exported here.

use super::*;
use crate::{
    app::EventPoster,
    lua::{
        AudioHandles, Media, MediaData,
        audio::AudioHandle,
        interval::{Interval, Timer},
        request::RequestSender,
    },
    media::MediaManager,
    monitor::Monitor,
};

pub(super) fn set_wallpaper<T: EventPoster>(
    _: &Lua,
    image: Media,
    media_manager: MediaManager,
    request_sender: RequestSender<T>,
) -> mlua::Result<bool> {
    if !matches!(image.media_data, MediaData::Image { .. }) {
        return Err("`image` is not an image".into_lua_err());
    }

    let file = media_manager.get_image_file(image.id).into_lua_err()?;

    request_sender.set_wallpaper(file).into_lua_err()
}

pub(super) fn reset_wallpaper<T: EventPoster>(
    _: &Lua,
    _: (),
    request_sender: RequestSender<T>,
) -> mlua::Result<()> {
    request_sender.reset_wallpaper().into_lua_err()
}

#[derive(Serialize, Deserialize)]
pub(super) struct PlayAudioOpts {
    #[serde(rename = "loop")]
    #[serde(default)]
    pub(super) loop_audio: bool,
    #[serde(default = "return_one")]
    pub(super) volume: f32,
}

impl Default for PlayAudioOpts {
    fn default() -> Self {
        Self {
            loop_audio: false,
            volume: 1.0,
        }
    }
}

impl FromLua for PlayAudioOpts {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub(super) fn play_audio<T: EventPoster>(
    _: &Lua,
    (audio, opts): (Media, Option<PlayAudioOpts>),
    request_sender: RequestSender<T>,
    audio_handles: AudioHandles<T>,
    dev_mode: bool,
) -> mlua::Result<Rc<AudioHandle<T>>> {
    let opts = opts.unwrap_or_default();

    if !matches!(audio.media_data, MediaData::Audio { .. }) {
        return Err("`audio` is not a audio".into_lua_err());
    }

    // Decode happens on the main thread now, after the ack — see `App::spawn_audio`.
    let id = request_sender.spawn_audio(audio.id, opts.loop_audio, opts.volume)?;

    let audio_handle = Rc::new(AudioHandle::new(
        id,
        audio,
        request_sender.audio_sender(id),
        dev_mode,
    ));

    audio_handles
        .try_borrow_mut()
        .into_lua_err()?
        .insert(id, audio_handle.clone());

    Ok(audio_handle)
}

pub(super) fn open_link<T: EventPoster>(
    _: &Lua,
    url: String,
    request_sender: RequestSender<T>,
) -> mlua::Result<bool> {
    request_sender.open_link(url).into_lua_err()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Notification {
    pub summary: Option<String>,
    pub body: String,
}

impl FromLua for Notification {
    fn from_lua(value: mlua::Value, lua: &Lua) -> mlua::Result<Self> {
        lua.from_value(value)
    }
}

pub(super) fn show_notification<T: EventPoster>(
    _: &Lua,
    notification: Notification,
    request_sender: RequestSender<T>,
) -> mlua::Result<bool> {
    request_sender
        .show_notification(notification)
        .into_lua_err()
}

pub(super) fn list_monitors<T: EventPoster>(
    _: &Lua,
    _: (),
    request_sender: RequestSender<T>,
) -> mlua::Result<Vec<Monitor>> {
    request_sender.list_monitors().into_lua_err()
}

pub(super) fn primary_monitor<T: EventPoster>(
    _: &Lua,
    _: (),
    request_sender: RequestSender<T>,
) -> mlua::Result<Monitor> {
    request_sender.primary_monitor().into_lua_err()
}

pub(super) fn exit<T: EventPoster>(
    _: &Lua,
    _: (),
    request_sender: RequestSender<T>,
) -> mlua::Result<()> {
    request_sender.exit().into_lua_err()
}

pub(super) fn after(
    _: &Lua,
    (ms, function): (u64, mlua::Function),
    dev_mode: bool,
) -> mlua::Result<Timer> {
    Ok(Timer::new(Duration::from_millis(ms), function, dev_mode))
}

pub(super) fn every(
    _: &Lua,
    (ms, function): (u64, mlua::Function),
    dev_mode: bool,
) -> mlua::Result<Interval> {
    Ok(Interval::new(Duration::from_millis(ms), function, dev_mode))
}
