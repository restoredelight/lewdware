use std::collections::HashMap;

use mlua::{IntoLua, Lua, LuaSerdeExt, serde::SerializeOptions};
use shared::mode::OptionValue;

// The Lua layer's own names for these: they describe a colour and an alignment, and a *theme*
// needs to as well, so they live with the themes and are re-exported here.
pub use shared::theme::{Color, TextAlign};

use crate::{
    app::EventPoster,
    lua::{AudioHandles, Windows, request::RequestSender},
    media::MediaManager,
    window::{AppearanceChoice, ChromeDefaults, ThemeChoice},
};

mod dialog;
mod geometry;
mod media;
mod popup;
mod system;

pub use dialog::*;
pub use geometry::*;
use media::*;
pub use popup::*;
pub use system::*;

pub struct ApiOptions {
    pub pack_info: Option<crate::lua::PackInfo>,
    pub config: HashMap<String, OptionValue>,
    /// The behaviour `content` section for `Mode::Sandbox` and `Mode::Experience`, already
    /// resolved for Lua by `lua::lua_view` -- media slots carry file names here, not the ids the
    /// document stores.
    pub content: serde_json::Value,
    /// The behaviour `experience` section for `Mode::Experience`, resolved the same way.
    pub experience: serde_json::Value,
    /// The user's own window look, applied to any window the mode does not theme itself.
    pub chrome: ChromeDefaults,
    pub gpu_available: bool,
    pub dev_mode: bool,
}

