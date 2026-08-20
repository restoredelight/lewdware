mod api;
mod audio;
mod dev_log;
mod interval;
mod media;
mod mode;
mod request;
mod storage;
mod watchdog;
mod window;

use std::{cell::RefCell, collections::HashMap, fs::File, io::Cursor, rc::Rc, sync::Arc, thread};

use anyhow::bail;
use mlua::{ExternalResult, Lua, StdLib};
use shared::{
    behaviour::{Content, Experience, effective_config, effective_options},
    encode::FileType,
    mode::{
        Metadata, OptionValue, StoredValue, VERSION_MAJOR, read_mode_metadata, resolve_options,
    },
    user_config::AppConfig,
};
use tokio::{
    sync::{mpsc::UnboundedSender, oneshot},
    task::LocalSet,
};
use uuid::Uuid;

use crate::{
    app::EventPoster,
    lua::{
        api::create_api,
        audio::AudioHandle,
        mode::{Mode, ReadSeek},
        request::RequestSender,
        window::Window,
    },
    media::MediaManager,
    monitor::Monitor,
    utils::report_fatal_startup_error,
    window::ChromeDefaults,
};

pub use api::{
    Color, Coord, DEFAULT_TEXT_POPUP_FONT_SIZE, DialogButton, DialogElement, DialogElementUpdate,
    FontSize, Notification, PopupSpawnOpts, TextAlign, TextFont, TextStyle,
};
pub use audio::VolumeFadeOpts;
pub use media::{Media, MediaData, MediaType};
pub use request::{AudioAction, ItemAction, LuaRequest, WindowAction};
pub use window::{Easing, FadeOpts, MoveOpts};

pub enum Event {
    WindowClosed {
        id: ItemId,
    },
    WindowSpawned {
        id: ItemId,
    },
    MoveFinish {
        id: ItemId,
        move_id: u64,
        x: i32,
        y: i32,
    },
    AudioFinish {
        id: ItemId,
    },
    WindowClicked {
        id: ItemId,
    },
    DialogSelect {
        id: ItemId,
        button_id: String,
        values: HashMap<String, String>,
    },
    DialogSubmit {
        id: ItemId,
        element_id: String,
        values: HashMap<String, String>,
    },
    FadeFinish {
        id: ItemId,
        fade_id: u64,
    },
    VolumeFadeFinish {
        id: ItemId,
        fade_id: u64,
    },
}

/// Identifies a Lua-created window or audio handle. Allocated by the Lua request layer before the
/// main thread receives the spawn request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(pub u64);

impl From<ItemId> for u64 {
    fn from(value: ItemId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone)]
pub struct WindowProps {
    pub window_id: ItemId,
    pub width: u32,
    pub height: u32,
    pub outer_width: u32,
    pub outer_height: u32,
    pub x: i32,
    pub y: i32,
    pub monitor: Monitor,
}

pub type Windows<T> = Rc<RefCell<HashMap<ItemId, Window<T>>>>;
pub type AudioHandles<T> = Rc<RefCell<HashMap<ItemId, Rc<AudioHandle<T>>>>>;

pub struct LuaThreadHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<thread::JoinHandle<()>>,
}

const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl LuaThreadHandle {
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.join_handle.take() {
            let (done_tx, done_rx) = std::sync::mpsc::channel();

            thread::spawn(move || {
                let _ = done_tx.send(handle.join());
            });

            match done_rx.recv_timeout(SHUTDOWN_TIMEOUT) {
                Ok(Ok(())) => {}
                Ok(Err(_)) => tracing::error!("Lua thread panicked"),
                Err(_) => tracing::warn!(
                    "Lua thread did not shut down within {SHUTDOWN_TIMEOUT:?}; \
                     some temp files may not be cleaned up"
                ),
            }
        }
    }
}

impl Drop for LuaThreadHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The currently loaded pack's identity and metadata, exposed to standalone modes as
/// `lewdware.pack` (see `start_lua_thread` for why pack-embedded modes don't get one).
pub struct PackInfo {
    pub id: Uuid,
    pub metadata: shared::pack::Metadata,
}

