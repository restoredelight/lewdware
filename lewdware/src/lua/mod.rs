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
    behaviour::{Behaviour, Content, Experience, effective_config, effective_options},
    mode::{Metadata, OptionValue, VERSION_MAJOR, read_mode_metadata},
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

/// Resolves the config a mode actually runs with, and — only for the engine's two built-in
/// default modes (`Sandbox`/`Experience`) — the pack's behaviour.json `content`/`experience`
/// sections, handed to the default-modes library code as `__lewdware_content`/
/// `__lewdware_experience` (see `create_api`) for its own query-layer filtering (e.g.
/// `default-modes/shared/lib/media.lua` honoring disabled content groups). Standalone from
/// `start_lua_thread` so it's unit-testable without a full thread spawn.
///
/// Custom modes (`Mode::Pack`/`Mode::File`) never see behaviour data or its synthesized
/// `content_group.*` options — see `behaviour-design/default-mode.md`'s Ownership section ("the
/// toggle is only shown where it is honored") — so they keep the plain schema-defaulting walk.
///
/// `Mode::Experience`'s stored values come from `experience_options[pack_id]` rather than
/// `mode_options[mode]` — its meta-controls are calibrated per pack (see
/// `behaviour-design/default-mode.md`, Ownership), unlike every other mode's global storage.
fn resolve_mode_config(
    metadata: &Metadata,
    mode: &shared::user_config::Mode,
    mode_options: &HashMap<shared::user_config::Mode, HashMap<String, OptionValue>>,
    experience_options: &HashMap<Uuid, HashMap<String, OptionValue>>,
    pack_id: Uuid,
    media_manager: &MediaManager,
    dev_mode: bool,
) -> (HashMap<String, OptionValue>, Content, Experience) {
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
        let mut mode_config = stored;

        for (key, option) in metadata.all_options() {
            if mode_config
                .get(key)
                .is_none_or(|value| !option.matches_value(value))
            {
                mode_config.insert(key.to_string(), option.default_value());
            }
        }

        return (mode_config, Content::default(), Experience::default());
    }

    let behaviour = media_manager
        .get_pack_data("behaviour".to_string())
        .inspect_err(|err| tracing::warn!("Failed to read pack behaviour data: {err}"))
        .ok()
        .flatten()
        .and_then(|bytes| {
            Behaviour::from_json_bytes(&bytes)
                .inspect_err(|err| tracing::warn!("Failed to parse pack behaviour.json: {err}"))
                .ok()
        })
        .unwrap_or_default();

    if dev_mode && behaviour.is_from_newer_engine() {
        tracing::warn!(
            "This pack's behaviour.json (v{}) is newer than this engine understands (v{})",
            behaviour.version,
            shared::behaviour::CURRENT_VERSION
        );
    }

    let schema = effective_options(metadata, &behaviour);
    let resolved = effective_config(&schema, &stored);

    (
        resolved,
        behaviour.content,
        behaviour.experience.unwrap_or_default(),
    )
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
            shared::user_config::Mode::Sandbox => {
                let mode_data =
                    include_bytes!("../../../default-modes/sandbox/build/Sandbox.lwmode");

                Box::new(Cursor::new(mode_data))
            }
            shared::user_config::Mode::Experience => {
                let mode_data =
                    include_bytes!("../../../default-modes/experience/build/Experience.lwmode");

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

        let (mode_config, content, experience) = resolve_mode_config(
            &metadata,
            &config.mode,
            &config.mode_options,
            &config.experience_options,
            pack_info.id,
            &media_manager,
            dev_mode,
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

    use shared::behaviour::{DesignValues, FrequencyAnchors, Level, Modifiers, Timeline};
    use shared::user_config::{Capabilities, Volume};
    use tempfile::NamedTempFile;
    use tokio::sync::mpsc::UnboundedSender;

    use super::*;
    use crate::{app::UserEvent, monitor::Monitor};

    /// Build a minimal but valid, media-less-by-default `.lwpack` fixture on disk, purely so
    /// `MediaManager::open` has something real to read — mirrors the on-disk layout built (and
    /// already exercised) in `media/pack.rs`'s `open_reads_deserialized_index` test. With
    /// `with_image`, `pic.avif`'s row also gets a real `offset`/`length` pointing at dummy bytes
    /// appended to the file (content doesn't matter — `get_image_file` is a raw byte copy, never
    /// decoded here), so `get_image_file`/`wallpaper.set` have real data to read rather than
    /// erroring on a null offset.
    fn pack_fixture(with_image: bool) -> NamedTempFile {
        const IMAGE_BYTES: &[u8] = b"not a real avif, just needs to be some bytes";

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

        let metadata = shared::read_pack::Metadata {
            name: "test-pack".to_string(),
            ..Default::default()
        };
        let metadata_bytes = metadata.to_buf().unwrap();

        let mut header = shared::read_pack::Header::new();
        header.metadata_offset = shared::read_pack::HEADER_SIZE as u64;
        header.metadata_length = metadata_bytes.len() as u64;
        header.index_offset = header.metadata_offset + header.metadata_length;

        // Two-pass, like `media/pack.rs`'s `get_video_data_opens_embedded_clip`: the row's
        // offset needs the db's own serialized length to compute, so serialize once to size it,
        // patch the row, then serialize again.
        let db_bytes = db.serialize(rusqlite::MAIN_DB).unwrap();
        header.index_length = db_bytes.len() as u64;
        let image_offset = header.index_offset + header.index_length;

        if with_image {
            db.execute(
                "UPDATE media SET offset = ?, length = ? WHERE file_name = 'pic.avif'",
                rusqlite::params![image_offset, IMAGE_BYTES.len() as u64],
            )
            .unwrap();
        }
        let db_bytes = db.serialize(rusqlite::MAIN_DB).unwrap();

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&header.to_buf().unwrap()).unwrap();
        file.write_all(&metadata_bytes).unwrap();
        file.write_all(&db_bytes).unwrap();
        if with_image {
            file.write_all(IMAGE_BYTES).unwrap();
        }
        file.flush().unwrap();
        file
    }

    /// Like `pack_fixture(true)`, but the single real image is tagged with the given tags instead
    /// of the hardcoded "red"/"blue" -- for tests needing a real-bytes image tagged e.g.
    /// "wallpaper" or "splash" (`get_image_file`/`wallpaper.set` need real offset/length data,
    /// which `pack_fixture_with_data` below deliberately doesn't attach).
    fn pack_fixture_with_tagged_image(tags: &[&str]) -> NamedTempFile {
        const IMAGE_BYTES: &[u8] = b"not a real avif, just needs to be some bytes";

        let db = rusqlite::Connection::open_in_memory().unwrap();
        shared::db::migrate(&db).unwrap();

        db.execute(
            "INSERT INTO media (file_name, file_type, width, height, transparent, hash) \
             VALUES ('pic.avif', 'image', 64, 64, 0, x'00')",
            [],
        )
        .unwrap();
        for tag in tags {
            db.execute("INSERT INTO tags (name) VALUES (?)", [*tag])
                .unwrap();
            let tag_id = db.last_insert_rowid();
            db.execute(
                "INSERT INTO media_tags (media_id, tag_id) VALUES (1, ?)",
                [tag_id],
            )
            .unwrap();
        }

        let metadata = shared::read_pack::Metadata {
            name: "test-pack".to_string(),
            ..Default::default()
        };
        let metadata_bytes = metadata.to_buf().unwrap();

        let mut header = shared::read_pack::Header::new();
        header.metadata_offset = shared::read_pack::HEADER_SIZE as u64;
        header.metadata_length = metadata_bytes.len() as u64;
        header.index_offset = header.metadata_offset + header.metadata_length;

        let db_bytes = db.serialize(rusqlite::MAIN_DB).unwrap();
        header.index_length = db_bytes.len() as u64;
        let image_offset = header.index_offset + header.index_length;

        db.execute(
            "UPDATE media SET offset = ?, length = ? WHERE file_name = 'pic.avif'",
            rusqlite::params![image_offset, IMAGE_BYTES.len() as u64],
        )
        .unwrap();
        let db_bytes = db.serialize(rusqlite::MAIN_DB).unwrap();

        let mut file = NamedTempFile::new().unwrap();
        file.write_all(&header.to_buf().unwrap()).unwrap();
        file.write_all(&metadata_bytes).unwrap();
        file.write_all(&db_bytes).unwrap();
        file.write_all(IMAGE_BYTES).unwrap();
        file.flush().unwrap();
        file
    }

    /// Build a `.lwpack` fixture with the given tagged image rows (name, tags) and, if `Some`, a
    /// `pack_data` row named `"behaviour"` holding the given bytes. Media rows get no real file
    /// bytes/offsets -- fine for `list`/`random` queries, which never read `offset`/`length` (see
    /// `MediaPack::parse_media`); only `get_image_data`/`get_image_file` would need that.
    fn pack_fixture_with_data(
        media: &[(&str, &[&str])],
        behaviour_bytes: Option<&[u8]>,
    ) -> NamedTempFile {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        shared::db::migrate(&db).unwrap();

        let mut tag_ids: HashMap<&str, i64> = HashMap::new();
        for (index, (file_name, tags)) in media.iter().enumerate() {
            let hash = vec![index as u8];
            db.execute(
                "INSERT INTO media (file_name, file_type, width, height, transparent, hash) \
                 VALUES (?, 'image', 64, 64, 0, ?)",
                rusqlite::params![file_name, hash],
            )
            .unwrap();
            let media_id = db.last_insert_rowid();

            for tag in *tags {
                let tag_id = *tag_ids.entry(tag).or_insert_with(|| {
                    db.execute("INSERT OR IGNORE INTO tags (name) VALUES (?)", [*tag])
                        .unwrap();
                    db.query_row("SELECT id FROM tags WHERE name = ?", [*tag], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap()
                });
                db.execute(
                    "INSERT INTO media_tags (media_id, tag_id) VALUES (?, ?)",
                    rusqlite::params![media_id, tag_id],
                )
                .unwrap();
            }
        }

        if let Some(bytes) = behaviour_bytes {
            db.execute(
                "INSERT INTO pack_data (name, blob) VALUES ('behaviour', ?)",
                rusqlite::params![bytes],
            )
            .unwrap();
        }

        let metadata = shared::read_pack::Metadata {
            name: "test-pack".to_string(),
            ..Default::default()
        };
        let metadata_bytes = metadata.to_buf().unwrap();

        let mut header = shared::read_pack::Header::new();
        header.metadata_offset = shared::read_pack::HEADER_SIZE as u64;
        header.metadata_length = metadata_bytes.len() as u64;
        header.index_offset = header.metadata_offset + header.metadata_length;

        let db_bytes = db.serialize(rusqlite::MAIN_DB).unwrap();
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
        SpawnVideo { media_id: u64, volume: f32 },
        SpawnAudio { media_id: u64, volume: f32 },
        SpawnDialog,
        SpawnText,
        CloseWindow { id: PopupId },
        OpenLink { url: String },
        SetTitle { id: PopupId, title: Option<String> },
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
        capabilities: Capabilities,
        master_volume: Volume,
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
                LuaRequest::SpawnVideo {
                    media_id,
                    volume,
                    tx,
                    ..
                } => {
                    let id = PopupId(next_id);
                    next_id += 1;
                    // Mirrors `LewdwareApp::spawn_video`: master volume is applied where the raw
                    // value first arrives, so what's recorded is the effective volume.
                    let volume = volume * master_volume.video;
                    recorded
                        .lock()
                        .unwrap()
                        .push(Recorded::SpawnVideo { media_id, volume });
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
                LuaRequest::SpawnAudio {
                    media_id,
                    volume,
                    tx,
                    ..
                } => {
                    // See `LuaRequest::SpawnVideo`'s equivalent comment -- same reasoning, the
                    // `audio` channel instead.
                    let volume = volume * master_volume.audio;
                    recorded
                        .lock()
                        .unwrap()
                        .push(Recorded::SpawnAudio { media_id, volume });
                    let _ = tx.send(0);
                }
                LuaRequest::SetWallpaper { tx, .. } => {
                    let _ = tx.send(Ok(capabilities.wallpaper));
                }
                LuaRequest::ResetWallpaper { tx } => {
                    let _ = tx.send(());
                }
                LuaRequest::OpenLink { url, tx } => {
                    // Mirrors `LewdwareApp::open_link`: a disabled capability is checked before
                    // anything else happens, so a denied request leaves no trace to record.
                    if capabilities.open_link {
                        recorded.lock().unwrap().push(Recorded::OpenLink { url });
                    }
                    let _ = tx.send(Ok(capabilities.open_link));
                }
                LuaRequest::ShowNotification { tx, .. } => {
                    let _ = tx.send(Ok(capabilities.notify));
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
                        WindowAction::GetDialogValue { id: element_id, tx } => {
                            let value = dialog_values
                                .get(&id)
                                .and_then(|values| values.get(&element_id))
                                .cloned();
                            let _ = tx.send(value);
                        }
                        WindowAction::SetTitle { tx, title } => {
                            recorded
                                .lock()
                                .unwrap()
                                .push(Recorded::SetTitle { id, title });
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
            Self::with_config(
                sources,
                with_image,
                Capabilities::default(),
                Volume::default(),
            )
        }

        /// Like [`Harness::new`], but with the fake handler gating `SetWallpaper`/`OpenLink`/
        /// `ShowNotification` on `capabilities` exactly like `LewdwareApp` does -- lets a test
        /// simulate a hostile mode running with one or more capabilities denied.
        fn with_capabilities(
            sources: &[(&str, &str)],
            with_image: bool,
            capabilities: Capabilities,
        ) -> Self {
            Self::with_config(sources, with_image, capabilities, Volume::default())
        }

        /// Like [`Harness::new`], but with the fake handler scaling `SpawnVideo`/`SpawnAudio`
        /// volume by `volume` exactly like `LewdwareApp` does -- lets a test check master volume
        /// is applied at spawn time.
        fn with_volume(sources: &[(&str, &str)], with_image: bool, volume: Volume) -> Self {
            Self::with_config(sources, with_image, Capabilities::default(), volume)
        }

        fn with_config(
            sources: &[(&str, &str)],
            with_image: bool,
            capabilities: Capabilities,
            volume: Volume,
        ) -> Self {
            Self::with_pack(
                sources,
                pack_fixture(with_image),
                Content::default(),
                HashMap::new(),
                capabilities,
                volume,
            )
        }

        /// Like `with_config`, but for tests that need a specific pack fixture (e.g. several
        /// distinctly-tagged media rows) and/or behaviour-derived `content`/`mode_config` --
        /// exactly what `resolve_mode_config` would have produced for `Mode::Sandbox`. The
        /// content-group query-layer tests use this to exercise `lib/media.lua` against real
        /// media without going through the full pack-behaviour.json resolution path.
        fn with_pack(
            sources: &[(&str, &str)],
            pack_file: NamedTempFile,
            content: Content,
            mode_config: HashMap<String, OptionValue>,
            capabilities: Capabilities,
            volume: Volume,
        ) -> Self {
            Self::with_pack_and_experience(
                sources,
                pack_file,
                content,
                Experience::default(),
                mode_config,
                capabilities,
                volume,
            )
        }

        /// Like `with_pack`, but also injects a custom `Experience` section (anchors/design
        /// values) as `__lewdware_experience` -- for exercising
        /// `default-modes/experience/src/main.lua` end to end the same way `with_pack` exercises
        /// Sandbox's.
        #[allow(clippy::too_many_arguments)]
        fn with_pack_and_experience(
            sources: &[(&str, &str)],
            pack_file: NamedTempFile,
            content: Content,
            experience: Experience,
            mode_config: HashMap<String, OptionValue>,
            capabilities: Capabilities,
            volume: Volume,
        ) -> Self {
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
                thread::spawn(move || {
                    run_fake_handler(request_rx, recorded, event_tx, capabilities, volume)
                });
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
                        config: mode_config,
                        content,
                        experience,
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

        /// Evaluates a Lua expression against the running mode's global state and returns it as
        /// a number -- e.g. a counter a test's own wrapped API function incremented (see
        /// `wrapped_default_mode_sources`). Not needed for anything the fake handler already
        /// tracks via `Recorded`; only for effects it doesn't (e.g. `show_notification`, which
        /// carries no `Recorded` variant since nothing else needs to assert on it).
        fn eval_number(&self, expr: &str) -> f64 {
            self.runtime.lua.load(expr).eval().unwrap()
        }

        /// Like `eval_number`, but for a string-valued global.
        fn eval_string(&self, expr: &str) -> String {
            self.runtime.lua.load(expr).eval().unwrap()
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
                        recommended_mode: None,
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
                        content: Content::default(),
                        experience: Experience::default(),
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
                        content: Content::default(),
                        experience: Experience::default(),
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

    /// The user-owned master volume (`AppConfig.volume`) is independent per channel -- video's
    /// embedded audio track vs. standalone `play_audio` -- mirroring the engine's own API split.
    /// Applied where the raw spawn-time volume from Lua first enters `LewdwareApp` (see
    /// `spawn_video`/`spawn_audio`'s comments), reproduced here in the fake handler (see
    /// `Harness::with_volume`) so the effective volume is exactly what a real session would use.
    #[tokio::test(start_paused = true)]
    async fn master_volume_scales_spawn_time_volume_independently_per_channel() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::with_volume(
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
                            lewdware.popup.video(video, { volume = 0.6 })

                            local audio = { id = 0, name = "test", type = "audio", duration = 1.0 }
                            lewdware.play_audio(audio, { volume = 0.8 })
                        "#,
                    )],
                    false,
                    Volume {
                        video: 0.5,
                        audio: 0.25,
                    },
                );

                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(
                    harness.recorded(),
                    vec![
                        Recorded::SpawnVideo {
                            media_id: 0,
                            volume: 0.3, // 0.6 requested * 0.5 master
                        },
                        Recorded::SpawnAudio {
                            media_id: 0,
                            volume: 0.2, // 0.8 requested * 0.25 master
                        },
                    ]
                );
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

    /// Capability toggles are the one consent-critical surface the release plan calls out as
    /// "not cuttable": a hostile mode with every capability denied must see `false` from each
    /// call (never a thrown error -- these are "could not do it" outcomes, indistinguishable to
    /// Lua from e.g. no browser being installed) and must leave no side effect, exactly as
    /// `LewdwareApp::open_link`/`show_notification`/`set_wallpaper` behave for real (see
    /// `Harness::with_capabilities`, which mirrors that gating in the fake handler).
    #[tokio::test(start_paused = true)]
    async fn hostile_mode_sees_every_denied_capability_as_false() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::with_capabilities(
                    &[(
                        "main.lua",
                        r#"
                            local image = lewdware.media.get_image("pic.avif")
                            assert(lewdware.wallpaper.set(image) == false,
                                "wallpaper.set should be denied")
                            assert(lewdware.open_link("https://should-not-fire.example") == false,
                                "open_link should be denied")
                            assert(lewdware.show_notification({ body = "should not fire" }) == false,
                                "show_notification should be denied")
                        "#,
                    )],
                    true,
                    Capabilities {
                        wallpaper: false,
                        open_link: false,
                        notify: false,
                    },
                );

                harness.run_entrypoint("main.lua").unwrap();

                // No side effect leaked through -- in particular, the denied `open_link` never
                // reached the point the fake handler records it.
                assert_eq!(harness.recorded(), vec![]);
            })
            .await;
    }

    #[test]
    fn media_manager_get_pack_data_round_trips_a_named_blob() {
        let pack_file = pack_fixture_with_data(&[], Some(b"hello behaviour"));
        let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);
        let (media_manager, _metadata, _pack_id, _handle) =
            MediaManager::open(pack_file.path(), event_poster, None).unwrap();

        assert_eq!(
            media_manager
                .get_pack_data("behaviour".to_string())
                .unwrap(),
            Some(b"hello behaviour".to_vec())
        );
        assert_eq!(
            media_manager
                .get_pack_data("nonexistent".to_string())
                .unwrap(),
            None
        );
    }

    fn empty_default_mode_metadata() -> Metadata {
        Metadata {
            name: "test-mode".to_string(),
            version: None,
            author: None,
            entrypoint: "main.lua".to_string(),
            entries: Default::default(),
            files: HashMap::new(),
        }
    }

    fn behaviour_with_one_content_group() -> Behaviour {
        Behaviour {
            content: Content {
                content_groups: vec![shared::behaviour::ContentGroup {
                    id: "kinky".to_string(),
                    label: "Kinky".to_string(),
                    description: None,
                    tags: vec!["kinky".to_string()],
                    enabled_by_default: true,
                }],
                ..Default::default()
            },
            ..Behaviour::new()
        }
    }

    /// The core invariant `resolve_mode_config` exists for: for the built-in Sandbox mode, a
    /// content group synthesizes into the resolved config (falling back to its
    /// `enabled_by_default`, or a stored override when present) and its tags are handed back via
    /// `Content` for the query layer -- see `default-modes/shared/lib/media.lua`.
    #[test]
    fn resolve_mode_config_synthesizes_content_group_toggle_for_sandbox_mode() {
        let behaviour = behaviour_with_one_content_group();
        let behaviour_bytes = behaviour.to_json_bytes().unwrap();
        let pack_file = pack_fixture_with_data(&[], Some(&behaviour_bytes));

        let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);
        let (media_manager, _metadata, pack_id, _handle) =
            MediaManager::open(pack_file.path(), event_poster, None).unwrap();

        let metadata = empty_default_mode_metadata();
        let key = format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX);

        // No stored override -> falls back to the group's `enabled_by_default`.
        let (config, content, _experience) = resolve_mode_config(
            &metadata,
            &shared::user_config::Mode::Sandbox,
            &HashMap::new(),
            &HashMap::new(),
            pack_id,
            &media_manager,
            false,
        );
        assert_eq!(config.get(&key), Some(&OptionValue::Boolean(true)));
        assert_eq!(content.content_groups.len(), 1);
        assert_eq!(content.content_groups[0].tags, vec!["kinky".to_string()]);

        // A stored override wins over the default.
        let mut stored = HashMap::new();
        stored.insert(key.clone(), OptionValue::Boolean(false));
        let mut mode_options = HashMap::new();
        mode_options.insert(shared::user_config::Mode::Sandbox, stored);

        let (config, _, _experience) = resolve_mode_config(
            &metadata,
            &shared::user_config::Mode::Sandbox,
            &mode_options,
            &HashMap::new(),
            pack_id,
            &media_manager,
            false,
        );
        assert_eq!(config.get(&key), Some(&OptionValue::Boolean(false)));
    }

    /// The actual "custom modes never see the toggles" invariant (`behaviour-design/
    /// default-mode.md`, Ownership): even against a pack whose behaviour.json declares content
    /// groups, a mode embedded in that pack gets neither the synthesized option nor the content
    /// data.
    #[test]
    fn resolve_mode_config_hides_content_groups_from_custom_modes() {
        let behaviour = behaviour_with_one_content_group();
        let behaviour_bytes = behaviour.to_json_bytes().unwrap();
        let pack_file = pack_fixture_with_data(&[], Some(&behaviour_bytes));

        let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);
        let (media_manager, _metadata, pack_id, _handle) =
            MediaManager::open(pack_file.path(), event_poster, None).unwrap();

        let metadata = empty_default_mode_metadata();
        let key = format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX);

        for mode in [
            shared::user_config::Mode::Pack { id: 1 },
            shared::user_config::Mode::File {
                path: "some/mode.lwmode".into(),
            },
        ] {
            let (config, content, _experience) = resolve_mode_config(
                &metadata,
                &mode,
                &HashMap::new(),
                &HashMap::new(),
                pack_id,
                &media_manager,
                false,
            );

            assert_eq!(config.get(&key), None);
            assert_eq!(content, Content::default());
        }
    }

    /// A pack with no behaviour.json at all (the common case today) shouldn't panic or fail the
    /// mode -- `resolve_mode_config` falls back to an empty `Behaviour`.
    #[test]
    fn resolve_mode_config_sandbox_mode_with_no_behaviour_data() {
        let pack_file = pack_fixture_with_data(&[], None);

        let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);
        let (media_manager, _metadata, pack_id, _handle) =
            MediaManager::open(pack_file.path(), event_poster, None).unwrap();

        let metadata = empty_default_mode_metadata();

        let (config, content, _experience) = resolve_mode_config(
            &metadata,
            &shared::user_config::Mode::Sandbox,
            &HashMap::new(),
            &HashMap::new(),
            pack_id,
            &media_manager,
            false,
        );

        assert!(config.is_empty());
        assert_eq!(content, Content::default());
    }

    /// Integration-style: exercises the *shipped* `default-modes/src/lib/media.lua` file (not a
    /// re-implementation of its logic), confirming a disabled content group's tagged media never
    /// surfaces from `media.list()` -- the "no query path bypasses it" gate from the release
    /// plan's M3 content-groups bullet.
    #[tokio::test(start_paused = true)]
    async fn shared_media_query_layer_honors_disabled_content_groups() {
        LocalSet::new()
            .run_until(async {
                const MEDIA: &[(&str, &[&str])] = &[
                    ("kinky1.avif", &["kinky"]),
                    ("kinky2.avif", &["kinky"]),
                    ("vanilla1.avif", &[]),
                ];

                let content = Content {
                    content_groups: vec![shared::behaviour::ContentGroup {
                        id: "kinky".to_string(),
                        label: "Kinky".to_string(),
                        description: None,
                        tags: vec!["kinky".to_string()],
                        enabled_by_default: true,
                    }],
                    ..Default::default()
                };

                let sources: &[(&str, &str)] = &[
                    (
                        "main.lua",
                        r#"
                            local media = require("lib.media")
                            local items = media.list({ type = { "image" } })

                            local seen = {}
                            for _, item in ipairs(items) do seen[item.name] = true end

                            assert(seen["vanilla1.avif"], "untagged media should always be present")
                            if lewdware.config["content_group.kinky"] then
                                assert(seen["kinky1.avif"], "kinky1 should be present when enabled")
                                assert(seen["kinky2.avif"], "kinky2 should be present when enabled")
                            else
                                assert(not seen["kinky1.avif"], "kinky1 should be excluded when disabled")
                                assert(not seen["kinky2.avif"], "kinky2 should be excluded when disabled")
                            end
                        "#,
                    ),
                    (
                        "lib/media.lua",
                        include_str!("../../../default-modes/shared/lib/media.lua"),
                    ),
                ];

                let key = format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX);

                // Group enabled: all three items present.
                let mut enabled_config = HashMap::new();
                enabled_config.insert(key.clone(), OptionValue::Boolean(true));
                let mut harness = Harness::with_pack(
                    sources,
                    pack_fixture_with_data(MEDIA, None),
                    content.clone(),
                    enabled_config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();

                // Group disabled: the kinky-tagged media never surfaces.
                let mut disabled_config = HashMap::new();
                disabled_config.insert(key, OptionValue::Boolean(false));
                let mut harness = Harness::with_pack(
                    sources,
                    pack_fixture_with_data(MEDIA, None),
                    content,
                    disabled_config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    /// Builds the `(sources, Content)` pair shared by `lib/content.lua`'s own tests: no media
    /// pack fixture needed beyond the media-less-by-default default (the picker never touches
    /// `lewdware.media.*`), so every test below runs `main.lua` against `pack_fixture(false)`.
    fn content_lib_sources(main: &str) -> Vec<(&str, &str)> {
        vec![
            ("main.lua", main),
            (
                "lib/content.lua",
                include_str!("../../../default-modes/shared/lib/content.lua"),
            ),
            (
                "lib/media.lua",
                include_str!("../../../default-modes/shared/lib/media.lua"),
            ),
        ]
    }

    /// The real `default-modes` mode's own source files (not test-only stand-ins) -- for tests
    /// that exercise `main.lua`'s actual process wiring end-to-end rather than a re-implementation
    /// of its logic. Extended with additional `lib/*.lua` entries as later milestone phases add
    /// new process modules.
    fn real_default_mode_sources() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "main.lua",
                include_str!("../../../default-modes/sandbox/src/main.lua"),
            ),
            (
                "lib/media.lua",
                include_str!("../../../default-modes/shared/lib/media.lua"),
            ),
            (
                "lib/content.lua",
                include_str!("../../../default-modes/shared/lib/content.lua"),
            ),
            (
                "lib/notifications.lua",
                include_str!("../../../default-modes/shared/lib/notifications.lua"),
            ),
            (
                "lib/web.lua",
                include_str!("../../../default-modes/shared/lib/web.lua"),
            ),
            (
                "lib/subliminals.lua",
                include_str!("../../../default-modes/shared/lib/subliminals.lua"),
            ),
            (
                "lib/prompts.lua",
                include_str!("../../../default-modes/shared/lib/prompts.lua"),
            ),
            (
                "lib/wallpaper.lua",
                include_str!("../../../default-modes/shared/lib/wallpaper.lua"),
            ),
            (
                "lib/spawn.lua",
                include_str!("../../../default-modes/shared/lib/spawn.lua"),
            ),
        ]
    }

    /// Like `real_default_mode_sources`, but the real `main.lua` is aliased to `real_main.lua` and
    /// `setup` runs first as `main.lua` -- lets a test wrap an API function (e.g.
    /// `lewdware.show_notification`, `lewdware.popup.dialog`) before the real mode's processes
    /// start. This works because the library modules look up `lewdware.*` fields fresh each time
    /// their scheduled callback fires, not once at `require()`/`start()` time, so the override
    /// only needs to land before the first `advance()`, not before `require("real_main")` itself.
    fn wrapped_default_mode_sources(setup: &str) -> Vec<(String, String)> {
        let mut sources = vec![(
            "main.lua".to_string(),
            format!("{setup}\nrequire(\"real_main\")\n"),
        )];
        for (path, source) in real_default_mode_sources() {
            let path = if path == "main.lua" {
                "real_main.lua"
            } else {
                path
            };
            sources.push((path.to_string(), source.to_string()));
        }
        sources
    }

    /// Like `real_default_mode_sources`, but the shipped Experience mode
    /// (`default-modes/experience/src/main.lua`) -- everything except `main.lua` is shared with
    /// Sandbox (`lib/*.lua`), so both source lists point at the same files.
    fn real_experience_mode_sources() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "main.lua",
                include_str!("../../../default-modes/experience/src/main.lua"),
            ),
            (
                "timeline.lua",
                include_str!("../../../default-modes/experience/src/timeline.lua"),
            ),
            (
                "lib/media.lua",
                include_str!("../../../default-modes/shared/lib/media.lua"),
            ),
            (
                "lib/content.lua",
                include_str!("../../../default-modes/shared/lib/content.lua"),
            ),
            (
                "lib/notifications.lua",
                include_str!("../../../default-modes/shared/lib/notifications.lua"),
            ),
            (
                "lib/web.lua",
                include_str!("../../../default-modes/shared/lib/web.lua"),
            ),
            (
                "lib/subliminals.lua",
                include_str!("../../../default-modes/shared/lib/subliminals.lua"),
            ),
            (
                "lib/prompts.lua",
                include_str!("../../../default-modes/shared/lib/prompts.lua"),
            ),
            (
                "lib/wallpaper.lua",
                include_str!("../../../default-modes/shared/lib/wallpaper.lua"),
            ),
            (
                "lib/spawn.lua",
                include_str!("../../../default-modes/shared/lib/spawn.lua"),
            ),
        ]
    }

    /// Like `wrapped_default_mode_sources`, but for Experience (see `real_experience_mode_sources`).
    fn wrapped_experience_mode_sources(setup: &str) -> Vec<(String, String)> {
        let mut sources = vec![(
            "main.lua".to_string(),
            format!("{setup}\nrequire(\"real_main\")\n"),
        )];
        for (path, source) in real_experience_mode_sources() {
            let path = if path == "main.lua" {
                "real_main.lua"
            } else {
                path
            };
            sources.push((path.to_string(), source.to_string()));
        }
        sources
    }

    /// Baseline `mode_config` covering every key `default-modes/src/main.lua` reads unconditionally
    /// at startup -- image-only, constant-interval spawning at 1 popup/second, every optional
    /// feature off. The harness hands Lua `lewdware.config` exactly as given (unlike the real app,
    /// which resolves defaults first via `resolve_mode_config`), so a test running the real
    /// `main.lua` must supply every key it reads; tests extend/override individual keys via
    /// `.insert(...)` on the returned map.
    fn base_default_mode_config() -> HashMap<String, OptionValue> {
        let mut config = HashMap::new();
        config.insert(
            "spawn_mode".to_string(),
            OptionValue::Enum("constant".to_string()),
        );
        config.insert("popup_frequency".to_string(), OptionValue::Number(1.0));
        config.insert("max_popups".to_string(), OptionValue::Integer(5));
        config.insert("images_enabled".to_string(), OptionValue::Boolean(true));
        config.insert("videos_enabled".to_string(), OptionValue::Boolean(false));
        config.insert("audio_enabled".to_string(), OptionValue::Boolean(false));
        config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(false));
        config.insert(
            "close_trigger_enabled".to_string(),
            OptionValue::Boolean(false),
        );
        config.insert("movement_enabled".to_string(), OptionValue::Boolean(false));
        config.insert("captions_enabled".to_string(), OptionValue::Boolean(true));
        config
    }

    /// `base_default_mode_config()` with popup spawning turned off (`images_enabled = false`),
    /// for tests exercising one of the frequency-scheduled processes (notifications/web/
    /// subliminals/prompts) in isolation, unpolluted by the ordinary spawn loop's own
    /// `SpawnImage`/`SetTitle` traffic.
    fn isolated_process_config() -> HashMap<String, OptionValue> {
        let mut config = base_default_mode_config();
        config.insert("images_enabled".to_string(), OptionValue::Boolean(false));
        config
    }

    /// Like `base_default_mode_config`, but for Experience's meta-controls schema
    /// (`default-modes/experience/config.jsonc`): `pace = 1.0` (no scaling), image-only spawning,
    /// every off-switch on (rate/design values themselves come from the test's `Experience`
    /// fixture, not this config map -- see `behaviour-design/default-mode.md`, Ownership).
    fn base_experience_mode_config() -> HashMap<String, OptionValue> {
        let mut config = HashMap::new();
        config.insert("pace".to_string(), OptionValue::Number(1.0));
        config.insert("max_popups".to_string(), OptionValue::Integer(5));
        config.insert("images_enabled".to_string(), OptionValue::Boolean(true));
        config.insert("videos_enabled".to_string(), OptionValue::Boolean(false));
        config.insert("audio_enabled".to_string(), OptionValue::Boolean(false));
        config.insert(
            "close_trigger_enabled".to_string(),
            OptionValue::Boolean(false),
        );
        config.insert("movement_enabled".to_string(), OptionValue::Boolean(false));
        config.insert("captions_enabled".to_string(), OptionValue::Boolean(true));
        config.insert(
            "notifications_enabled".to_string(),
            OptionValue::Boolean(true),
        );
        config.insert(
            "web_opening_enabled".to_string(),
            OptionValue::Boolean(true),
        );
        config.insert(
            "subliminals_enabled".to_string(),
            OptionValue::Boolean(true),
        );
        config.insert("subliminal_opacity".to_string(), OptionValue::Number(0.5));
        config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
        config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));
        config.insert("splash_enabled".to_string(), OptionValue::Boolean(true));
        config
    }

    /// `base_experience_mode_config()` with popup spawning turned off, mirroring
    /// `isolated_process_config`'s reasoning.
    fn isolated_experience_process_config() -> HashMap<String, OptionValue> {
        let mut config = base_experience_mode_config();
        config.insert("images_enabled".to_string(), OptionValue::Boolean(false));
        config
    }

    fn as_str_sources(owned: &[(String, String)]) -> Vec<(&str, &str)> {
        owned
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect()
    }

    const COUNTING_NOTIFICATION_WRAP: &str = r#"
        COUNT = 0
        local real = lewdware.show_notification
        lewdware.show_notification = function(n)
            COUNT = COUNT + 1
            return real(n)
        end
    "#;

    #[tokio::test(start_paused = true)]
    async fn notifications_fire_at_configured_frequency() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    notifications: vec![shared::behaviour::TextItem {
                        text: "hi".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(COUNTING_NOTIFICATION_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert(
                    "notifications_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert(
                    "notification_frequency".to_string(),
                    OptionValue::Number(1.0),
                );

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(3100)).await;

                assert_eq!(harness.eval_number("COUNT"), 3.0);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn notifications_skip_when_pool_empty_but_enabled() {
        LocalSet::new()
            .run_until(async {
                let owned = wrapped_default_mode_sources(COUNTING_NOTIFICATION_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert(
                    "notifications_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert(
                    "notification_frequency".to_string(),
                    OptionValue::Number(1.0),
                );

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    Content::default(),
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(2100)).await;

                assert_eq!(harness.eval_number("COUNT"), 0.0);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn notifications_disabled_never_fires() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    notifications: vec![shared::behaviour::TextItem {
                        text: "hi".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(COUNTING_NOTIFICATION_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert(
                    "notifications_enabled".to_string(),
                    OptionValue::Boolean(false),
                );
                config.insert(
                    "notification_frequency".to_string(),
                    OptionValue::Number(1.0),
                );

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(2100)).await;

                assert_eq!(harness.eval_number("COUNT"), 0.0);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn notifications_pause_while_dormant() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    notifications: vec![shared::behaviour::TextItem {
                        text: "hi".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(COUNTING_NOTIFICATION_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert(
                    "notifications_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                // Ticks at 300ms/600ms/900ms/1200ms...; `active_min`/`active_max` (and
                // `dormant_min`/`dormant_max`) are whole seconds -- `schedule_dormancy` passes
                // them straight to `math.random(m, n)`, which requires integer-valued arguments.
                // Dormancy triggers at 1000ms and stays dormant for the rest of the test
                // (dormant_min/max = 10s): the 300/600/900ms ticks should fire, the 1200ms+ ticks
                // should all be suppressed.
                config.insert(
                    "notification_frequency".to_string(),
                    OptionValue::Number(0.3),
                );
                config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(true));
                config.insert("active_min".to_string(), OptionValue::Number(1.0));
                config.insert("active_max".to_string(), OptionValue::Number(1.0));
                config.insert("dormant_min".to_string(), OptionValue::Number(10.0));
                config.insert("dormant_max".to_string(), OptionValue::Number(10.0));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1300)).await;

                assert_eq!(harness.eval_number("COUNT"), 3.0);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn web_link_opens_with_random_arg_appended() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    web_links: vec![shared::behaviour::WebLink {
                        url: "https://example.com/?q=".to_string(),
                        args: vec!["a".to_string(), "b".to_string()],
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert(
                    "web_opening_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert("web_frequency".to_string(), OptionValue::Number(1.0));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1100)).await;

                let recorded = harness.recorded();
                assert_eq!(recorded.len(), 1);
                match &recorded[0] {
                    Recorded::OpenLink { url } => assert!(
                        url == "https://example.com/?q=a" || url == "https://example.com/?q=b",
                        "unexpected url: {url}"
                    ),
                    other => panic!("expected OpenLink, got {other:?}"),
                }
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn web_link_opens_url_unmodified_when_no_args() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    web_links: vec![shared::behaviour::WebLink {
                        url: "https://example.com/".to_string(),
                        args: vec![],
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert(
                    "web_opening_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert("web_frequency".to_string(), OptionValue::Number(1.0));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1100)).await;

                assert_eq!(
                    harness.recorded(),
                    vec![Recorded::OpenLink {
                        url: "https://example.com/".to_string()
                    }]
                );
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn web_opening_skips_when_pool_empty() {
        LocalSet::new()
            .run_until(async {
                let mut config = isolated_process_config();
                config.insert(
                    "web_opening_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert("web_frequency".to_string(), OptionValue::Number(1.0));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    Content::default(),
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(2100)).await;

                assert_eq!(harness.recorded(), vec![]);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn web_opening_pauses_while_dormant() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    web_links: vec![shared::behaviour::WebLink {
                        url: "https://example.com/".to_string(),
                        args: vec![],
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert(
                    "web_opening_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                // Same timing scheme as `notifications_pause_while_dormant`: ticks at
                // 300/600/900/1200ms, dormancy triggers at 1000ms and stays dormant for the rest
                // of the test.
                config.insert("web_frequency".to_string(), OptionValue::Number(0.3));
                config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(true));
                config.insert("active_min".to_string(), OptionValue::Number(1.0));
                config.insert("active_max".to_string(), OptionValue::Number(1.0));
                config.insert("dormant_min".to_string(), OptionValue::Number(10.0));
                config.insert("dormant_max".to_string(), OptionValue::Number(10.0));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1300)).await;

                assert_eq!(harness.recorded().len(), 3);
            })
            .await;
    }

    const OPACITY_CAPTURE_WRAP: &str = r#"
        OPACITY = -1
        local real = lewdware.popup.text
        lewdware.popup.text = function(text, opts)
            OPACITY = opts.opacity
            return real(text, opts)
        end
    "#;

    #[tokio::test(start_paused = true)]
    async fn subliminal_flashes_then_closes_after_flash_duration() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    subliminals: vec![shared::behaviour::TextItem {
                        text: "Obey".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert(
                    "subliminals_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert("subliminal_frequency".to_string(), OptionValue::Number(1.0));
                config.insert("subliminal_opacity".to_string(), OptionValue::Number(0.5));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1100)).await;
                assert_eq!(harness.recorded(), vec![Recorded::SpawnText]);

                // FLASH_DURATION_MS = 200 -- the window should self-close shortly after spawning.
                harness.advance(Duration::from_millis(300)).await;
                assert_eq!(
                    harness.recorded(),
                    vec![
                        Recorded::SpawnText,
                        Recorded::CloseWindow { id: PopupId(0) },
                    ]
                );
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn subliminal_opacity_passed_through() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    subliminals: vec![shared::behaviour::TextItem {
                        text: "Obey".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(OPACITY_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert(
                    "subliminals_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert("subliminal_frequency".to_string(), OptionValue::Number(1.0));
                config.insert("subliminal_opacity".to_string(), OptionValue::Number(0.3));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1100)).await;

                assert_eq!(harness.eval_number("OPACITY"), 0.3);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn subliminal_skips_when_pool_empty() {
        LocalSet::new()
            .run_until(async {
                let mut config = isolated_process_config();
                config.insert(
                    "subliminals_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert("subliminal_frequency".to_string(), OptionValue::Number(1.0));
                config.insert("subliminal_opacity".to_string(), OptionValue::Number(0.5));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    Content::default(),
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(2100)).await;

                assert_eq!(harness.recorded(), vec![]);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn subliminals_pause_while_dormant() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    subliminals: vec![shared::behaviour::TextItem {
                        text: "Obey".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert(
                    "subliminals_enabled".to_string(),
                    OptionValue::Boolean(true),
                );
                config.insert("subliminal_opacity".to_string(), OptionValue::Number(0.5));
                // Same timing scheme as `notifications_pause_while_dormant`: ticks at
                // 300/600/900/1200ms, dormancy triggers at 1000ms. Each firing spawns then closes
                // 200ms later, so 3 firings before dormancy produce 6 recorded events total
                // (spawn+close for each), and the 1200ms tick is suppressed.
                config.insert("subliminal_frequency".to_string(), OptionValue::Number(0.3));
                config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(true));
                config.insert("active_min".to_string(), OptionValue::Number(1.0));
                config.insert("active_max".to_string(), OptionValue::Number(1.0));
                config.insert("dormant_min".to_string(), OptionValue::Number(10.0));
                config.insert("dormant_max".to_string(), OptionValue::Number(10.0));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1300)).await;

                assert_eq!(harness.recorded().len(), 6);
            })
            .await;
    }

    const DIALOG_ELEMENT_CAPTURE_WRAP: &str = r#"
        DIALOG_TEXT = nil
        DIALOG_LABEL = nil
        local real = lewdware.popup.dialog
        lewdware.popup.dialog = function(opts)
            for _, el in ipairs(opts.elements) do
                if el.type == "text" then DIALOG_TEXT = el.text end
                if el.type == "buttons" then DIALOG_LABEL = el.options[1].label end
            end
            return real(opts)
        end
    "#;

    #[tokio::test(start_paused = true)]
    async fn prompt_dialog_spawns_with_pool_text_and_configured_submit_label() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    prompts: vec![shared::behaviour::TextItem {
                        text: "Well?".to_string(),
                        tags: vec![],
                    }],
                    prompt_settings: shared::behaviour::PromptSettings {
                        submit_label: Some("Confirm".to_string()),
                    },
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(DIALOG_ELEMENT_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
                config.insert("prompt_frequency".to_string(), OptionValue::Number(1.0));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1100)).await;

                assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);
                assert_eq!(harness.eval_string("DIALOG_TEXT"), "Well?");
                assert_eq!(harness.eval_string("DIALOG_LABEL"), "Confirm");
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn prompt_dialog_falls_back_to_default_submit_label_when_unset() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    prompts: vec![shared::behaviour::TextItem {
                        text: "Well?".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(DIALOG_ELEMENT_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
                config.insert("prompt_frequency".to_string(), OptionValue::Number(1.0));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1100)).await;

                assert_eq!(harness.eval_string("DIALOG_LABEL"), "Submit");
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn prompt_submit_and_select_both_close_the_dialog() {
        async fn spawn_one_prompt_dialog() -> Harness {
            let content = Content {
                prompts: vec![shared::behaviour::TextItem {
                    text: "Well?".to_string(),
                    tags: vec![],
                }],
                ..Default::default()
            };

            let mut config = isolated_process_config();
            config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
            config.insert("prompt_frequency".to_string(), OptionValue::Number(1.0));

            let mut harness = Harness::with_pack(
                &real_default_mode_sources(),
                pack_fixture(false),
                content,
                config,
                Capabilities::default(),
                Volume::default(),
            );
            harness.run_entrypoint("main.lua").unwrap();
            harness.advance(Duration::from_millis(1100)).await;
            assert_eq!(harness.recorded(), vec![Recorded::SpawnDialog]);
            harness
        }

        LocalSet::new()
            .run_until(async {
                let mut harness = spawn_one_prompt_dialog().await;
                harness.send_event(Event::DialogSelect {
                    id: PopupId(0),
                    button_id: "submit".to_string(),
                    values: HashMap::new(),
                });
                harness.pump_events();
                assert_eq!(
                    harness.recorded(),
                    vec![
                        Recorded::SpawnDialog,
                        Recorded::CloseWindow { id: PopupId(0) },
                    ]
                );

                let mut harness = spawn_one_prompt_dialog().await;
                harness.send_event(Event::DialogSubmit {
                    id: PopupId(0),
                    element_id: "response".to_string(),
                    values: HashMap::new(),
                });
                harness.pump_events();
                assert_eq!(
                    harness.recorded(),
                    vec![
                        Recorded::SpawnDialog,
                        Recorded::CloseWindow { id: PopupId(0) },
                    ]
                );
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn prompts_skip_when_pool_empty() {
        LocalSet::new()
            .run_until(async {
                let mut config = isolated_process_config();
                config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
                config.insert("prompt_frequency".to_string(), OptionValue::Number(1.0));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    Content::default(),
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(2100)).await;

                assert_eq!(harness.recorded(), vec![]);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn prompts_pause_while_dormant() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    prompts: vec![shared::behaviour::TextItem {
                        text: "Well?".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert("prompts_enabled".to_string(), OptionValue::Boolean(true));
                // Same timing scheme as `notifications_pause_while_dormant`: ticks at
                // 300/600/900/1200ms, dormancy triggers at 1000ms and stays dormant for the rest
                // of the test -- the 300/600/900ms ticks should spawn a dialog, the 1200ms tick
                // should be suppressed.
                config.insert("prompt_frequency".to_string(), OptionValue::Number(0.3));
                config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(true));
                config.insert("active_min".to_string(), OptionValue::Number(1.0));
                config.insert("active_max".to_string(), OptionValue::Number(1.0));
                config.insert("dormant_min".to_string(), OptionValue::Number(10.0));
                config.insert("dormant_max".to_string(), OptionValue::Number(10.0));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1300)).await;

                assert_eq!(harness.recorded().len(), 3);
            })
            .await;
    }

    const WALLPAPER_CAPTURE_WRAP: &str = r#"
        SET_COUNT = 0
        RESET_COUNT = 0
        SET_IMAGE_NAME = nil
        local real_set = lewdware.wallpaper.set
        lewdware.wallpaper.set = function(image, opts)
            SET_COUNT = SET_COUNT + 1
            SET_IMAGE_NAME = image.name
            return real_set(image, opts)
        end
        local real_reset = lewdware.wallpaper.reset
        lewdware.wallpaper.reset = function()
            RESET_COUNT = RESET_COUNT + 1
            return real_reset()
        end
    "#;

    #[tokio::test(start_paused = true)]
    async fn wallpaper_applied_once_at_mode_start() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    wallpaper_tags: vec!["wallpaper".to_string()],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture_with_tagged_image(&["wallpaper"]),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(harness.eval_number("SET_COUNT"), 1.0);
                assert_eq!(harness.eval_string("SET_IMAGE_NAME"), "pic.avif");
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn wallpaper_skips_silently_with_no_wallpaper_tagged_media() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    wallpaper_tags: vec!["wallpaper".to_string()],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(harness.eval_number("SET_COUNT"), 0.0);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn wallpaper_not_applied_when_no_tags_declared() {
        // Opt-in only: `wallpaper_tags` empty (the pack never declared any) means no wallpaper
        // feature at all, even if `wallpaper_enabled` is somehow true (defensive against a custom
        // mode reusing this library, or a stale stored option) and the fixture has real media.
        LocalSet::new()
            .run_until(async {
                let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(true),
                    Content::default(),
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(harness.eval_number("SET_COUNT"), 0.0);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn wallpaper_honors_a_custom_configured_tag() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    wallpaper_tags: vec!["bg".to_string()],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture_with_tagged_image(&["bg"]),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(harness.eval_number("SET_COUNT"), 1.0);
                assert_eq!(harness.eval_string("SET_IMAGE_NAME"), "pic.avif");
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn wallpaper_resets_on_dormancy_sleep_and_reapplies_on_wake() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    wallpaper_tags: vec!["wallpaper".to_string()],
                    ..Default::default()
                };

                let owned = wrapped_default_mode_sources(WALLPAPER_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = isolated_process_config();
                config.insert("wallpaper_enabled".to_string(), OptionValue::Boolean(true));
                // Whole-second bounds: schedule_dormancy passes these straight to
                // math.random(m, n), which requires integer-valued arguments (see
                // notifications_pause_while_dormant). 1s active, 1s dormant: reset should happen
                // ~1000ms in, reapply ~2000ms in.
                config.insert("dormancy_enabled".to_string(), OptionValue::Boolean(true));
                config.insert("active_min".to_string(), OptionValue::Number(1.0));
                config.insert("active_max".to_string(), OptionValue::Number(1.0));
                config.insert("dormant_min".to_string(), OptionValue::Number(1.0));
                config.insert("dormant_max".to_string(), OptionValue::Number(1.0));

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture_with_tagged_image(&["wallpaper"]),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                assert_eq!(harness.eval_number("SET_COUNT"), 1.0);

                harness.advance(Duration::from_millis(1100)).await;
                assert_eq!(harness.eval_number("RESET_COUNT"), 1.0);
                assert_eq!(
                    harness.eval_number("SET_COUNT"),
                    1.0,
                    "shouldn't reapply until wake"
                );

                harness.advance(Duration::from_millis(1100)).await;
                assert_eq!(harness.eval_number("SET_COUNT"), 2.0);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn splash_shown_once_and_fades_out() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    splash_tags: vec!["splash".to_string()],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert("splash_enabled".to_string(), OptionValue::Boolean(true));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture_with_tagged_image(&["splash"]),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                assert_eq!(
                    harness.recorded(),
                    vec![Recorded::SpawnImage { media_id: 1 }]
                );

                // Drive the fade-in -> hold -> fade-out -> close chain: the fake handler doesn't
                // simulate FadeFinish itself (a real animation-completion event in the real app),
                // so the test supplies it directly, same as DialogSelect/DialogSubmit elsewhere.
                harness.send_event(Event::FadeFinish {
                    id: PopupId(0),
                    fade_id: 0,
                });
                harness.pump_events();
                harness.advance(Duration::from_millis(1600)).await; // SPLASH_HOLD_MS = 1500
                harness.send_event(Event::FadeFinish {
                    id: PopupId(0),
                    fade_id: 1,
                });
                harness.pump_events();

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

    #[tokio::test(start_paused = true)]
    async fn splash_disabled_never_spawns() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    splash_tags: vec!["splash".to_string()],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert("splash_enabled".to_string(), OptionValue::Boolean(false));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture_with_tagged_image(&["splash"]),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(harness.recorded(), vec![]);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn splash_skips_silently_with_no_splash_tagged_media() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    splash_tags: vec!["splash".to_string()],
                    ..Default::default()
                };

                let mut config = isolated_process_config();
                config.insert("splash_enabled".to_string(), OptionValue::Boolean(true));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(false),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(harness.recorded(), vec![]);
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn open_popup_sets_title_from_matching_caption() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    captions: vec![shared::behaviour::TextItem {
                        text: "Obey.".to_string(),
                        tags: vec!["red".to_string()],
                    }],
                    ..Default::default()
                };

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(true),
                    content,
                    base_default_mode_config(),
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                // Slack beyond the nominal 1000ms popup_frequency: the fake handler's reply is a
                // real OS-thread round trip, which eats into the paused clock's margin (see
                // `interval_keeps_firing_after_a_callback_errors`'s comment for the same reasoning).
                harness.advance(Duration::from_millis(1100)).await;

                assert_eq!(
                    harness.recorded(),
                    vec![
                        Recorded::SpawnImage { media_id: 1 },
                        Recorded::SetTitle {
                            id: PopupId(0),
                            title: Some("Obey.".to_string()),
                        },
                    ]
                );
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn open_popup_skips_caption_silently_when_pool_empty_or_unmatched() {
        LocalSet::new()
            .run_until(async {
                for content in [
                    Content::default(),
                    Content {
                        captions: vec![shared::behaviour::TextItem {
                            text: "Obey.".to_string(),
                            tags: vec!["nonexistent".to_string()],
                        }],
                        ..Default::default()
                    },
                ] {
                    let mut harness = Harness::with_pack(
                        &real_default_mode_sources(),
                        pack_fixture(true),
                        content,
                        base_default_mode_config(),
                        Capabilities::default(),
                        Volume::default(),
                    );
                    harness.run_entrypoint("main.lua").unwrap();
                    harness.advance(Duration::from_millis(1100)).await;

                    assert_eq!(
                        harness.recorded(),
                        vec![Recorded::SpawnImage { media_id: 1 }],
                        "expected no SetTitle when the caption pool is empty or unmatched"
                    );
                }
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn captions_disabled_via_config_leaves_title_unset() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    captions: vec![shared::behaviour::TextItem {
                        text: "Obey.".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };

                let mut config = base_default_mode_config();
                config.insert("captions_enabled".to_string(), OptionValue::Boolean(false));

                let mut harness = Harness::with_pack(
                    &real_default_mode_sources(),
                    pack_fixture(true),
                    content,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(1100)).await;

                assert_eq!(
                    harness.recorded(),
                    vec![Recorded::SpawnImage { media_id: 1 }],
                    "expected no SetTitle when captions are disabled via config"
                );
            })
            .await;
    }

    /// A tagged-only item (no empty-tag fallback in the pool) is eligible exactly when the
    /// spawned media's tags intersect its own -- proving tag-matching actually gates eligibility,
    /// independent of the empty-tag-always-applies case covered by the next test.
    #[tokio::test(start_paused = true)]
    async fn content_picker_matches_media_tags_over_empty_bucket() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    captions: vec![shared::behaviour::TextItem {
                        text: "kinky".to_string(),
                        tags: vec!["kinky".to_string()],
                    }],
                    ..Default::default()
                };

                let sources = content_lib_sources(
                    r#"
                        local content = require("lib.content")
                        for _ = 1, 20 do
                            local caption = content.pick_caption({ "kinky" })
                            assert(caption ~= nil and caption.text == "kinky",
                                "expected the tagged caption to be eligible on a matching tag")
                        end
                        assert(content.pick_caption({ "other" }) == nil,
                            "a non-matching tag should exclude the only pool entry")
                    "#,
                );

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    HashMap::new(),
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn content_picker_falls_back_to_empty_tag_item_when_no_match() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    captions: vec![
                        shared::behaviour::TextItem {
                            text: "untagged".to_string(),
                            tags: vec![],
                        },
                        shared::behaviour::TextItem {
                            text: "kinky".to_string(),
                            tags: vec!["kinky".to_string()],
                        },
                    ],
                    ..Default::default()
                };

                let sources = content_lib_sources(
                    r#"
                        local content = require("lib.content")
                        for _ = 1, 20 do
                            local caption = content.pick_caption({ "other" })
                            assert(caption.text == "untagged", "expected the empty-tag caption to win")
                        end
                    "#,
                );

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    HashMap::new(),
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn content_picker_returns_nil_for_empty_pool() {
        LocalSet::new()
            .run_until(async {
                let sources = content_lib_sources(
                    r#"
                        local content = require("lib.content")
                        assert(content.pick_caption({ "anything" }) == nil)
                        assert(content.pick_prompt() == nil)
                        assert(content.pick_notification() == nil)
                        assert(content.pick_subliminal() == nil)
                        assert(content.pick_web_link() == nil)
                    "#,
                );

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    Content::default(),
                    HashMap::new(),
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn content_picker_excludes_disabled_content_group_tags() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    content_groups: vec![shared::behaviour::ContentGroup {
                        id: "kinky".to_string(),
                        label: "Kinky".to_string(),
                        description: None,
                        tags: vec!["kinky".to_string()],
                        enabled_by_default: true,
                    }],
                    notifications: vec![
                        shared::behaviour::TextItem {
                            text: "vanilla".to_string(),
                            tags: vec![],
                        },
                        shared::behaviour::TextItem {
                            text: "kinky".to_string(),
                            tags: vec!["kinky".to_string()],
                        },
                    ],
                    ..Default::default()
                };

                let sources = content_lib_sources(
                    r#"
                        local content = require("lib.content")
                        for _ = 1, 20 do
                            local notification = content.pick_notification()
                            assert(notification.text == "vanilla",
                                "expected the kinky-tagged notification to be excluded")
                        end
                    "#,
                );

                let mut mode_config = HashMap::new();
                mode_config.insert(
                    format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX),
                    OptionValue::Boolean(false),
                );

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    mode_config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn content_picker_standalone_mode_ignores_own_tags() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    subliminals: vec![shared::behaviour::TextItem {
                        text: "obey".to_string(),
                        tags: vec!["hypno".to_string()],
                    }],
                    ..Default::default()
                };

                let sources = content_lib_sources(
                    r#"
                        local content = require("lib.content")
                        for _ = 1, 20 do
                            local item = content.pick_subliminal()
                            assert(item ~= nil and item.text == "obey",
                                "a standalone pool's own tags shouldn't gate eligibility")
                        end
                    "#,
                );

                let mut harness = Harness::with_pack(
                    &sources,
                    pack_fixture(false),
                    content,
                    HashMap::new(),
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    /// Regression test for the `merge_tags` shorthand-array bug: querying with the shorthand
    /// `tags = { "wallpaper" }` form must still narrow the results even once some *other* content
    /// group is disabled -- before the fix, `merge_tags` silently dropped the shorthand filter in
    /// that case (`tags.any` read off a plain array is always nil), matching all media instead.
    #[tokio::test(start_paused = true)]
    async fn media_query_layer_honors_shorthand_tags_when_a_group_is_disabled() {
        LocalSet::new()
            .run_until(async {
                const MEDIA: &[(&str, &[&str])] = &[
                    ("wallpaper1.avif", &["wallpaper"]),
                    ("kinky1.avif", &["kinky"]),
                ];

                let content = Content {
                    content_groups: vec![shared::behaviour::ContentGroup {
                        id: "kinky".to_string(),
                        label: "Kinky".to_string(),
                        description: None,
                        tags: vec!["kinky".to_string()],
                        enabled_by_default: true,
                    }],
                    ..Default::default()
                };

                let sources: &[(&str, &str)] = &[
                    (
                        "main.lua",
                        r#"
                            local media = require("lib.media")
                            local items = media.list({ type = { "image" }, tags = { "wallpaper" } })
                            assert(#items == 1, "shorthand tag filter should still narrow the results")
                            assert(items[1].name == "wallpaper1.avif")
                        "#,
                    ),
                    (
                        "lib/media.lua",
                        include_str!("../../../default-modes/shared/lib/media.lua"),
                    ),
                ];

                let mut mode_config = HashMap::new();
                mode_config.insert(
                    format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX),
                    OptionValue::Boolean(false),
                );

                let mut harness = Harness::with_pack(
                    sources,
                    pack_fixture_with_data(MEDIA, None),
                    content,
                    mode_config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
            })
            .await;
    }

    // ─── Experience mode ────────────────────────────────────────────────────────

    /// The core Experience arithmetic: `effective_interval = anchor_seconds / pace`, exercised
    /// against the real `default-modes/experience/src/main.lua`. Anchor 2s x pace 2.0 -> 1s
    /// popups, so three should have spawned after 3.1s (mirrors the Sandbox constant-rate spawn
    /// tests' shape).
    #[tokio::test(start_paused = true)]
    async fn experience_popup_spawns_at_anchor_over_pace_interval() {
        LocalSet::new()
            .run_until(async {
                let experience = Experience {
                    anchors: FrequencyAnchors {
                        popup: Some(2.0),
                        ..Default::default()
                    },
                    design: DesignValues::default(),
                    timeline: None,
                };

                let mut config = base_experience_mode_config();
                config.insert("pace".to_string(), OptionValue::Number(2.0));

                let mut harness = Harness::with_pack_and_experience(
                    &real_experience_mode_sources(),
                    pack_fixture(true),
                    Content::default(),
                    experience,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(3100)).await;

                let spawn_count = harness
                    .recorded()
                    .iter()
                    .filter(|r| matches!(r, Recorded::SpawnImage { .. }))
                    .count();
                assert_eq!(spawn_count, 3);
            })
            .await;
    }

    /// An absent `anchors.popup` means the spawn loop never starts at all in Experience -- not
    /// "spawn at some default rate" -- see `FrequencyAnchors`'s doc comment and interaction rule
    /// 5's "skip, don't error" spirit generalized to "doesn't exist here".
    #[tokio::test(start_paused = true)]
    async fn experience_missing_popup_anchor_never_spawns() {
        LocalSet::new()
            .run_until(async {
                let experience = Experience::default(); // no anchors at all

                let harness_config = base_experience_mode_config();

                let mut harness = Harness::with_pack_and_experience(
                    &real_experience_mode_sources(),
                    pack_fixture(true),
                    Content::default(),
                    experience,
                    harness_config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_secs(60)).await;

                assert!(harness.recorded().is_empty());
            })
            .await;
    }

    /// Same anchor-over-pace arithmetic, for a standalone-scheduled feature (notifications)
    /// rather than the spawn loop -- confirms `experience/src/main.lua` threads `anchors.* /
    /// pace` through to `lib/notifications.lua` correctly.
    #[tokio::test(start_paused = true)]
    async fn experience_notification_anchor_scaled_by_pace() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    notifications: vec![shared::behaviour::TextItem {
                        text: "hi".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };
                let experience = Experience {
                    anchors: FrequencyAnchors {
                        notification: Some(2.0),
                        ..Default::default()
                    },
                    design: DesignValues::default(),
                    timeline: None,
                };

                let mut config = isolated_experience_process_config();
                config.insert("pace".to_string(), OptionValue::Number(2.0));

                let owned = wrapped_experience_mode_sources(COUNTING_NOTIFICATION_WRAP);
                let sources = as_str_sources(&owned);

                let mut harness = Harness::with_pack_and_experience(
                    &sources,
                    pack_fixture(false),
                    content,
                    experience,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(3100)).await;

                assert_eq!(harness.eval_number("COUNT"), 3.0);
            })
            .await;
    }

    /// The user's off-switch still wins even when the pack's design anchors the feature --
    /// class-1 ownership applies identically in Experience (`behaviour-design/default-mode.md`,
    /// Ownership: "Available in *both* modes; the timeline can never touch them").
    #[tokio::test(start_paused = true)]
    async fn experience_notifications_off_switch_wins_over_present_anchor() {
        LocalSet::new()
            .run_until(async {
                let content = Content {
                    notifications: vec![shared::behaviour::TextItem {
                        text: "hi".to_string(),
                        tags: vec![],
                    }],
                    ..Default::default()
                };
                let experience = Experience {
                    anchors: FrequencyAnchors {
                        notification: Some(1.0),
                        ..Default::default()
                    },
                    design: DesignValues::default(),
                    timeline: None,
                };

                let mut config = isolated_experience_process_config();
                config.insert(
                    "notifications_enabled".to_string(),
                    OptionValue::Boolean(false),
                );

                let owned = wrapped_experience_mode_sources(COUNTING_NOTIFICATION_WRAP);
                let sources = as_str_sources(&owned);

                let mut harness = Harness::with_pack_and_experience(
                    &sources,
                    pack_fixture(false),
                    content,
                    experience,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_secs(10)).await;

                assert_eq!(harness.eval_number("COUNT"), 0.0);
            })
            .await;
    }

    // ─── Transitions (timeline) ─────────────────────────────────────────────────

    /// Minimal sources for exercising `experience/src/timeline.lua` in isolation, uncoupled from
    /// the spawn loop's own timing -- most of the timeline's own state-machine behaviour (trigger
    /// evaluation, absolute-snapshot params) doesn't need a real pack or media at all.
    fn timeline_only_sources() -> Vec<(&'static str, &'static str)> {
        vec![
            ("main.lua", "require('timeline')"),
            (
                "timeline.lua",
                include_str!("../../../default-modes/experience/src/timeline.lua"),
            ),
        ]
    }

    /// Same as `timeline_only_sources`, but also calls `M.init()` -- needed by any test relying on
    /// a level's `at_seconds` timer actually firing (as opposed to only `on_popup_spawned()`-driven
    /// advances).
    fn timeline_only_sources_initialized() -> Vec<(&'static str, &'static str)> {
        vec![
            ("main.lua", "require('timeline').init()"),
            (
                "timeline.lua",
                include_str!("../../../default-modes/experience/src/timeline.lua"),
            ),
        ]
    }

    fn timeline_harness(sources: &[(&str, &str)], experience: Experience) -> Harness {
        Harness::with_pack_and_experience(
            sources,
            pack_fixture(false),
            Content::default(),
            experience,
            HashMap::new(),
            Capabilities::default(),
            Volume::default(),
        )
    }

    /// A no-`timeline` pack (anchors/design only, exactly what M4's earlier "Experience mode"
    /// bullet already shipped) must leave every getter at the level-0 baseline forever: `nil`
    /// timeline is not a degraded case, it's the common one (most Experience packs won't design an
    /// arc) -- see `Timeline`'s doc comment.
    #[tokio::test(start_paused = true)]
    async fn experience_timeline_absent_stays_at_baseline_forever() {
        LocalSet::new()
            .run_until(async {
                let experience = Experience {
                    anchors: FrequencyAnchors::default(),
                    design: DesignValues::default(),
                    timeline: None,
                };
                let mut harness =
                    timeline_harness(&timeline_only_sources_initialized(), experience);
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_secs(3600)).await;

                assert_eq!(harness.eval_number("require('timeline').modifier()"), 1.0);
                assert_eq!(
                    harness.eval_string("table.concat(require('timeline').tags() or {}, '|')"),
                    ""
                );
                assert_eq!(
                    harness.eval_string(
                        "table.concat(require('timeline').wallpaper_tags() or {}, '|')"
                    ),
                    ""
                );
            })
            .await;
    }

    /// The core trigger + snapshot mechanics: before a level's `at_seconds`, every getter stays at
    /// baseline; once active-time elapses past it, `modifier()`/`tags()` jump to that level's
    /// absolute values in one step (interaction rules 1-3).
    #[tokio::test(start_paused = true)]
    async fn experience_timeline_advances_on_active_time_and_applies_modifier() {
        LocalSet::new()
            .run_until(async {
                let experience = Experience {
                    anchors: FrequencyAnchors::default(),
                    design: DesignValues::default(),
                    timeline: Some(Timeline {
                        levels: vec![Level {
                            at_seconds: 5.0,
                            at_popups: None,
                            modifiers: Modifiers {
                                modifier: Some(4.0),
                                tags: Some(vec!["kinky".to_string()]),
                                wallpaper_tags: None,
                            },
                        }],
                    }),
                };
                let mut harness =
                    timeline_harness(&timeline_only_sources_initialized(), experience);
                harness.run_entrypoint("main.lua").unwrap();

                harness.advance(Duration::from_millis(4900)).await;
                assert_eq!(
                    harness.eval_number("require('timeline').modifier()"),
                    1.0,
                    "still baseline just before at_seconds"
                );

                harness.advance(Duration::from_millis(300)).await; // crosses t=5.0s
                assert_eq!(harness.eval_number("require('timeline').modifier()"), 4.0);
                assert_eq!(
                    harness.eval_string("table.concat(require('timeline').tags() or {}, '|')"),
                    "kinky"
                );
            })
            .await;
    }

    /// A popup-count trigger reached well before its level's `at_seconds` advances immediately --
    /// no waiting for the time timer, and no stepping through intermediate levels (there are none
    /// here to step through, but see the order-independence test below for that half).
    #[tokio::test(start_paused = true)]
    async fn experience_timeline_popup_count_trigger_advances_before_time_elapses() {
        LocalSet::new()
            .run_until(async {
                let experience = Experience {
                    anchors: FrequencyAnchors::default(),
                    design: DesignValues::default(),
                    timeline: Some(Timeline {
                        levels: vec![Level {
                            at_seconds: 1_000.0, // never reached within this test
                            at_popups: Some(3),
                            modifiers: Modifiers {
                                modifier: Some(2.0),
                                ..Default::default()
                            },
                        }],
                    }),
                };
                // No `.init()` -- this test isolates popup-count-driven advancement from any
                // `at_seconds` timer.
                let mut harness = timeline_harness(&timeline_only_sources(), experience);
                harness.run_entrypoint("main.lua").unwrap();

                assert_eq!(
                    harness.eval_number(
                        "(function() \
                            local t = require('timeline') \
                            t.on_popup_spawned() \
                            t.on_popup_spawned() \
                            return t.modifier() \
                        end)()"
                    ),
                    1.0,
                    "two popups: threshold of three not yet reached"
                );

                assert_eq!(
                    harness.eval_number(
                        "(function() \
                            local t = require('timeline') \
                            t.on_popup_spawned() \
                            return t.modifier() \
                        end)()"
                    ),
                    2.0,
                    "third popup crosses the threshold"
                );
            })
            .await;
    }

    /// Rule 6, made structural: a level with an `at_popups` threshold still advances via
    /// `at_seconds` even when nothing ever calls `on_popup_spawned()` (the real-world case: the
    /// user disabled every popup-spawning media type). The design can't stall.
    #[tokio::test(start_paused = true)]
    async fn experience_timeline_rule6_time_fallback_when_popups_never_spawn() {
        LocalSet::new()
            .run_until(async {
                let experience = Experience {
                    anchors: FrequencyAnchors::default(),
                    design: DesignValues::default(),
                    timeline: Some(Timeline {
                        levels: vec![Level {
                            at_seconds: 2.0,
                            at_popups: Some(1_000_000), // never reached -- popups never spawn
                            modifiers: Modifiers {
                                modifier: Some(9.0),
                                ..Default::default()
                            },
                        }],
                    }),
                };
                let mut harness =
                    timeline_harness(&timeline_only_sources_initialized(), experience);
                harness.run_entrypoint("main.lua").unwrap();

                harness.advance(Duration::from_millis(2100)).await;
                assert_eq!(harness.eval_number("require('timeline').modifier()"), 9.0);
            })
            .await;
    }

    /// Levels are absolute snapshots relative to baseline, not deltas relative to the previous
    /// level: jumping straight from level 0 to level 2 (skipping level 1's own threshold) produces
    /// identical params to passing through level 1 first -- see `Modifiers`'s doc comment.
    #[tokio::test(start_paused = true)]
    async fn experience_timeline_order_independence_jumping_directly_matches_sequential() {
        LocalSet::new()
            .run_until(async {
                fn two_level_experience() -> Experience {
                    Experience {
                        anchors: FrequencyAnchors::default(),
                        design: DesignValues::default(),
                        timeline: Some(Timeline {
                            levels: vec![
                                Level {
                                    at_seconds: 1_000.0,
                                    at_popups: Some(2),
                                    modifiers: Modifiers {
                                        modifier: Some(2.0),
                                        tags: Some(vec!["a".to_string()]),
                                        ..Default::default()
                                    },
                                },
                                Level {
                                    at_seconds: 1_000.0,
                                    at_popups: Some(5),
                                    modifiers: Modifiers {
                                        modifier: Some(5.0),
                                        tags: Some(vec!["b".to_string()]),
                                        ..Default::default()
                                    },
                                },
                            ],
                        }),
                    }
                }

                let probe = "(function() \
                    local t = require('timeline') \
                    return t.modifier() * 1000 + #(t.tags() or {}) \
                end)()";

                // Sequential: reach level 1 (2 popups), observe it, then continue to level 2.
                let mut sequential = timeline_harness(&timeline_only_sources(), two_level_experience());
                sequential.run_entrypoint("main.lua").unwrap();
                sequential.eval_number(
                    "(function() local t = require('timeline') t.on_popup_spawned() t.on_popup_spawned() return 1 end)()",
                );
                assert_eq!(sequential.eval_number(probe), 2000.0 + 1.0, "level 1 reached");
                sequential.eval_number(
                    "(function() local t = require('timeline') for i=1,3 do t.on_popup_spawned() end return 1 end)()",
                );
                let sequential_final = sequential.eval_number(probe);

                // Direct jump: five popups in one batch, never separately observing level 1.
                let mut direct = timeline_harness(&timeline_only_sources(), two_level_experience());
                direct.run_entrypoint("main.lua").unwrap();
                let direct_final = direct.eval_number(
                    &format!(
                        "(function() \
                            local t = require('timeline') \
                            for i=1,5 do t.on_popup_spawned() end \
                            return {probe} \
                        end)()"
                    ),
                );

                assert_eq!(sequential_final, 5000.0 + 1.0, "level 2's own absolute params");
                assert_eq!(
                    direct_final, sequential_final,
                    "jumping straight to level 2 must match passing through level 1 first"
                );
            })
            .await;
    }

    /// Wallpaper is the one mode parameter that needs *push* semantics (rule 3): it must reapply
    /// when a level's effective override actually changes, but never redundantly for an unchanged
    /// value -- otherwise every level transition would re-randomize/flicker the wallpaper even when
    /// that level doesn't touch it.
    #[tokio::test(start_paused = true)]
    async fn experience_timeline_wallpaper_override_reapplies_only_when_changed() {
        LocalSet::new()
            .run_until(async {
                fn level_at(at_seconds: f64, wallpaper_tags: Option<Vec<String>>) -> Level {
                    Level {
                        at_seconds,
                        at_popups: None,
                        modifiers: Modifiers {
                            wallpaper_tags,
                            ..Default::default()
                        },
                    }
                }

                let experience = Experience {
                    anchors: FrequencyAnchors::default(),
                    design: DesignValues::default(),
                    timeline: Some(Timeline {
                        levels: vec![
                            level_at(1.0, Some(vec!["special".to_string()])), // baseline -> special: applies
                            level_at(2.0, None), // special -> baseline (empty): no-ops internally
                            level_at(3.0, Some(vec!["special".to_string()])), // baseline -> special again: applies
                            level_at(4.0, Some(vec!["special".to_string()])), // unchanged: must not reapply
                        ],
                    }),
                };

                let owned = wrapped_experience_mode_sources(WALLPAPER_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut config = base_experience_mode_config();
                config.insert("images_enabled".to_string(), OptionValue::Boolean(false));

                let mut harness = Harness::with_pack_and_experience(
                    &sources,
                    pack_fixture_with_tagged_image(&["special"]),
                    Content::default(),
                    experience,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(4100)).await;

                assert_eq!(
                    harness.eval_number("SET_COUNT"),
                    2.0,
                    "two genuine special-tag applications; the no-op-to-empty and repeat-same-value \
                     transitions must not add to this"
                );
            })
            .await;
    }

    /// `max_popups` (a class-1 user cap) stays authoritative even when a level's modifier cranks
    /// the effective spawn rate far above what the pack's anchor alone would produce -- caps are
    /// "clamps applied after everything else" (`behaviour-design/default-mode.md`, Ownership),
    /// never something a design can spawn its way past.
    #[tokio::test(start_paused = true)]
    async fn experience_timeline_max_popups_still_clamps_when_modifier_raises_rate() {
        LocalSet::new()
            .run_until(async {
                let experience = Experience {
                    anchors: FrequencyAnchors {
                        popup: Some(0.1),
                        ..Default::default()
                    },
                    design: DesignValues::default(),
                    timeline: Some(Timeline {
                        levels: vec![Level {
                            at_seconds: 0.05,
                            at_popups: None,
                            modifiers: Modifiers {
                                modifier: Some(100.0),
                                ..Default::default()
                            },
                        }],
                    }),
                };

                let mut config = base_experience_mode_config();
                config.insert("max_popups".to_string(), OptionValue::Integer(2));

                let mut harness = Harness::with_pack_and_experience(
                    &real_experience_mode_sources(),
                    pack_fixture(true),
                    Content::default(),
                    experience,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_secs(2)).await;

                let spawn_count = harness
                    .recorded()
                    .iter()
                    .filter(|r| matches!(r, Recorded::SpawnImage { .. }))
                    .count();
                assert_eq!(
                    spawn_count, 2,
                    "the cap, not the (much faster) modulated rate, wins"
                );
            })
            .await;
    }

    /// Records, at the moment each spawn decision is made, whether it violated the timeline's
    /// *currently active* tag restriction -- rather than comparing spawn counts across a
    /// before/after time window (which races against exactly which spawn decisions happen to fall
    /// in the gap around the level's boundary instant, per rule 2). Checking the invariant at each
    /// decision point directly is deterministic regardless of that timing.
    const SPAWN_NAME_CAPTURE_WRAP: &str = r#"
        KINKY_COUNT = 0
        VANILLA_WHILE_RESTRICTED = 0
        local real = lewdware.popup.image
        lewdware.popup.image = function(item, opts)
            local tags = require("timeline").tags()
            if item.name == "kinky.avif" then KINKY_COUNT = KINKY_COUNT + 1 end
            if item.name == "vanilla.avif" and tags and tags[1] == "kinky" then
                VANILLA_WHILE_RESTRICTED = VANILLA_WHILE_RESTRICTED + 1
            end
            return real(item, opts)
        end
    "#;

    /// A level's `tags` restricts which media can spawn (the active tag set, an `any`-style
    /// restriction -- see `Modifiers::tags`'s doc comment): once the timeline's active tag set is
    /// `{"kinky"}`, `vanilla.avif` (tagged only `"vanilla"`) must never spawn -- checked
    /// deterministically at every spawn decision (the engine's tag filter is a SQL
    /// `EXISTS`/`NOT EXISTS` exclusion, not a random sample), not via a racy spawn-count window.
    #[tokio::test(start_paused = true)]
    async fn experience_timeline_tags_narrow_which_media_can_spawn() {
        LocalSet::new()
            .run_until(async {
                const MEDIA: &[(&str, &[&str])] =
                    &[("kinky.avif", &["kinky"]), ("vanilla.avif", &["vanilla"])];

                let experience = Experience {
                    anchors: FrequencyAnchors {
                        popup: Some(0.05),
                        ..Default::default()
                    },
                    design: DesignValues::default(),
                    timeline: Some(Timeline {
                        levels: vec![Level {
                            at_seconds: 0.5,
                            at_popups: None,
                            modifiers: Modifiers {
                                tags: Some(vec!["kinky".to_string()]),
                                ..Default::default()
                            },
                        }],
                    }),
                };

                let mut config = base_experience_mode_config();
                config.insert("max_popups".to_string(), OptionValue::Integer(1_000));
                config.insert("captions_enabled".to_string(), OptionValue::Boolean(false));

                let owned = wrapped_experience_mode_sources(SPAWN_NAME_CAPTURE_WRAP);
                let sources = as_str_sources(&owned);

                let mut harness = Harness::with_pack_and_experience(
                    &sources,
                    pack_fixture_with_data(MEDIA, None),
                    Content::default(),
                    experience,
                    config,
                    Capabilities::default(),
                    Volume::default(),
                );
                harness.run_entrypoint("main.lua").unwrap();
                harness.advance(Duration::from_millis(2900)).await;

                assert_eq!(
                    harness.eval_number("VANILLA_WHILE_RESTRICTED"),
                    0.0,
                    "vanilla.avif must never spawn while the timeline's active tag set is \
                     restricted to kinky"
                );
                assert!(
                    harness.eval_number("KINKY_COUNT") > 10.0,
                    "kinky spawns should still be happening at the full baseline rate"
                );
            })
            .await;
    }

    /// `Mode::Experience`'s stored options are scoped per pack (`AppConfig::experience_options`),
    /// unlike every other mode's global `mode_options` -- see `behaviour-design/default-mode.md`,
    /// Ownership. A stored value under a *different* pack's UUID must never leak into this one's
    /// resolved config.
    #[test]
    fn resolve_mode_config_experience_mode_reads_scoped_experience_options() {
        let pack_file = pack_fixture_with_data(&[], None);

        let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);
        let (media_manager, _metadata, pack_id, _handle) =
            MediaManager::open(pack_file.path(), event_poster, None).unwrap();

        let mut entries = indexmap::IndexMap::new();
        entries.insert(
            "pace".to_string(),
            shared::mode::ModeEntry::Option(shared::mode::ModeOption {
                label: "Pacing".to_string(),
                description: None,
                option_type: shared::mode::OptionType::Number {
                    default: 1.0,
                    min: None,
                    max: None,
                    step: None,
                    clamp: false,
                    slider: false,
                },
                optional: false,
                enabled_by_default: false,
                show_when: None,
            }),
        );
        let metadata = Metadata {
            name: "Experience".to_string(),
            version: None,
            author: None,
            entrypoint: "main.lua".to_string(),
            entries,
            files: HashMap::new(),
        };

        let mut this_pack_options = HashMap::new();
        this_pack_options.insert("pace".to_string(), OptionValue::Number(2.0));
        let mut other_pack_options = HashMap::new();
        other_pack_options.insert("pace".to_string(), OptionValue::Number(9.0));

        let mut experience_options = HashMap::new();
        experience_options.insert(pack_id, this_pack_options);
        experience_options.insert(Uuid::new_v4(), other_pack_options);

        let (config, _content, _experience) = resolve_mode_config(
            &metadata,
            &shared::user_config::Mode::Experience,
            &HashMap::new(),
            &experience_options,
            pack_id,
            &media_manager,
            false,
        );

        assert_eq!(config.get("pace"), Some(&OptionValue::Number(2.0)));
    }

    /// Experience is the pack-behaviour-consuming default mode too -- content-group toggles
    /// synthesize identically to Sandbox's (mirrors
    /// `resolve_mode_config_synthesizes_content_group_toggle_for_sandbox_mode`).
    #[test]
    fn resolve_mode_config_experience_mode_synthesizes_content_group_toggle() {
        let behaviour = behaviour_with_one_content_group();
        let behaviour_bytes = behaviour.to_json_bytes().unwrap();
        let pack_file = pack_fixture_with_data(&[], Some(&behaviour_bytes));

        let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);
        let (media_manager, _metadata, pack_id, _handle) =
            MediaManager::open(pack_file.path(), event_poster, None).unwrap();

        let metadata = empty_default_mode_metadata();
        let key = format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX);

        let (config, content, _experience) = resolve_mode_config(
            &metadata,
            &shared::user_config::Mode::Experience,
            &HashMap::new(),
            &HashMap::new(),
            pack_id,
            &media_manager,
            false,
        );
        assert_eq!(config.get(&key), Some(&OptionValue::Boolean(true)));
        assert_eq!(content.content_groups.len(), 1);
    }
}