pub fn create_api<T: EventPoster>(
    lua: &Lua,
    request_sender: RequestSender<T>,
    media_manager: MediaManager,
    windows: Windows<T>,
    audio_handles: AudioHandles<T>,
    storage: crate::lua::storage::Storage,
    options: ApiOptions,
) -> mlua::Result<()> {
    let ApiOptions {
        pack_info,
        config,
        content,
        experience,
        chrome,
        gpu_available,
        dev_mode,
    } = options;

    let api_table = lua.create_table()?;

    // Nothing empty may reach Lua as mlua's `Value::NULL` sentinel (a lightuserdata, distinct from
    // Lua `nil`, for JSON-style null-vs-absent round-tripping): that sentinel is *truthy*, so the
    // idiomatic `field or default` fallback used throughout `default-modes/shared/lib/*.lua` (e.g.
    // `TextItem.timeout_seconds`) would silently never fall back. This is an internal
    // engine-to-Lua channel, not a JSON API needing that distinction, so plain `nil` is correct.
    //
    // Both options, not just `serialize_none_to_null`: these sections arrive as `serde_json::Value`
    // (already resolved for Lua by `lua::lua_view`), and a `Value::Null` serializes through
    // `serialize_unit`, not `serialize_none`.
    let for_lua = SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(false);

    // Private channel to the default-modes library code (never under the public `lewdware`
    // table, so it stays out of api.lua/the docs site): the whole behaviour.json `content`
    // section, empty for custom modes. See `ApiOptions::content`'s doc comment.
    let content_value = lua.to_value_with(&content, for_lua)?;
    lua.globals().set("__lewdware_content", content_value)?;

    // Mirrors `__lewdware_content` for the `experience` section -- only
    // `default-modes/experience/src/main.lua` reads this (empty for Sandbox/custom modes).
    let experience_value = lua.to_value_with(&experience, for_lua)?;
    lua.globals()
        .set("__lewdware_experience", experience_value)?;

    api_table.set("config", config.into_lua(lua)?)?;

    // The theme names this engine understands, so a mode can validate one that came from pack
    // data before passing it to a spawn call (where an unknown name is a hard error).
    api_table.set(
        "themes",
        ThemeChoice::ALL
            .iter()
            .map(|choice| choice.name())
            .collect::<Vec<_>>(),
    )?;
    api_table.set(
        "appearances",
        AppearanceChoice::ALL
            .iter()
            .map(|choice| choice.name())
            .collect::<Vec<_>>(),
    )?;

    // The user's own choice, which every window already uses unless the mode overrides it. Here
    // so a mode can deliberately *differ* from it for one window while leaving the rest alone —
    // as the name it was chosen by, not the look it resolves to, since that is what a mode would
    // pass back to a spawn call.
    api_table.set("user_theme", chrome.theme.name())?;
    api_table.set("user_appearance", chrome.appearance.name())?;

    let storage_table = lua.create_table()?;
    {
        let storage = storage.clone();
        storage_table.set(
            "get",
            lua.create_function(move |lua, key: String| storage.get(lua, &key))?,
        )?;
    }
    {
        let storage = storage.clone();
        storage_table.set(
            "set",
            lua.create_function(move |_, (key, value): (String, mlua::Value)| {
                storage.set(key, value)
            })?,
        )?;
    }
    {
        let storage = storage.clone();
        storage_table.set(
            "remove",
            lua.create_function(move |_, key: String| Ok(storage.remove(&key)))?,
        )?;
    }
    {
        let storage = storage.clone();
        storage_table.set(
            "clear",
            lua.create_function(move |_, ()| {
                storage.clear();
                Ok(())
            })?,
        )?;
    }
    {
        let storage = storage.clone();
        storage_table.set(
            "keys",
            lua.create_function(move |_, ()| Ok(storage.keys()))?,
        )?;
    }
    api_table.set("storage", storage_table)?;

    // `None` for a mode embedded in a pack -- see the doc comment on `PackInfo`'s only
    // constructor call site (`start_lua_thread`) for why.
    if let Some(pack_info) = pack_info {
        let pack_table = lua.create_table()?;
        pack_table.set("id", pack_info.id.to_string())?;
        pack_table.set("name", pack_info.metadata.name)?;
        pack_table.set("author", pack_info.metadata.creator)?;
        pack_table.set("version", pack_info.metadata.version)?;
        api_table.set("pack", pack_table)?;
    }

    let media_table = lua.create_table()?;

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get",
            lua.create_function(move |lua, name| get_media(lua, name, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_image",
            lua.create_function(move |lua, name| get_image(lua, name, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_video",
            lua.create_function(move |lua, name| get_video(lua, name, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "get_audio",
            lua.create_function(move |lua, name| get_audio(lua, name, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list",
            lua.create_function(move |lua, opts| list_media(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_images",
            lua.create_function(move |lua, opts| list_images(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_videos",
            lua.create_function(move |lua, opts| list_videos(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_audio",
            lua.create_function(move |lua, opts| list_audio(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random",
            lua.create_function(move |lua, opts| random_media(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_image",
            lua.create_function(move |lua, opts| random_image(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_video",
            lua.create_function(move |lua, opts| random_video(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "random_audio",
            lua.create_function(move |lua, opts| random_audio(lua, opts, media_manager.clone()))?,
        )?;
    }

    {
        let media_manager = media_manager.clone();

        media_table.set(
            "list_tags",
            lua.create_function(move |lua, ()| list_tags(lua, (), media_manager.clone()))?,
        )?;
    }

    api_table.set("media", media_table)?;

    let popup_table = lua.create_table()?;

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        popup_table.set(
            "image",
            lua.create_function(move |lua, args| {
                spawn_image_popup(
                    lua,
                    args,
                    request_sender.clone(),
                    windows.clone(),
                    chrome,
                    gpu_available,
                    dev_mode,
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        popup_table.set(
            "video",
            lua.create_function(move |lua, args| {
                spawn_video_popup(
                    lua,
                    args,
                    request_sender.clone(),
                    windows.clone(),
                    chrome,
                    gpu_available,
                    dev_mode,
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        popup_table.set(
            "text",
            lua.create_function(move |lua, args| {
                spawn_text_popup(
                    lua,
                    args,
                    request_sender.clone(),
                    windows.clone(),
                    chrome,
                    gpu_available,
                    dev_mode,
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();
        let windows = windows.clone();

        popup_table.set(
            "dialog",
            lua.create_function(move |lua, args| {
                spawn_dialog(
                    lua,
                    args,
                    request_sender.clone(),
                    windows.clone(),
                    chrome,
                    gpu_available,
                    dev_mode,
                )
            })?,
        )?;
    }

    api_table.set("popup", popup_table)?;

    let wallpaper_table = lua.create_table()?;

    {
        let media_manager = media_manager.clone();
        let request_sender = request_sender.clone();

        wallpaper_table.set(
            "set",
            lua.create_function(move |lua, args| {
                set_wallpaper(lua, args, media_manager.clone(), request_sender.clone())
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        wallpaper_table.set(
            "reset",
            lua.create_function(move |lua, args| {
                reset_wallpaper(lua, args, request_sender.clone())
            })?,
        )?;
    }

    api_table.set("wallpaper", wallpaper_table)?;

    {
        let request_sender = request_sender.clone();
        let audio_handles = audio_handles.clone();

        api_table.set(
            "play_audio",
            lua.create_function(move |lua, args| {
                play_audio(
                    lua,
                    args,
                    request_sender.clone(),
                    audio_handles.clone(),
                    dev_mode,
                )
            })?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "open_link",
            lua.create_function(move |lua, url| open_link(lua, url, request_sender.clone()))?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "show_notification",
            lua.create_function(move |lua, notification| {
                show_notification(lua, notification, request_sender.clone())
            })?,
        )?;
    }

    let monitors_table = lua.create_table()?;

    {
        let request_sender = request_sender.clone();

        monitors_table.set(
            "list",
            lua.create_function(move |lua, args| list_monitors(lua, args, request_sender.clone()))?,
        )?;
    }

    {
        let request_sender = request_sender.clone();

        monitors_table.set(
            "primary",
            lua.create_function(move |lua, args| {
                primary_monitor(lua, args, request_sender.clone())
            })?,
        )?;
    }

    api_table.set("monitors", monitors_table)?;

    {
        let request_sender = request_sender.clone();

        api_table.set(
            "exit",
            lua.create_function(move |lua, x| exit(lua, x, request_sender.clone()))?,
        )?;
    }

    api_table.set(
        "after",
        lua.create_function(move |lua, args| after(lua, args, dev_mode))?,
    )?;

    api_table.set(
        "every",
        lua.create_function(move |lua, args| every(lua, args, dev_mode))?,
    )?;

    lua.globals().set("lewdware", api_table)?;

    Ok(())
}

#[cfg(test)]
mod tests;