fn resolve_mode_config(
    metadata: &Metadata,
    mode: &shared::user_config::Mode,
    mode_options: &HashMap<shared::user_config::Mode, HashMap<String, StoredValue>>,
    experience_options: &HashMap<Uuid, HashMap<String, StoredValue>>,
    pack_id: Uuid,
    media_manager: &MediaManager,
) -> (
    HashMap<String, OptionValue>,
    serde_json::Value,
    serde_json::Value,
) {
    let stored = if matches!(mode, shared::user_config::Mode::Experience) {
        experience_options
            .get(&pack_id)
            .cloned()
            .unwrap_or_default()
    } else {
        mode_options.get(mode).cloned().unwrap_or_default()
    };

    if !matches!(
        mode,
        shared::user_config::Mode::Sandbox | shared::user_config::Mode::Experience
    ) {
        let mode_config = resolve_options(&metadata.entries, &stored);

        return (
            mode_config,
            lua_view(&Content::default(), |_| None),
            lua_view(&Experience::default(), |_| None),
        );
    }

    // Mode::Sandbox and Mode::Experience require custom options/data resolution
    let behaviour = media_manager
        .get_behaviour()
        .inspect_err(|err| tracing::warn!("Failed to read the pack's behaviour document: {err}"))
        .unwrap_or_default();

    // The behaviour document addresses media by id, so every lookup here is one query away from
    // the row itself -- used both for `pack_has_wallpaper`/`pack_has_splash` (which ask what a
    // slot actually resolves to; see `MediaLookup`) and for the file names the modes are given.
    let media_of = |id: u64| {
        media_manager
            .get_media_by_id(id)
            .inspect_err(|err| tracing::warn!("Failed to look up media #{id}: {err}"))
            .ok()
            .flatten()
    };
    let media_lookup = |id: u64| {
        media_of(id).map(|media| match media.media_data {
            MediaData::Image { .. } => FileType::Image,
            MediaData::Video { .. } => FileType::Video,
            MediaData::Audio { .. } => FileType::Audio,
        })
    };
    let name_of = |id: u64| media_of(id).map(|media| media.name);

    let schema = effective_options(metadata, &behaviour, &media_lookup);
    let resolved = effective_config(&schema, &stored);

    (
        resolved,
        lua_view(&behaviour.content, name_of),
        lua_view(&behaviour.experience.unwrap_or_default(), name_of),
    )
}

/// A behaviour section as the modes see it: every media slot resolved from the id it stores back
/// to that file's name.
///
/// Slots hold ids so that renaming a file cannot break them (`design/behaviour-storage.md`, step
/// 1), but the modes ask the media API for a *name* --
/// `lewdware.media.get_image(content().wallpaper)` in `default-modes/shared/lib/wallpaper.lua`.
/// Resolving on the way out keeps the private `__lewdware_content`/`__lewdware_experience`
/// channels, the public Lua API and every mode exactly as they were when slots held names.
///
/// A slot pointing at media the pack no longer has loses its key entirely, which reads to a mode
/// as "no wallpaper set" -- the same answer `pack_has_wallpaper` gives for it, and the same shape
/// an unfilled slot has always had.
fn lua_view<T: serde::Serialize>(
    section: &T,
    name_of: impl Fn(u64) -> Option<String>,
) -> serde_json::Value {
    let mut value = serde_json::to_value(section)
        .expect("behaviour sections always serialize: every field is a plain data type");

    // The slots, by where they live rather than by walking for a key called "wallpaper" -- a
    // content group or a caption is free to grow a field of that name without becoming a slot.
    if let Some(content) = value.as_object_mut() {
        resolve_slot(content, "wallpaper", &name_of);
        resolve_slot(content, "splash", &name_of);
    }
    if let Some(stages) = value
        .get_mut("timeline")
        .and_then(|timeline| timeline.get_mut("stages"))
        .and_then(serde_json::Value::as_array_mut)
    {
        for stage in stages {
            if let Some(content) = stage
                .get_mut("content")
                .and_then(serde_json::Value::as_object_mut)
            {
                resolve_slot(content, "wallpaper", &name_of);
                resolve_slot(content, "audio", &name_of);
            }
            if let Some(entry) = stage
                .get_mut("on_enter")
                .and_then(serde_json::Value::as_object_mut)
            {
                resolve_slot(entry, "splash", &name_of);
                resolve_slot(entry, "sound", &name_of);
            }
            if let Some(prompt) = stage
                .get_mut("prompt")
                .and_then(serde_json::Value::as_object_mut)
            {
                resolve_slot(prompt, "sound", &name_of);
            }
        }
    }
    value
}

