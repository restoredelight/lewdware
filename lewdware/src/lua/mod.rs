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
    mode::{VERSION_MAJOR, read_mode_metadata},
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
};

pub use api::{
    Color, Coord, DialogButton, DialogElement, DialogElementUpdate, Notification, PopupSpawnOpts,
    TextAlign, TextFont, TextStyle, WallpaperMode,
};
pub use media::{Media, MediaData, MediaType};
pub use request::{AudioAction, LuaRequest, WindowAction};
pub use window::{Easing, FadeOpts, MoveOpts};

pub enum Event {
    WindowClosed {
        id: PopupId,
    },
    WindowSpawned {
        id: PopupId,
    },
    MoveFinish {
        id: PopupId,
        move_id: u64,
        x: i32,
        y: i32,
    },
    AudioFinish {
        id: u64,
    },
    WindowClicked {
        id: PopupId,
    },
    DialogSelect {
        id: PopupId,
        button_id: String,
        values: HashMap<String, String>,
    },
    DialogSubmit {
        id: PopupId,
        element_id: String,
        values: HashMap<String, String>,
    },
    FadeFinish {
        id: PopupId,
        fade_id: u64,
    },
}

/// Identifies a popup (window) from the Lua API's perspective. Assigned by the main thread the
/// moment it acks a spawn request, which may happen before the underlying winit window (and its
/// own, unrelated `WindowId`) actually exists — see `spawn_image`/`spawn_video` in `app.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PopupId(pub u64);

impl From<PopupId> for u64 {
    fn from(value: PopupId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone)]
pub struct WindowProps {
    pub window_id: PopupId,
    pub width: u32,
    pub height: u32,
    pub outer_width: u32,
    pub outer_height: u32,
    pub x: i32,
    pub y: i32,
    pub monitor: Monitor,
}

pub type Windows = Rc<RefCell<HashMap<PopupId, Window>>>;
pub type AudioHandles = Rc<RefCell<HashMap<u64, Rc<AudioHandle>>>>;

/// Handle used to shut the Lua thread down cleanly, so its temp files get a chance to be deleted
/// via `Drop` instead of being abandoned when the process exits. See [`start_lua_thread`].
pub struct LuaThreadHandle {
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: Option<thread::JoinHandle<()>>,
}

/// How long [`LuaThreadHandle::shutdown`] waits for a clean shutdown before giving up. This
/// exists purely as a safety net (e.g. a mode script stuck in a synchronous loop that never
/// yields) so a slow shutdown can never turn into the app failing to quit.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl LuaThreadHandle {
    /// Signals the Lua thread to stop and waits (up to [`SHUTDOWN_TIMEOUT`]) for it to finish,
    /// running its `Drop` impls.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        if let Some(handle) = self.join_handle.take() {
            let (done_tx, done_rx) = std::sync::mpsc::channel();

            // `JoinHandle::join` has no timeout, so join it from a throwaway watcher thread and
            // wait on that with a timeout instead.
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

/// The currently loaded pack's identity and metadata, exposed to standalone modes as
/// `lewdware.pack` (see `start_lua_thread` for why pack-embedded modes don't get one).
pub struct PackInfo {
    pub id: Uuid,
    pub metadata: shared::read_pack::Metadata,
}

/// Starts the Lua thread using the given `media_manager` — already opened by the caller (see
/// `LewdwareApp::new` in `app.rs`), which also keeps its own clone to resolve media for popups
/// asynchronously (see `App::spawn_image`/`spawn_video`/`spawn_audio`).
pub fn start_lua_thread(
    event_poster: EventPoster,
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
            shared::user_config::Mode::Default => {
                let mode_data = include_bytes!("../../../default-modes/build/Default Modes.lwmode");

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
            if let Err(err) = shared::status::set_warning(warning) {
                tracing::warn!("Failed to write engine status: {err}");
            }
        }

        let entrypoint = metadata.entrypoint.clone();

        let mut mode_config = config
            .mode_options
            .get(&config.mode)
            .cloned()
            .unwrap_or_default();

        // Make sure the config contains all the correct options
        for (key, option) in metadata.all_options() {
            if mode_config
                .get(key)
                .is_none_or(|value| !option.matches_value(value))
            {
                mode_config.insert(key.to_string(), option.default_value());
            }
        }

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
        if let Err(err) = shared::status::set_state(shared::status::EngineState::Running) {
            tracing::warn!("Failed to write engine status: {err}");
        }

        let runtime_clone = runtime.clone();

        local.spawn_local(async move {
            if let Err(err) = runtime_clone.run_entrypoint(entrypoint) {
                tracing::error!("{err}");
                if let Err(err) = shared::status::set_last_runtime_error(err.to_string()) {
                    tracing::warn!("Failed to write engine status: {err}");
                }
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
                    if let Err(err) = shared::status::set_last_runtime_error(err.to_string()) {
                        tracing::warn!("Failed to write engine status: {err}");
                    }
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

        if let Err(err) = shared::status::set_state(shared::status::EngineState::Exited) {
            tracing::warn!("Failed to write engine status: {err}");
        }

        tracing::info!("Thread killed");
    });

    let handle = LuaThreadHandle {
        shutdown_tx: Some(shutdown_tx),
        join_handle: Some(join_handle),
    };

    (event_tx, request_rx, handle)
}

struct LuaRuntime {
    mode: Rc<Mode>,
    request_sender: RequestSender,
    media_manager: MediaManager,
    windows: Windows,
    audio_handles: AudioHandles,
    storage: storage::Storage,
    lua: Lua,
}

impl LuaRuntime {
    fn new(
        mode: Mode,
        request_tx: RequestSender,
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

    fn handle_event(&self, event: Event) -> anyhow::Result<()> {
        match event {
            Event::WindowClosed { id } => {
                if let Some(window) = self.windows.try_borrow_mut()?.remove(&id) {
                    window.inner_window().on_close()?;
                }
            }
            Event::WindowSpawned { id } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    window.inner_window().on_spawn()?;
                }
            }
            Event::MoveFinish { id, move_id, x, y } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    window.inner_window().on_move_finished(move_id, x, y)?;
                }
            }
            Event::FadeFinish { id, fade_id } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    window.inner_window().on_fade_finished(fade_id)?;
                }
            }
            Event::AudioFinish { id } => {
                if let Some(audio) = self.audio_handles.try_borrow()?.get(&id).cloned() {
                    audio.on_finish()?;
                }
            }
            Event::WindowClicked { id } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    window.inner_window().on_click()?;
                }
            }
            Event::DialogSelect {
                id,
                button_id,
                values,
            } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
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
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
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

