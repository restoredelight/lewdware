mod api;
mod audio;
mod interval;
mod media;
mod mode;
mod request;
mod window;

use std::{cell::RefCell, collections::HashMap, fs::File, io::Cursor, rc::Rc, sync::Arc, thread};

use anyhow::bail;
use mlua::{ExternalResult, Lua, StdLib};
use shared::{
    mode::{Metadata, OptionValue, VERSION_MAJOR, read_mode_metadata},
    user_config::AppConfig,
};
use tokio::{
    sync::{mpsc::UnboundedSender, oneshot},
    task::LocalSet,
};

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
    DialogClick {
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

/// Starts the Lua thread using the given `media_manager` — already opened by the caller (see
/// `LewdwareApp::new` in `app.rs`), which also keeps its own clone to resolve media for popups
/// asynchronously (see `App::spawn_image`/`spawn_video`/`spawn_audio`).
pub fn start_lua_thread(
    event_poster: EventPoster,
    config: Arc<AppConfig>,
    media_manager: MediaManager,
    gpu_available: bool,
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

        let (mut file, mode): (Box<dyn ReadSeek>, _) = match config.mode.clone() {
            shared::user_config::Mode::Default(default_mode) => {
                let mode_data = include_bytes!("../../../default-modes/build/Default Modes.lwmode");

                (Box::new(Cursor::new(mode_data)), default_mode)
            }
            shared::user_config::Mode::Pack { id, mode } => {
                let mode_data = match media_manager.get_mode(id) {
                    Ok(data) => data,
                    Err(err) => {
                        report_fatal_startup_error(err);
                        return;
                    }
                };

                (Box::new(Cursor::new(mode_data)), mode)
            }
            shared::user_config::Mode::File { path, mode } => {
                let file = match File::open(path) {
                    Ok(file) => file,
                    Err(err) => {
                        report_fatal_startup_error(err);
                        return;
                    }
                };

                (Box::new(file), mode)
            }
        };

        let (header, Metadata { modes, files, .. }) = match read_mode_metadata(&mut file) {
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

        let mode_obj = match modes.get(&mode) {
            Some(mode_obj) => mode_obj,
            None => {
                report_fatal_startup_error(format!(
                    "Mode '{mode}' was not found in this mode file \
                     (the configuration may be stale)"
                ));
                return;
            }
        };

        let entrypoint = mode_obj.entrypoint.clone();

        let mut mode_config = config
            .mode_options
            .get(&config.mode)
            .cloned()
            .unwrap_or_default();

        // Make sure the config contains all the correct options
        for (key, option) in mode_obj.all_options() {
            if mode_config
                .get(key)
                .is_none_or(|value| !option.matches_value(value))
            {
                mode_config.insert(key.to_string(), option.default_value());
            }
        }

        let mode = Mode::new(file, files);

        let mut local = LocalSet::new();

        let runtime = match LuaRuntime::new(
            mode,
            RequestSender::new(request_tx, event_poster),
            media_manager,
            mode_config,
            gpu_available,
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
    lua: Lua,
}

impl LuaRuntime {
    fn new(
        mode: Mode,
        request_tx: RequestSender,
        media_manager: MediaManager,
        config: HashMap<String, OptionValue>,
        gpu_available: bool,
    ) -> anyhow::Result<Self> {
        let lua = create_sandboxed_lua()?;

        let mut runtime = Self {
            mode: Rc::new(mode),
            request_sender: request_tx,
            media_manager,
            windows: Rc::new(RefCell::new(HashMap::new())),
            audio_handles: Rc::new(RefCell::new(HashMap::new())),
            lua,
        };

        runtime.create_api(config, gpu_available)?;

        Ok(runtime)
    }

    fn run_entrypoint(&self, entrypoint: String) -> mlua::Result<()> {
        self.mode.load(&self.lua, entrypoint).into_lua_err()?.eval()
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
            Event::DialogClick {
                id,
                button_id,
                values,
            } => {
                if let Some(window) = self.windows.try_borrow()?.get(&id).cloned() {
                    match window {
                        Window::Dialog(dialog) => {
                            dialog.on_click(button_id, values)?;
                        }
                        _ => bail!("Dialog click event for a non-dialog window"),
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

    fn create_api(
        &mut self,
        config: HashMap<String, OptionValue>,
        gpu_available: bool,
    ) -> mlua::Result<()> {
        create_api(
            &self.lua,
            self.request_sender.clone(),
            self.media_manager.clone(),
            self.windows.clone(),
            self.audio_handles.clone(),
            config,
            gpu_available,
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
    }

    impl Harness {
        fn new(sources: &[(&str, &str)], with_image: bool) -> Self {
            let pack_file = pack_fixture(with_image);
            let event_poster: EventPoster = Arc::new(|_event: UserEvent| true);

            let (media_manager, _metadata, _media_manager_handle) =
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

            let mode = make_mode(sources);
            let runtime = Rc::new(
                LuaRuntime::new(mode, request_sender, media_manager, HashMap::new(), false)
                    .unwrap(),
            );

            Self {
                runtime,
                event_rx,
                event_tx: event_tx_clone,
                recorded,
                _pack_file: pack_file,
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
    async fn dead_window_semantics() {
        LocalSet::new()
            .run_until(async {
                let mut harness = Harness::new(
                    &[(
                        "main.lua",
                        r#"
                            local image = lewdware.media.get_image("pic.avif")
                            local window = lewdware.popup.image(image)
                            window:close()
                            -- Closing twice is documented as a safe no-op.
                            window:close()

                            local ok = pcall(function()
                                window:set_opacity(0.5)
                            end)
                            assert(not ok, "acting on a closed window should currently error")
                        "#,
                    )],
                    true,
                );

                harness.run_entrypoint("main.lua").unwrap();
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
    /// existed), and `on_click`/`on_submit` (fired here via manually injected events, since the
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
                                DIALOG:on_click(function(button_id, values)
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
                                assert(#CLICKS == 1, "on_click should have fired once")
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

                harness.send_event(Event::DialogClick {
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