fn resolve_slot(
    section: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    name_of: impl Fn(u64) -> Option<String>,
) {
    let Some(id) = section.get(key).and_then(serde_json::Value::as_u64) else {
        return;
    };
    match name_of(id) {
        Some(name) => {
            section.insert(key.to_string(), serde_json::Value::String(name));
        }
        None => {
            section.remove(key);
        }
    }
}

/// Starts the Lua thread using the given `media_manager` — already opened by the caller (see
/// `LewdwareApp::new` in `app.rs`), which also keeps its own clone to resolve media for popups
/// asynchronously (see `App::spawn_image`/`spawn_video`/`spawn_audio`).
pub fn start_lua_thread<T: EventPoster>(
    event_poster: T,
    config: Arc<AppConfig>,
    media_manager: MediaManager,
    pack_info: PackInfo,
    gpu_available: bool,
    dev_mode: bool,
) -> (
    UnboundedSender<Event>,
    std::sync::mpsc::Receiver<LuaRequest>,
    LuaThreadHandle,
) {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (request_tx, request_rx) = std::sync::mpsc::sync_channel(20);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let join_handle = thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        let mut file: Box<dyn ReadSeek> = match config.mode.clone() {
            shared::user_config::Mode::Sandbox => {
                let mode_data = include_bytes!("../../default-modes/sandbox/build/Sandbox.lwmode");

                Box::new(Cursor::new(mode_data))
            }
            shared::user_config::Mode::Experience => {
                let mode_data =
                    include_bytes!("../../default-modes/experience/build/Sequence.lwmode");

                Box::new(Cursor::new(mode_data))
            }
            shared::user_config::Mode::Pack { id } => {
                let mode_data = match media_manager.get_mode(id) {
                    Ok(data) => data,
                    Err(err) => {
                        report_fatal_startup_error(err);
                        return;
                    }
                };

                Box::new(Cursor::new(mode_data))
            }
            shared::user_config::Mode::File { path } => {
                let file = match File::open(path) {
                    Ok(file) => file,
                    Err(err) => {
                        report_fatal_startup_error(err);
                        return;
                    }
                };

                Box::new(file)
            }
        };

        let (header, metadata) = match read_mode_metadata(&mut file) {
            Ok(x) => x,
            Err(err) => {
                report_fatal_startup_error(err);
                return;
            }
        };
        tracing::info!("Read header and metadata");

        // VERSION_MAJOR is currently 0, so this is a no-op until the engine's major
        // version is bumped, at which point it starts warning about stale modes.
        #[allow(clippy::absurd_extreme_comparisons)]
        if header.version_major < VERSION_MAJOR {
            let warning = format!(
                "Mode was built for API v{}.x; this engine provides API v{}.x. \
                 Rebuild the mode with `lw mode build` for best compatibility.",
                header.version_major, VERSION_MAJOR
            );
            tracing::warn!("{warning}");
            crate::supervisor_link::report(shared::ipc::engine::EngineToSupervisor::Warning {
                message: warning,
            });
        }

        let entrypoint = metadata.entrypoint.clone();

        let (mode_config, content, experience) = resolve_mode_config(
            &metadata,
            &config.mode,
            &config.mode_options,
            &config.experience_options,
            pack_info.id,
            &media_manager,
        );

        let mode = Mode::new(file, metadata.files);

        // A mode embedded in a pack only ever runs against that one pack, so its storage is
        // scoped by pack UUID + mode UUID combined (so the same mode embedded in two different
        // packs gets independent storage); a standalone mode's storage is scoped by its own UUID
        // alone, shared across whichever pack it happens to run against.
        let scope_key = if matches!(config.mode, shared::user_config::Mode::Pack { .. }) {
            format!("{}-{}", pack_info.id, header.id)
        } else {
            header.id.to_string()
        };

        let storage = match storage::Storage::open(&scope_key) {
            Ok(x) => x,
            Err(err) => {
                report_fatal_startup_error(err);
                return;
            }
        };

        // A mode embedded in a pack only ever runs against that one pack, so `lewdware.pack`
        // wouldn't tell it anything it doesn't already know -- and (see above) its storage is
        // already scoped per-pack, so it doesn't need `pack.id` to compose keys either.
        let pack_info = if matches!(config.mode, shared::user_config::Mode::Pack { .. }) {
            None
        } else {
            Some(pack_info)
        };

        let mut local = LocalSet::new();

        let runtime = match LuaRuntime::new(
            mode,
            RequestSender::new(request_tx, event_poster),
            media_manager,
            storage,
            api::ApiOptions {
                pack_info,
                config: mode_config,
                content,
                experience,
                chrome: ChromeDefaults::from_config(&config.theme, &config.appearance),
                gpu_available,
                dev_mode,
            },
            dev_mode,
        ) {
            Ok(x) => Rc::new(x),
            Err(err) => {
                report_fatal_startup_error(err);
                return;
            }
        };

        // Everything needed to run the mode is in place; from here on a failure is a runtime
        // error rather than a reason the engine never started.
        crate::supervisor_link::report(shared::ipc::engine::EngineToSupervisor::Started);

        let runtime_clone = runtime.clone();

        local.spawn_local(async move {
            if let Err(err) = runtime_clone.run_entrypoint(entrypoint) {
                tracing::error!("{err}");
                crate::supervisor_link::report(shared::ipc::engine::EngineToSupervisor::RuntimeError {
                    message: err.to_string(),
                });
            }

            tracing::info!("Code finished");
        });

        // Kept alive past the two `spawn_local` calls below (which each move their own clone),
        // so storage can still be flushed synchronously once they're cancelled on shutdown.
        let runtime_for_shutdown = runtime.clone();

        local.spawn_local(async move {
            while let Some(event) = event_rx.recv().await {
                if let Err(err) = runtime.handle_event(event) {
                    tracing::error!("{err}");
                    crate::supervisor_link::report(shared::ipc::engine::EngineToSupervisor::RuntimeError {
                        message: err.to_string(),
                    });
                }
            }
        });

        rt.block_on(async {
            tokio::select! {
                _ = &mut local => {}
                _ = shutdown_rx => {
                    tracing::info!("Lua thread received shutdown signal");
                }
            }
        });

        // A debounced storage write scheduled on `local` would otherwise never run: `drop(local)`
        // below cancels every task still spawned on it, silently losing the last write of the
        // session. Flushing synchronously here, before that drop, guarantees it's persisted.
        if let Err(err) = runtime_for_shutdown.flush_storage() {
            tracing::warn!("Failed to flush lewdware.storage on shutdown: {err}");
        }

        // Cancels any tasks still spawned on `local` (mode scripts almost always have at least
        // one `every`/`after` timer running forever, so we get here via the shutdown signal
        // rather than `local` finishing on its own). This drops the last references to
        // `LuaRuntime` and this thread's `MediaManager` clone. The main thread opened the media
        // manager and holds its own clone too (see `LewdwareApp::new`), so the manager's request
        // channel — and hence its thread's shutdown/temp-file cleanup — is the main thread's
        // responsibility to wait on, not ours.
        drop(local);

        tracing::info!("Thread killed");
    });

    let handle = LuaThreadHandle {
        shutdown_tx: Some(shutdown_tx),
        join_handle: Some(join_handle),
    };

    (event_tx, request_rx, handle)
}