fn print(_: &Lua, args: mlua::Variadic<mlua::Value>) -> mlua::Result<()> {
    let args_str = args
        .into_iter()
        .map(|value| value.to_string())
        .collect::<mlua::Result<Vec<_>>>()?;

    println!("{}", args_str.join("\t"));

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

/// A headless harness for running mode scripts end to end.
///
/// The Lua runtime talks to the outside world through two channels: `RequestSender` (window/
/// audio/monitor/wallpaper requests) and `MediaManager` (media metadata queries). Both are real
/// here — the same `LuaRuntime`, `Mode` and `MediaManager` types the engine actually uses — but
/// `RequestSender`'s requests are answered by a fake handler thread that replies with synthetic
/// data instead of spawning real windows, and `MediaManager` is backed by a tiny in-memory pack
/// fixture instead of a user's real one. Neither needs winit or a GPU: see `EventPoster` in
/// `app.rs`, which is what makes this possible without a real event loop.
#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        io::{Cursor, Write as _},
        sync::{Arc, Mutex},
        time::Duration,
    };

    use tempfile::NamedTempFile;
    use tokio::sync::mpsc::UnboundedSender;

    use super::*;
    use crate::{app::UserEvent, monitor::Monitor};

    /// Build a minimal but valid, media-less-by-default `.lwpack` fixture on disk, purely so
    /// `MediaManager::open` has something real to read — mirrors the on-disk layout built (and
    /// already exercised) in `media/pack.rs`'s `open_reads_deserialized_index` test.
    fn pack_fixture(with_image: bool) -> NamedTempFile {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        shared::db::migrate(&db).unwrap();

        if with_image {
            db.execute(
                "INSERT INTO media (file_name, file_type, width, height, transparent, hash) \
                 VALUES ('pic.avif', 'image', 64, 64, 0, x'00')",
                [],
            )
            .unwrap();
            db.execute("INSERT INTO tags (name) VALUES ('red'), ('blue')", [])
                .unwrap();
            db.execute(
                "INSERT INTO media_tags (media_id, tag_id) VALUES (1, 1), (1, 2)",
                [],
            )
            .unwrap();
        }

        let db_bytes = db.serialize(rusqlite::MAIN_DB).unwrap();

        let metadata = shared::read_pack::Metadata {
            name: "test-pack".to_string(),
            ..Default::default()
        };
        let metadata_bytes = metadata.to_buf().unwrap();

        let mut header = shared::read_pack::Header::new();
        header.metadata_offset = shared::read_pack::HEADER_SIZE as u64;
        header.metadata_length = metadata_bytes.len() as u64;
        header.index_offset = header.metadata_offset + header.metadata_length;
        header.index_length = db_bytes.len() as u64;

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&header.to_buf().unwrap()).unwrap();
        file.write_all(&metadata_bytes).unwrap();
        file.write_all(&db_bytes).unwrap();
        file.flush().unwrap();
        file
    }

    /// Build a `Mode` from in-memory Lua source, without needing a real `.lwmode` file.
    /// `SourceFile` offsets index into a buffer of zstd-compressed chunks — the same encoding
    /// `lw mode build` uses for each module (see `lw/src/mode/build.rs::write_files`), since
    /// `Mode::require`/`load` decompress with `zstd::decode_all`.
    fn make_mode(sources: &[(&str, &str)]) -> Mode {
        let mut buf = Vec::new();
        let mut files = HashMap::new();

        for (path, source) in sources {
            let offset = buf.len() as u64;
            zstd::stream::copy_encode(source.as_bytes(), &mut buf, 0).unwrap();
            files.insert(
                (*path).to_string(),
                shared::mode::SourceFile {
                    offset,
                    length: buf.len() as u64 - offset,
                },
            );
        }

        Mode::new(Box::new(Cursor::new(buf)), files)
    }

    /// A simplified, assertable summary of a `LuaRequest` seen by the fake handler.
    #[derive(Debug, Clone, PartialEq)]
    enum Recorded {
        SpawnImage { media_id: u64 },
        SpawnVideo { media_id: u64 },
        SpawnDialog,
        SpawnText,
        CloseWindow { id: PopupId },
        OpenLink { url: String },
        Exit,
    }

    fn fake_monitor() -> Monitor {
        Monitor {
            id: 0,
            primary: true,
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
        }
    }

    fn fake_window_props(id: PopupId) -> WindowProps {
        WindowProps {
            window_id: id,
            width: 100,
            height: 100,
            outer_width: 100,
            outer_height: 100,
            x: 0,
            y: 0,
            monitor: fake_monitor(),
        }
    }

    /// Stands in for `LewdwareApp::process_lua_request` + `about_to_wait`/`user_event`: answers
    /// every `LuaRequest` with synthetic data instead of touching winit/wgpu.
    ///
    /// Reproduces one real subtlety exactly: like `LewdwareApp`, a `WindowAction` for an id that's
    /// already closed is dropped entirely (its `tx` included) rather than replied to — the
    /// requester then sees a disconnected channel, which `WindowRequestSender::send` turns into
    /// `LewdwareError::WindowNotFound`. `Window::close()` treats that as a no-op; other methods
    /// (e.g. `set_opacity`) surface it as a Lua error — see the `dead_window_semantics` test.
    fn run_fake_handler(
        request_rx: std::sync::mpsc::Receiver<LuaRequest>,
        recorded: Arc<Mutex<Vec<Recorded>>>,
        event_tx: UnboundedSender<Event>,
    ) {
        let mut next_id = 0u64;
        let mut closed_windows = HashSet::new();
        // Stands in for the real `DialogWindow`'s input-element state (see `window_type.rs`),
        // just enough for `values`/`value`/`update` to round-trip in tests.
        let mut dialog_values: HashMap<PopupId, HashMap<String, String>> = HashMap::new();

        while let Ok(request) = request_rx.recv() {
            match request {
                LuaRequest::SpawnImage { media_id, tx, .. } => {
                    let id = PopupId(next_id);
                    next_id += 1;
                    recorded
                        .lock()
                        .unwrap()
                        .push(Recorded::SpawnImage { media_id });
                    let _ = tx.send(Ok(fake_window_props(id)));
                }
                LuaRequest::SpawnVideo { media_id, tx, .. } => {
                    let id = PopupId(next_id);
                    next_id += 1;
                    recorded
                        .lock()
                        .unwrap()
                        .push(Recorded::SpawnVideo { media_id });
                    let _ = tx.send(Ok(fake_window_props(id)));
                }
                LuaRequest::SpawnDialog { elements, tx, .. } => {
                    let id = PopupId(next_id);
                    next_id += 1;
                    recorded.lock().unwrap().push(Recorded::SpawnDialog);
                    let values = elements
                        .into_iter()
                        .filter_map(|element| match element {
                            DialogElement::Input {
                                id,
                                initial_value: value,
                                ..
                            } => Some((id, value.unwrap_or_default())),
                            _ => None,
                        })
                        .collect();
                    dialog_values.insert(id, values);
                    let _ = tx.send(Ok(fake_window_props(id)));
                }
                LuaRequest::SpawnText { tx, .. } => {
                    let id = PopupId(next_id);
                    next_id += 1;
                    recorded.lock().unwrap().push(Recorded::SpawnText);
                    let _ = tx.send(Ok(fake_window_props(id)));
                }
                LuaRequest::SpawnAudio { tx, .. } => {
                    let _ = tx.send(0);
                }
                LuaRequest::SetWallpaper { tx, .. } => {
                    let _ = tx.send(Ok(()));
                }
                LuaRequest::ResetWallpaper { tx } => {
                    let _ = tx.send(());
                }
                LuaRequest::OpenLink { url, tx } => {
                    recorded.lock().unwrap().push(Recorded::OpenLink { url });
                    let _ = tx.send(Ok(()));
                }
                LuaRequest::ShowNotification { tx, .. } => {
                    let _ = tx.send(Ok(()));
                }
                LuaRequest::ListMonitors { tx } => {
                    // Non-empty: popup spawns that don't specify a monitor now pick a random one
                    // from this list on the Lua thread (see `resolve_monitor` in `lua/api.rs`).
                    let _ = tx.send(vec![fake_monitor()]);
                }
                LuaRequest::PrimaryMonitor { tx } => {
                    let _ = tx.send(Ok(fake_monitor()));
                }
                LuaRequest::Exit { tx } => {
                    recorded.lock().unwrap().push(Recorded::Exit);
                    let _ = tx.send(());
                    break;
                }
                LuaRequest::WindowAction { id, action } => {
                    if closed_windows.contains(&id) {
                        // Mirrors `LewdwareApp::process_lua_request`'s `else { true }` branch for
                        // a non-occupied entry: drop the whole action (and its `tx`) without
                        // replying.
                        continue;
                    }

                    match action {
                        WindowAction::CloseWindow { tx } => {
                            closed_windows.insert(id);
                            recorded.lock().unwrap().push(Recorded::CloseWindow { id });
                            let _ = event_tx.send(Event::WindowClosed { id });
                            let _ = tx.send(());
                        }
                        WindowAction::Move { tx, .. } => {
                            let _ = tx.send(Ok(()));
                        }
                        WindowAction::Fade { tx, .. } => {
                            let _ = tx.send(Ok(()));
                        }
                        WindowAction::PauseVideo { tx } => {
                            let _ = tx.send(Ok(()));
                        }
                        WindowAction::PlayVideo { tx } => {
                            let _ = tx.send(Ok(()));
                        }
                        WindowAction::SetVideoVolume { tx, .. } => {
                            let _ = tx.send(Ok(()));
                        }
                        WindowAction::SetVideoLoop { tx, .. } => {
                            let _ = tx.send(Ok(()));
                        }
                        WindowAction::SetText { tx, .. } => {
                            let _ = tx.send(Ok(()));
                        }
                        WindowAction::UpdateDialogElement {
                            tx,
                            id: element_id,
                            props,
                        } => {
                            let updated = dialog_values
                                .get_mut(&id)
                                .and_then(|values| values.get_mut(&element_id))
                                .map(|value| {
                                    if let Some(new_value) = props.value {
                                        *value = new_value;
                                    }
                                })
                                .is_some();
                            let _ = tx.send(updated);
                        }
                        WindowAction::GetDialogValues { tx } => {
                            let values = dialog_values.get(&id).cloned().unwrap_or_default();
                            let _ = tx.send(values);
                        }
                        WindowAction::GetDialogValue {
                            id: element_id,
                            tx,
                        } => {
                            let value = dialog_values
                                .get(&id)
                                .and_then(|values| values.get(&element_id))
                                .cloned();
                            let _ = tx.send(value);
                        }
                        WindowAction::SetTitle { tx, .. } => {
                            let _ = tx.send(());
                        }
                        WindowAction::SetOpacity { tx, .. } => {
                            let _ = tx.send(Ok(()));
                        }
                    }
                }
                LuaRequest::AudioAction { action, .. } => match action {
                    AudioAction::Pause { tx } => {
                        let _ = tx.send(());
                    }
                    AudioAction::Play { tx } => {
                        let _ = tx.send(());
                    }
                    AudioAction::SetVolume { tx, .. } => {
                        let _ = tx.send(());
                    }
                    AudioAction::Stop { tx } => {
                        let _ = tx.send(());
                    }
                },
            }
        }
    }

    struct Harness {
        runtime: Rc<LuaRuntime>,
        event_rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
        /// Lets a test simulate an `Event` the real main thread would send but that the fake
        /// handler doesn't model — e.g. `WindowSpawned`, which (unlike `WindowClosed`) is never
        /// the reply to a `LuaRequest`: in the real app it fires from `InnerWindow::show()` once
        /// a popup's media finishes decoding, which this harness has no equivalent of.
        event_tx: UnboundedSender<Event>,
        recorded: Arc<Mutex<Vec<Recorded>>>,
        _pack_file: NamedTempFile,
        _storage_dir: tempfile::TempDir,
    }

    impl Harness {
        fn new(sources: &[(&str, &str)], with_image: bool) -> Self {
            let pack_file = pack_fixture(with_image);
            let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);

            let (media_manager, _metadata, _pack_id, _media_manager_handle) =
                MediaManager::open(pack_file.path(), event_poster.clone(), None).unwrap();

            let (request_tx, request_rx) = std::sync::mpsc::sync_channel(20);
            let request_sender = RequestSender::new(request_tx, event_poster);

            let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
            let event_tx_clone = event_tx.clone();
            let recorded = Arc::new(Mutex::new(Vec::new()));
            {
                let recorded = recorded.clone();
                thread::spawn(move || run_fake_handler(request_rx, recorded, event_tx));
            }

            let storage_dir = tempfile::tempdir().unwrap();
            let storage = storage::Storage::open_at(storage_dir.path().join("test.db")).unwrap();

            let mode = make_mode(sources);
            let runtime = Rc::new(
                LuaRuntime::new(
                    mode,
                    request_sender,
                    media_manager,
                    storage,
                    api::ApiOptions {
                        pack_info: None,
                        config: HashMap::new(),
                        gpu_available: false,
                        dev_mode: false,
                    },
                    false,
                )
                .unwrap(),
            );

            Self {
                runtime,
                event_rx,
                event_tx: event_tx_clone,
                recorded,
                _pack_file: pack_file,
                _storage_dir: storage_dir,
            }
        }

        /// Simulate an `Event` the fake handler doesn't produce itself — see the field doc on
        /// [`Harness::event_tx`].
        fn send_event(&self, event: Event) {
            let _ = self.event_tx.send(event);
        }

        fn run_entrypoint(&mut self, entrypoint: &str) -> mlua::Result<()> {
            let result = self.runtime.run_entrypoint(entrypoint.to_string());
            self.pump_events();
            result
        }

        /// Delivers any `Event`s the fake handler has produced so far (e.g. `WindowClosed` from a
        /// `close()` request) — the real counterpart of the main thread's `about_to_wait`/
        /// `user_event` calling `LuaRuntime::handle_event`.
        fn pump_events(&mut self) {
            while let Ok(event) = self.event_rx.try_recv() {
                self.runtime.handle_event(event).unwrap();
            }
        }

        /// Advances paused virtual time so `after()`/`every()` timers due within `duration` fire,
        /// then delivers any resulting events.
        async fn advance(&mut self, duration: Duration) {
            // Advancing in one large jump can skip over intermediate timers that only get
            // (re-)armed as a side effect of an earlier one firing (e.g. `Interval`'s next
            // `tick()` call, made fresh each loop iteration in `interval.rs`) -- stepping in small
            // increments, yielding between each, gives every one of them a chance to register and
            // fire in order.
            let step = Duration::from_millis(10);
            let mut remaining = duration;
            while remaining > Duration::ZERO {
                let this_step = step.min(remaining);
                tokio::time::advance(this_step).await;
                tokio::task::yield_now().await;
                remaining -= this_step;
            }
            self.pump_events();
        }

        fn recorded(&self) -> Vec<Recorded> {
            self.recorded.lock().unwrap().clone()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn spawn_image_popup_request_stream() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local image = lewdware.media.get_image("pic.avif")
                            assert(image ~= nil, "expected fixture image")
                            local window = lewdware.popup.image(image)
                            window:close()
                        "#,
                    )],
                    true,
                );

                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(
                    harness.recorded(),
                    vec![
                        Recorded::SpawnImage { media_id: 1 },
                        Recorded::CloseWindow { id: PopupId(0) },
                    ]
                );
            })
            .await;
    }

    /// The fixture image carries the tags `red` and `blue`. Exercises the Lua-facing `tags`
    /// argument end-to-end: the plain-list shorthand (= `any`), the `{ any, all, none }` table
    /// form, and the unknown-tag semantics documented in `QueryMediaOpts`.
    #[tokio::test(start_paused = true)]
    async fn tag_filters_accept_shorthand_and_table_forms() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local function count(filter)
                                return #lewdware.media.list_images({ tags = filter })
                            end

                            assert(count({ "red" }) == 1, "shorthand list = any")
                            assert(count({ "unknown" }) == 0, "unknown tag matches nothing")
                            assert(count({ any = { "red", "unknown" } }) == 1, "any table form")
                            assert(count({ all = { "red", "blue" } }) == 1, "all satisfied")
                            assert(count({ all = { "red", "unknown" } }) == 0, "unknown all tag matches nothing")
                            assert(count({ none = { "blue" } }) == 0, "none excludes")
                            assert(count({ none = { "unknown" } }) == 1, "unknown none tag excludes nothing")
                            assert(count({ any = { "red" }, none = { "blue" } }) == 0, "fields combine")
                        "#,
                    )],
                    true,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn media_tags_and_list_tags() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local image = lewdware.media.get_image("pic.avif")
                            table.sort(image.tags)
                            assert(#image.tags == 2, "expected 2 tags, got " .. #image.tags)
                            assert(image.tags[1] == "blue")
                            assert(image.tags[2] == "red")

                            local tags = lewdware.media.list_tags()
                            table.sort(tags)
                            assert(#tags == 2, "expected the pack's full vocabulary")
                            assert(tags[1] == "blue")
                            assert(tags[2] == "red")
                        "#,
                    )],
                    true,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn timer_stop_prevents_a_pending_firing() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local timer = lewdware.after(1000, function()
                                lewdware.open_link("https://should-not-fire.example")
                            end)
                            timer:stop()
                        "#,
                    )],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(2000)).await;

                assert_eq!(harness.recorded(), vec![]);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn interval_keeps_firing_after_a_callback_errors() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            COUNT = 0
                            lewdware.every(1000, function()
                                COUNT = COUNT + 1
                                if COUNT == 1 then
                                    error("boom")
                                end
                                lewdware.open_link("tick " .. COUNT)
                            end)
                        "#,
                    )],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();

                // First tick errors inside the callback -- caught internally by `Interval`'s own
                // task (see `interval.rs`), so it must not kill the interval or the mode. Advanced
                // in multiple separate steps and past the nominal 2000ms for two ticks: a blocking
                // round trip through the fake handler's real OS thread (`open_link`'s request/
                // reply) eats into the paused clock's slack each time, on top of
                // `MissedTickBehavior::Delay` scheduling the next tick from when the previous one
                // actually completed rather than off a fixed schedule.
                harness.advance(Duration::from_millis(1000)).await;
                harness.advance(Duration::from_millis(1000)).await;
                harness.advance(Duration::from_millis(1000)).await;

                // Exactly one recorded request -- tick 1's `open_link` never ran (it errored
                // first), tick 2's did, proving the interval survived the error and kept firing.
                assert_eq!(
                    harness.recorded(),
                    vec![Recorded::OpenLink {
                        url: "tick 2".to_string()
                    }]
                );
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn require_caches_modules_and_runs_them_once() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[
                        (
                            "main.lua",
                            r#"
                                local a1 = require("a")
                                local a2 = require("a")
                                assert(a1.count == 1, "module body should only run once")
                                assert(a1 == a2, "require should return the cached value")
                            "#,
                        ),
                        (
                            "a.lua",
                            r#"
                                COUNT = (COUNT or 0) + 1
                                return { count = COUNT }
                            "#,
                        ),
                    ],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn require_detects_circular_dependencies() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[
                        ("main.lua", r#"require("a")"#),
                        ("a.lua", r#"require("b")"#),
                        ("b.lua", r#"require("a")"#),
                    ],
                    false,
                );

                let err = harness.run_entrypoint("main.lua").unwrap_err();
                assert!(
                    err.to_string().contains("circular"),
                    "expected a circular-require error, got: {err}"
                );
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn lewdware_pack_absent_for_pack_embedded_modes() {
        LocalSet::new()
            .run_until(async {
                // `Harness::new` always builds its `LuaRuntime` with `pack_info: None` -- the
                // same as what `start_lua_thread` passes for `Mode::Pack`.
                let mut harness =
                    Harness::new(&[("main.lua", r#"assert(lewdware.pack == nil)"#)], false);

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn lewdware_pack_reflects_pack_metadata_for_standalone_modes() {
        LocalSet::new()
            .run_until(async {
                let pack_file = pack_fixture(false);
                let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);

                let (media_manager, _metadata, pack_id, _handle) =
                    MediaManager::open(pack_file.path(), event_poster.clone(), None).unwrap();

                let (request_tx, _request_rx) = std::sync::mpsc::sync_channel(20);
                let request_sender = RequestSender::new(request_tx, event_poster);

                let pack_info = PackInfo {
                    id: pack_id,
                    metadata: shared::read_pack::Metadata {
                        name: "Test Pack".to_string(),
                        creator: Some("tester".to_string()),
                        description: None,
                        version: Some("2.0.0".to_string()),
                    },
                };

                let mode = make_mode(&[(
                    "main.lua",
                    r#"
                        assert(lewdware.pack.name == "Test Pack", "wrong name")
                        assert(lewdware.pack.author == "tester", "wrong author")
                        assert(lewdware.pack.version == "2.0.0", "wrong version")
                        assert(type(lewdware.pack.id) == "string", "id should be a string")
                    "#,
                )]);

                let storage_dir = tempfile::tempdir().unwrap();
                let storage =
                    storage::Storage::open_at(storage_dir.path().join("test.db")).unwrap();

                let runtime = LuaRuntime::new(
                    mode,
                    request_sender,
                    media_manager,
                    storage,
                    api::ApiOptions {
                        pack_info: Some(pack_info),
                        config: HashMap::new(),
                        gpu_available: false,
                        dev_mode: false,
                    },
                    false,
                )
                .unwrap();

                runtime.run_entrypoint("main.lua".to_string()).unwrap();

                assert_eq!(
                    runtime
                        .lua
                        .globals()
                        .get::<mlua::Table>("lewdware")
                        .unwrap()
                        .get::<mlua::Table>("pack")
                        .unwrap()
                        .get::<String>("id")
                        .unwrap(),
                    pack_id.to_string()
                );
            })
            .await;
    }

    /// Exercises the dev-mode no-op logging path (execution model rule 3) for real, inside an
    /// actual `LuaRuntime` -- not just `watchdog.rs`'s isolated `Lua::new()` tests. The main risk
    /// here isn't the logging itself (fire-and-forget, can't be observed without a tracing
    /// subscriber -- which is racy across parallel test threads, see `watchdog.rs`'s test module
    /// doc comment) but whether `dev_log::log_noop`'s `Lua::inspect_stack` call could itself
    /// error or panic inside a real callback and turn a no-op into a hard failure.
    #[tokio::test(start_paused = true)]
    async fn dev_mode_no_op_logging_does_not_break_the_call() {
        LocalSet::new()
            .run_until(async {
                let pack_file = pack_fixture(false);
                let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);

                let (media_manager, _metadata, _pack_id, _handle) =
                    MediaManager::open(pack_file.path(), event_poster.clone(), None).unwrap();

                // Timer:stop() never touches the request sender, so this can go nowhere -- no
                // fake handler thread needed for this test.
                let (request_tx, _request_rx) = std::sync::mpsc::sync_channel(20);
                let request_sender = RequestSender::new(request_tx, event_poster);

                let mode = make_mode(&[(
                    "main.lua",
                    r#"
                        local timer = lewdware.after(1000, function() end)
                        assert(timer:stop() == true, "first stop should succeed")
                        assert(timer:stop() == false, "second stop is a no-op, logged in dev mode")
                    "#,
                )]);

                let storage_dir = tempfile::tempdir().unwrap();
                let storage =
                    storage::Storage::open_at(storage_dir.path().join("test.db")).unwrap();

                let runtime = LuaRuntime::new(
                    mode,
                    request_sender,
                    media_manager,
                    storage,
                    api::ApiOptions {
                        pack_info: None,
                        config: HashMap::new(),
                        gpu_available: false,
                        dev_mode: true,
                    },
                    true,
                )
                .unwrap();

                runtime.run_entrypoint("main.lua".to_string()).unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn lewdware_storage_get_set_remove_clear_keys_roundtrip() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            assert(lewdware.storage.get("count") == nil, "should start empty")
                            assert(#lewdware.storage.keys() == 0)

                            lewdware.storage.set("count", 1)
                            lewdware.storage.set("name", "test")
                            lewdware.storage.set("nested", { a = 1, b = { 2, 3 } })

                            assert(lewdware.storage.get("count") == 1)
                            assert(lewdware.storage.get("name") == "test")
                            assert(lewdware.storage.get("nested").a == 1)
                            assert(lewdware.storage.get("nested").b[2] == 3)

                            local keys = lewdware.storage.keys()
                            table.sort(keys)
                            assert(keys[1] == "count", "unexpected keys: " .. table.concat(keys, ","))
                            assert(keys[2] == "name")
                            assert(keys[3] == "nested")

                            assert(lewdware.storage.remove("count") == true)
                            assert(lewdware.storage.remove("count") == false)
                            assert(lewdware.storage.get("count") == nil)

                            lewdware.storage.clear()
                            assert(#lewdware.storage.keys() == 0)
                        "#,
                    )],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn lewdware_storage_rejects_unstorable_values_with_a_lua_error() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local ok, err = pcall(function()
                                lewdware.storage.set("bad", print)
                            end)
                            assert(not ok, "storing a function should error")
                            assert(
                                tostring(err):find("booleans, numbers, strings"),
                                "unexpected error: " .. tostring(err)
                            )
                        "#,
                    )],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn audio_handle_finished_stops_callbacks_from_firing_again() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[
                        (
                            "main.lua",
                            r#"
                                local audio = lewdware.play_audio(
                                    { id = 0, name = "test", type = "audio", duration = 1.0 }
                                )

                                assert(audio.finished == false, "should not start finished")
                                assert(audio:pause() == true, "pause should run while not finished")
                                assert(audio:play() == true, "play should run while not finished")
                                assert(
                                    audio:set_volume(0.5) == true,
                                    "set_volume should run while not finished"
                                )

                                FINISH_COUNT = 0
                                audio:on_finish(function() FINISH_COUNT = FINISH_COUNT + 1 end)

                                AUDIO = audio
                            "#,
                        ),
                        (
                            "after_finish.lua",
                            r#"
                                assert(AUDIO.finished == true, "finished after AudioFinish event")
                                assert(FINISH_COUNT == 1, "on_finish should fire exactly once")

                                assert(AUDIO:pause() == false, "no-op once finished")
                                assert(AUDIO:play() == false, "no-op once finished")
                                assert(AUDIO:set_volume(0.9) == false, "no-op once finished")
                                assert(AUDIO:stop() == false, "no-op once finished")
                            "#,
                        ),
                    ],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();

                // Simulates a natural finish (or, equally, a decode failure -- both go through
                // this same event) -- see `AudioHandle::on_finish`'s doc comment.
                harness.send_event(Event::AudioFinish { id: 0 });
                harness.pump_events();

                harness.run_entrypoint("after_finish.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn audio_handle_stop_is_immediate_and_suppresses_on_finish() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local audio = lewdware.play_audio(
                                { id = 0, name = "test", type = "audio", duration = 1.0 }
                            )

                            FINISH_COUNT = 0
                            audio:on_finish(function() FINISH_COUNT = FINISH_COUNT + 1 end)

                            assert(audio.finished == false)
                            assert(audio:stop() == true, "stop should run while not finished")
                            assert(audio.finished == true, "finished immediately after stop()")
                            assert(FINISH_COUNT == 0, "on_finish must not fire for an explicit stop")

                            assert(audio:stop() == false, "a second stop is a no-op")
                            assert(audio:pause() == false, "no-op after stop")
                        "#,
                    )],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn video_window_set_loop_succeeds() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local video = {
                                id = 0,
                                name = "test",
                                type = "video",
                                width = 64,
                                height = 64,
                                duration = 1.0,
                                transparent = false,
                            }
                            local window = lewdware.popup.video(video)
                            assert(window:set_loop(false) == true)
                            assert(window:close() == true)
                            assert(window:set_loop(true) == false, "set_loop on a closed window is a no-op")
                        "#,
                    )],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn video_window_set_volume_succeeds() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local video = {
                                id = 0,
                                name = "test",
                                type = "video",
                                width = 64,
                                height = 64,
                                duration = 1.0,
                                transparent = false,
                            }
                            local window = lewdware.popup.video(video)
                            window:set_volume(0.5)
                        "#,
                    )],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn dead_window_semantics() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local image = lewdware.media.get_image("pic.avif")
                            local window = lewdware.popup.image(image)

                            -- Registering on a still-open window returns true (contrast with the
                            -- closed-window case below).
                            assert(window:on_close(function() end) == true, "on_close on open window")
                            assert(window:on_spawn(function() end) == true, "on_spawn on open window")

                            assert(window:close() == true, "closing an open window returns true")

                            -- `.closed` and on_close()/on_spawn()'s no-op behavior take effect
                            -- immediately, synchronously with close() -- not only once the
                            -- WindowClosed event this also triggers is later processed.
                            assert(window.closed == true, "closed flips synchronously with close()")

                            assert(
                                window:close() == false,
                                "closing an already-closed window is a no-op returning false"
                            )

                            -- Acting on a closed window is a no-op that returns false, not an
                            -- error -- matching AudioHandle's finished convention.
                            assert(window:set_opacity(0.5) == false)
                            assert(window:set_title("new title") == false)
                            assert(window:move({ x = 10 }) == false)
                            assert(window:fade({ opacity = 0.5 }) == false)

                            -- Registering a callback on an already-closed window is also a no-op:
                            -- it returns false and never runs cb.
                            local ran = false
                            assert(window:on_close(function() ran = true end) == false)
                            assert(window:on_spawn(function() ran = true end) == false)
                            assert(not ran)
                        "#,
                    )],
                    true,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    /// Exercises the Lua-facing side of `Window:on_click()`: registration, fan-out to multiple
    /// callbacks (fires every time, unlike `on_spawn`), and the no-op-on-closed convention.
    /// `WindowClicked` events are injected manually here -- the actual physical hit-testing (press
    /// + release inside the content area, decorations excluded) lives in the winit/egui rendering
    /// layer (`window/inner_window.rs`, `window/window_type.rs`), which this Lua-only harness
    /// doesn't exercise.
    #[tokio::test(start_paused = true)]
    async fn window_on_click_fires_and_is_a_no_op_when_closed() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[
                        (
                            "main.lua",
                            r#"
                                local image = lewdware.media.get_image("pic.avif")
                                WINDOW = lewdware.popup.image(image)
                                CLICKS = 0

                                assert(WINDOW:on_click(function() CLICKS = CLICKS + 1 end) == true)
                                assert(WINDOW:on_click(function() CLICKS = CLICKS + 1 end) == true)
                            "#,
                        ),
                        (
                            "after_clicks.lua",
                            r#"
                                assert(CLICKS == 4, "both callbacks should fire on every click")

                                assert(WINDOW:close() == true)
                                assert(WINDOW:on_click(function() CLICKS = CLICKS + 1 end) == false)
                            "#,
                        ),
                    ],
                    true,
                );

                harness.run_entrypoint("main.lua").unwrap();

                harness.send_event(Event::WindowClicked { id: PopupId(0) });
                harness.send_event(Event::WindowClicked { id: PopupId(0) });
                harness.pump_events();

                harness.run_entrypoint("after_clicks.lua").unwrap();
            })
            .await;
    }

    /// Exercises `Window.spawned`/`Window:on_spawn()` end to end: the field flips only once the
    /// (here, manually injected) `WindowSpawned` event is delivered, callbacks registered before
    /// that fire in registration order, and a callback registered *after* the window has already
    /// spawned still fires -- but queued via `tokio::task::spawn_local`, not inline (execution
    /// model rule 1: no Lewdware function may call back into Lua synchronously).
    #[tokio::test(start_paused = true)]
    async fn window_spawn_fires_callbacks_and_queues_late_registration() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[
                        (
                            "main.lua",
                            r#"
                                local image = lewdware.media.get_image("pic.avif")
                                WINDOW = lewdware.popup.image(image)
                                ORDER = {}

                                assert(WINDOW.spawned == false, "not spawned before the event arrives")

                                WINDOW:on_spawn(function() table.insert(ORDER, "first") end)
                                WINDOW:on_spawn(function() table.insert(ORDER, "second") end)
                            "#,
                        ),
                        (
                            "after_spawn.lua",
                            r#"
                                assert(WINDOW.spawned == true, "spawned once the event is delivered")
                                assert(#ORDER == 2, "both callbacks should have fired")
                                assert(
                                    ORDER[1] == "first" and ORDER[2] == "second",
                                    "callbacks fire in registration order"
                                )

                                WINDOW:on_spawn(function() table.insert(ORDER, "late") end)
                                assert(#ORDER == 2, "a late on_spawn callback must not fire inline")
                            "#,
                        ),
                        (
                            "after_queue_runs.lua",
                            r#"
                                assert(#ORDER == 3, "the late callback fires once queued")
                                assert(ORDER[3] == "late")
                            "#,
                        ),
                    ],
                    true,
                );

                harness.run_entrypoint("main.lua").unwrap();

                harness.send_event(Event::WindowSpawned { id: PopupId(0) });
                harness.pump_events();

                harness.run_entrypoint("after_spawn.lua").unwrap();

                // Let the `spawn_local`'d late callback actually run.
                harness.advance(Duration::from_millis(10)).await;

                harness.run_entrypoint("after_queue_runs.lua").unwrap();
            })
            .await;
    }

    /// Exercises the `lewdware.popup.dialog()` binding end to end: `values()`/`value()` reflect
    /// `initial_value`, `update()` changes a live value (and reports whether the target id
    /// existed), and `on_select`/`on_submit` (fired here via manually injected events, since the
    /// fake handler doesn't model real button clicks/Enter presses) receive the button/element id
    /// alongside a snapshot of every input's current value.
    #[tokio::test(start_paused = true)]
    async fn dialog_values_update_and_callbacks() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[
                        (
                            "main.lua",
                            r#"
                                DIALOG = lewdware.popup.dialog({
                                    elements = {
                                        { type = "text", text = "Confirm?" },
                                        { type = "input", id = "name", initial_value = "Bob" },
                                        { type = "buttons", options = {
                                            { id = "yes", label = "Yes", default = true },
                                            { id = "no", label = "No" },
                                        }},
                                    },
                                })

                                CLICKS = {}
                                DIALOG:on_select(function(button_id, values)
                                    table.insert(CLICKS, { button_id, values.name })
                                end)

                                SUBMITS = {}
                                DIALOG:on_submit(function(element_id, values)
                                    table.insert(SUBMITS, { element_id, values.name })
                                end)
                            "#,
                        ),
                        (
                            "check_values.lua",
                            r#"
                                assert(DIALOG:values().name == "Bob", "initial_value seeds values()")
                                assert(DIALOG:value("name") == "Bob")
                                assert(DIALOG:value("missing") == nil)

                                assert(DIALOG:update("name", { value = "Alice" }) == true)
                                assert(DIALOG:value("name") == "Alice")

                                assert(DIALOG:update("missing", { value = "x" }) == false)
                            "#,
                        ),
                        (
                            "check_callbacks.lua",
                            r#"
                                assert(#CLICKS == 1, "on_select should have fired once")
                                assert(CLICKS[1][1] == "yes")
                                assert(CLICKS[1][2] == "Alice", "click payload carries live values")

                                assert(#SUBMITS == 1, "on_submit should have fired once")
                                assert(SUBMITS[1][1] == "name")
                            "#,
                        ),
                    ],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
                assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);

                harness.run_entrypoint("check_values.lua").unwrap();

                harness.send_event(Event::DialogSelect {
                    id: PopupId(0),
                    button_id: "yes".to_string(),
                    values: HashMap::from([("name".to_string(), "Alice".to_string())]),
                });
                harness.send_event(Event::DialogSubmit {
                    id: PopupId(0),
                    element_id: "name".to_string(),
                    values: HashMap::from([("name".to_string(), "Alice".to_string())]),
                });
                harness.pump_events();

                harness.run_entrypoint("check_callbacks.lua").unwrap();
            })
            .await;
    }

    /// `spawn_dialog` validates the "at most one default button" invariant up front (rather than
    /// only at render time), so a mode author gets a clear error instead of ambiguous Enter-key
    /// routing.
    #[tokio::test(start_paused = true)]
    async fn dialog_rejects_more_than_one_default_button() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local ok = pcall(function()
                                lewdware.popup.dialog({
                                    elements = {
                                        { type = "buttons", options = {
                                            { id = "a", label = "A", default = true },
                                            { id = "b", label = "B", default = true },
                                        }},
                                    },
                                })
                            end)
                            assert(not ok, "a second default button should be rejected")
                        "#,
                    )],
                    false,
                );

                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }
}