struct LuaRuntime<T: EventPoster> {
    mode: Rc<Mode>,
    request_sender: RequestSender<T>,
    media_manager: MediaManager,
    windows: Windows<T>,
    audio_handles: AudioHandles<T>,
    storage: storage::Storage,
    lua: Lua,
}

impl<T: EventPoster> LuaRuntime<T> {
    fn new(
        mode: Mode,
        request_tx: RequestSender<T>,
        media_manager: MediaManager,
        storage: storage::Storage,
        options: api::ApiOptions,
        dev_mode: bool,
    ) -> anyhow::Result<Self> {
        let lua = create_sandboxed_lua()?;

        if dev_mode {
            watchdog::install(&lua)?;
        }

        let mut runtime = Self {
            mode: Rc::new(mode),
            request_sender: request_tx,
            media_manager,
            windows: Rc::new(RefCell::new(HashMap::new())),
            audio_handles: Rc::new(RefCell::new(HashMap::new())),
            storage,
            lua,
        };

        runtime.create_api(options)?;

        Ok(runtime)
    }

    fn run_entrypoint(&self, entrypoint: String) -> mlua::Result<()> {
        self.mode.load(&self.lua, entrypoint).into_lua_err()?.eval()
    }

    /// See the call site in `start_lua_thread` for why this must run synchronously on shutdown.
    fn flush_storage(&self) -> anyhow::Result<()> {
        self.storage.flush_now()
    }

    /// Look up a window, dropping the borrow before returning. Callbacks dispatched to the
    /// result re-enter `windows` (a callback that spawns a popup, say), so the borrow must not
    /// still be live -- note that a `try_borrow()` guard written inline in an `if let` scrutinee
    /// lives until the end of the *body*, which is exactly the trap this exists to avoid.
    fn get_window(&self, id: ItemId) -> anyhow::Result<Option<Window<T>>> {
        let window = self.windows.try_borrow()?.get(&id).cloned();
        Ok(window)
    }

    /// `get_window`, but removing -- same borrow discipline.
    fn remove_window(&self, id: ItemId) -> anyhow::Result<Option<Window<T>>> {
        let window = self.windows.try_borrow_mut()?.remove(&id);
        Ok(window)
    }

    /// `get_window`'s counterpart for audio -- an `on_finish` callback calling
    /// `lewdware.play_audio` re-enters `audio_handles` mutably.
    fn get_audio_handle(&self, id: ItemId) -> anyhow::Result<Option<Rc<AudioHandle<T>>>> {
        let audio = self.audio_handles.try_borrow()?.get(&id).cloned();
        Ok(audio)
    }

    fn handle_event(&self, event: Event) -> anyhow::Result<()> {
        match event {
            Event::WindowClosed { id } => {
                if let Some(window) = self.remove_window(id)? {
                    window.inner_window().on_close()?;
                }
            }
            Event::WindowSpawned { id } => {
                if let Some(window) = self.get_window(id)? {
                    window.inner_window().on_spawn()?;
                }
            }
            Event::MoveFinish { id, move_id, x, y } => {
                if let Some(window) = self.get_window(id)? {
                    window.inner_window().on_move_finished(move_id, x, y)?;
                }
            }
            Event::FadeFinish { id, fade_id } => {
                if let Some(window) = self.get_window(id)? {
                    window.inner_window().on_fade_finished(fade_id)?;
                }
            }
            Event::VolumeFadeFinish { id, fade_id } => {
                if let Some(audio) = self.get_audio_handle(id)? {
                    audio.on_volume_fade_finished(fade_id)?;
                } else if let Some(window) = self.get_window(id)? {
                    window.inner_window().on_volume_fade_finished(fade_id)?;
                }
            }
            Event::AudioFinish { id } => {
                if let Some(audio) = self.get_audio_handle(id)? {
                    audio.on_finish()?;
                }
            }
            Event::WindowClicked { id } => {
                if let Some(window) = self.get_window(id)? {
                    window.inner_window().on_click()?;
                }
            }
            Event::DialogSelect {
                id,
                button_id,
                values,
            } => {
                if let Some(window) = self.get_window(id)? {
                    match window {
                        Window::Dialog(dialog) => {
                            dialog.on_select(button_id, values)?;
                        }
                        _ => bail!("Dialog select event for a non-dialog window"),
                    }
                }
            }
            Event::DialogSubmit {
                id,
                element_id,
                values,
            } => {
                if let Some(window) = self.get_window(id)? {
                    match window {
                        Window::Dialog(dialog) => {
                            dialog.on_submit(element_id, values)?;
                        }
                        _ => bail!("Dialog submit event for a non-dialog window"),
                    }
                }
            }
        }

        Ok(())
    }

    fn create_api(&mut self, options: api::ApiOptions) -> mlua::Result<()> {
        create_api(
            &self.lua,
            self.request_sender.clone(),
            self.media_manager.clone(),
            self.windows.clone(),
            self.audio_handles.clone(),
            self.storage.clone(),
            options,
        )?;

        self.lua
            .globals()
            .set("print", self.lua.create_function(print)?)?;

        let mode = self.mode.clone();
        self.lua.globals().set(
            "require",
            self.lua
                .create_function(move |lua, module| mode.require(lua, module).into_lua_err())?,
        )?;

        Ok(())
    }
}

/// A mode's `print`, routed into the log rather than to stdout.
///
/// `println!` would be the obvious implementation and is the one thing that cannot work: the
/// supervisor spawns the engine with its stdout and stderr set to `Stdio::null()`
/// (`supervisor/src/engine.rs`), so everything printed by every mode ever written has gone
/// straight into the void. Through `tracing` it reaches all three places output is actually read
/// -- `lw mode dev`'s terminal (via the dev-log stream), the JSONL log file, and the config app's
/// diagnostics -- and a mode author gets the debugging tool they reasonably assumed they had.
///
/// `info`, not `debug`: a mode printing something did so deliberately, and the file and dev-log
/// layers are filtered at `info`.
fn print(_: &Lua, args: mlua::Variadic<mlua::Value>) -> mlua::Result<()> {
    let args_str = args
        .into_iter()
        .map(|value| value.to_string())
        .collect::<mlua::Result<Vec<_>>>()?;

    tracing::info!(target: "mode", "{}", args_str.join("\t"));

    Ok(())
}

fn create_sandboxed_lua() -> mlua::Result<Lua> {
    let libs = StdLib::COROUTINE
        | StdLib::TABLE
        | StdLib::STRING
        | StdLib::UTF8
        | StdLib::MATH
        | StdLib::OS;

    let lua = Lua::new_with(libs, mlua::LuaOptions::default())?;

    lua.globals().raw_remove("load")?;
    lua.globals().raw_remove("loadstring")?;
    lua.globals().raw_remove("loadfile")?;
    lua.globals().raw_remove("dofile")?;

    lua.globals().raw_remove("collectgarbage")?;
    lua.globals().raw_remove("warn")?;
    lua.globals().raw_remove("newproxy")?;
    lua.globals().raw_remove("module")?;
    lua.globals().raw_remove("getfenv")?;
    lua.globals().raw_remove("setfenv")?;

    lua.globals()
        .get::<mlua::Table>("string")?
        .raw_remove("dump")?;

    let os = lua.globals().get::<mlua::Table>("os")?;
    let sandboxed_os = lua.create_table()?;

    for name in ["clock", "date", "difftime", "time"] {
        sandboxed_os.set(name, os.get::<mlua::Value>(name)?)?;
    }

    lua.globals().set("os", sandboxed_os)?;

    Ok(lua)
}

#[cfg(test)]
mod tests;
