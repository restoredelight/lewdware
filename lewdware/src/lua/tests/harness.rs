use std::{
    collections::HashSet,
    io::{Cursor, Write as _},
    sync::{Arc, Mutex},
    time::Duration,
};

use shared::behaviour::Behaviour;
use shared::user_config::{Capabilities, Volume};
use tempfile::NamedTempFile;
use tokio::sync::mpsc::UnboundedSender;

use super::*;
use crate::{app::UserEvent, monitor::Monitor};

impl<F: Fn(UserEvent) -> bool + Send + Sync + 'static> EventPoster for Arc<F> {
    fn post_event(&self, event: UserEvent) -> bool {
        self(event)
    }
}

#[derive(Clone)]
pub(super) struct EmptyPoster();
impl EventPoster for EmptyPoster {
    fn post_event(&self, _event: UserEvent) -> bool {
        true
    }
}

/// Build a minimal but valid, media-less-by-default `.lwpack` fixture on disk, purely so
/// `MediaManager::open` has something real to read — mirrors the on-disk layout built (and
/// already exercised) in `media/pack.rs`'s `open_reads_deserialized_index` test. With
/// `with_image`, `pic.avif`'s row also gets a real `offset`/`length` pointing at dummy bytes
/// appended to the file (content doesn't matter — `get_image_file` is a raw byte copy, never
/// decoded here), so `get_image_file`/`wallpaper.set` have real data to read rather than
/// erroring on a null offset.
/// The id of the single media row every fixture inserts first (`pic.avif`, `clip.webm`, or
/// the first entry handed to `pack_fixture_with_data`). Behaviour media slots address media by
/// id, so a fixture that wants a slot filled names this.
pub(super) const FIXTURE_MEDIA: u64 = 1;

pub(super) fn pack_fixture(with_image: bool) -> NamedTempFile {
    const IMAGE_BYTES: &[u8] = b"not a real avif, just needs to be some bytes";

    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    shared::db::migrate(&mut db).unwrap();

    if with_image {
        db.execute(
            "INSERT INTO media (file_name, file_type, offset, length, width, height, transparent, hash) \
             VALUES ('pic.avif', 'image', 0, 0, 64, 64, 0, x'00')",
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

    let metadata = shared::pack::Metadata {
        name: "test-pack".to_string(),
        ..Default::default()
    };
    let metadata_bytes = metadata.to_buf().unwrap();

    let mut header = shared::pack::Header::new();
    header.metadata_offset = shared::pack::HEADER_SIZE as u64;
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

/// `pack_fixture(true)`'s counterpart for a *video*: one real-bytes `clip.webm`, optionally
/// tagged. Needed because the media type a pack ends up storing is not the one the author
/// supplied: an animated GIF -- Edgeware's usual `loading_splash` -- probes as
/// `FileInfo::Video` (see `shared/src/encode.rs`), so a video in the splash slot is the
/// ordinary case, not an exotic one.
pub(super) fn pack_fixture_with_tagged_video(tags: &[&str]) -> NamedTempFile {
    const VIDEO_BYTES: &[u8] = b"not a real webm, just needs to be some bytes";

    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    shared::db::migrate(&mut db).unwrap();

    db.execute(
        "INSERT INTO media (file_name, file_type, offset, length, width, height, transparent, duration, audio, hash) \
         VALUES ('clip.webm', 'video', 0, 0, 64, 64, 0, 1.5, 0, x'00')",
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

    let metadata = shared::pack::Metadata {
        name: "test-pack".to_string(),
        ..Default::default()
    };
    let metadata_bytes = metadata.to_buf().unwrap();

    let mut header = shared::pack::Header::new();
    header.metadata_offset = shared::pack::HEADER_SIZE as u64;
    header.metadata_length = metadata_bytes.len() as u64;
    header.index_offset = header.metadata_offset + header.metadata_length;

    let db_bytes = db.serialize(rusqlite::MAIN_DB).unwrap();
    header.index_length = db_bytes.len() as u64;
    let video_offset = header.index_offset + header.index_length;

    db.execute(
        "UPDATE media SET offset = ?, length = ? WHERE file_name = 'clip.webm'",
        rusqlite::params![video_offset, VIDEO_BYTES.len() as u64],
    )
    .unwrap();
    let db_bytes = db.serialize(rusqlite::MAIN_DB).unwrap();

    let mut file = NamedTempFile::new().unwrap();
    file.write_all(&header.to_buf().unwrap()).unwrap();
    file.write_all(&metadata_bytes).unwrap();
    file.write_all(&db_bytes).unwrap();
    file.write_all(VIDEO_BYTES).unwrap();
    file.flush().unwrap();
    file
}

/// Build a `.lwpack` fixture with the given tagged image rows (name, tags) and, if `Some`, a
/// `pack_data` row named `"behaviour"` holding the given bytes. Media rows use empty archive
/// ranges because `list`/`random` queries never read their bytes (see `MediaPack::parse_media`).
pub(super) fn pack_fixture_with_data(
    media: &[(&str, &[&str])],
    behaviour: Option<&Behaviour>,
) -> NamedTempFile {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    shared::db::migrate(&mut db).unwrap();

    let mut tag_ids: HashMap<&str, i64> = HashMap::new();
    for (index, (file_name, tags)) in media.iter().enumerate() {
        let hash = vec![index as u8];
        let file_type = if file_name.ends_with(".ogg") {
            "audio"
        } else {
            "image"
        };
        db.execute(
            "INSERT INTO media (file_name, file_type, offset, length, width, height, duration, transparent, hash) \
             VALUES (?, ?, 0, 0, 64, 64, 1.0, 0, ?)",
            rusqlite::params![file_name, file_type, hash],
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

    if let Some(behaviour) = behaviour {
        let tx = db.transaction().unwrap();
        shared::behaviour::storage::write(&tx, behaviour).unwrap();
        tx.commit().unwrap();
    }

    let metadata = shared::pack::Metadata {
        name: "test-pack".to_string(),
        ..Default::default()
    };
    let metadata_bytes = metadata.to_buf().unwrap();

    let mut header = shared::pack::Header::new();
    header.metadata_offset = shared::pack::HEADER_SIZE as u64;
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
pub(super) fn make_mode(sources: &[(&str, &str)]) -> Mode {
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
pub(super) enum Recorded {
    SpawnImage {
        media_id: u64,
    },
    SpawnVideo {
        media_id: u64,
        volume: f32,
    },
    SpawnAudio {
        media_id: u64,
        volume: f32,
    },
    SetAudioVolume {
        id: ItemId,
        volume: f32,
    },
    FadeAudioVolume {
        id: ItemId,
        volume: f32,
        duration: u64,
    },
    FadeVideoVolume {
        id: ItemId,
        volume: f32,
        duration: u64,
    },
    SpawnDialog,
    SpawnText,
    CloseWindow {
        id: ItemId,
    },
    OpenLink {
        url: String,
    },
    SetTitle {
        id: ItemId,
        title: Option<String>,
    },
    Exit,
}

pub(super) fn fake_monitor() -> Monitor {
    Monitor {
        id: 0,
        primary: true,
        width: 1920,
        height: 1080,
        scale_factor: 1.0,
    }
}

pub(super) fn fake_window_props(id: ItemId) -> WindowProps {
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
pub(super) fn run_fake_handler(
    request_rx: std::sync::mpsc::Receiver<LuaRequest>,
    recorded: Arc<Mutex<Vec<Recorded>>>,
    event_tx: UnboundedSender<Event>,
    capabilities: Capabilities,
    master_volume: Volume,
) {
    let mut closed_windows = HashSet::new();
    // Stands in for the real `DialogWindow`'s input-element state (see `window_type.rs`),
    // just enough for `values`/`value`/`update` to round-trip in tests.
    let mut dialog_values: HashMap<ItemId, HashMap<String, String>> = HashMap::new();

    while let Ok(request) = request_rx.recv() {
        match request {
            LuaRequest::SpawnImage {
                id, media_id, tx, ..
            } => {
                recorded
                    .lock()
                    .unwrap()
                    .push(Recorded::SpawnImage { media_id });
                let _ = tx.send(Ok(fake_window_props(id)));
            }
            LuaRequest::SpawnVideo {
                id,
                media_id,
                volume,
                tx,
                ..
            } => {
                // Mirrors `LewdwareApp::spawn_video`: master volume is applied where the raw
                // value first arrives, so what's recorded is the effective volume.
                let volume = volume * master_volume.video;
                recorded
                    .lock()
                    .unwrap()
                    .push(Recorded::SpawnVideo { media_id, volume });
                let _ = tx.send(Ok(fake_window_props(id)));
            }
            LuaRequest::SpawnDialog {
                id, elements, tx, ..
            } => {
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
            LuaRequest::SpawnText { id, tx, .. } => {
                recorded.lock().unwrap().push(Recorded::SpawnText);
                let _ = tx.send(Ok(fake_window_props(id)));
            }
            LuaRequest::SpawnAudio {
                id,
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
                let _ = tx.send(Ok(id));
            }
            LuaRequest::SetWallpaper { tx, .. } => {
                let _ = tx.send(Ok(capabilities.set_wallpaper));
            }
            LuaRequest::ResetWallpaper { tx } => {
                let _ = tx.send(());
            }
            LuaRequest::OpenLink { url, tx } => {
                // Mirrors `LewdwareApp::open_link`: a disabled capability is checked before
                // anything else happens, so a denied request leaves no trace to record.
                if capabilities.open_links {
                    recorded.lock().unwrap().push(Recorded::OpenLink { url });
                }
                let _ = tx.send(Ok(capabilities.open_links));
            }
            LuaRequest::ShowNotification { tx, .. } => {
                let _ = tx.send(Ok(capabilities.send_notifications));
            }
            LuaRequest::ListMonitors { tx } => {
                // Non-empty: popup spawns that don't specify a monitor now pick a random one
                // from this list on the Lua thread (see `resolve_monitor` in `lua/api/geometry.rs`).
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
            LuaRequest::ItemAction {
                id,
                action: ItemAction::Window(action),
            } => {
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
                    WindowAction::FadeVideoVolume {
                        tx,
                        id: fade_id,
                        opts,
                    } => {
                        if let Some(opts) = opts {
                            recorded.lock().unwrap().push(Recorded::FadeVideoVolume {
                                id,
                                volume: opts.volume,
                                duration: opts.duration,
                            });
                        }
                        let _ = tx.send(Ok(()));
                        if opts.is_some() {
                            let _ = event_tx.send(Event::VolumeFadeFinish { id, fade_id });
                        }
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
            LuaRequest::ItemAction {
                id,
                action: ItemAction::Audio(action),
            } => match action {
                AudioAction::Pause { tx } => {
                    let _ = tx.send(());
                }
                AudioAction::Play { tx } => {
                    let _ = tx.send(());
                }
                AudioAction::SetVolume { tx, volume } => {
                    recorded
                        .lock()
                        .unwrap()
                        .push(Recorded::SetAudioVolume { id, volume });
                    let _ = tx.send(());
                }
                AudioAction::FadeVolume {
                    tx,
                    id: fade_id,
                    opts,
                } => {
                    if let Some(opts) = opts {
                        recorded.lock().unwrap().push(Recorded::FadeAudioVolume {
                            id,
                            volume: opts.volume,
                            duration: opts.duration,
                        });
                        let _ = event_tx.send(Event::VolumeFadeFinish { id, fade_id });
                    }
                    let _ = tx.send(());
                }
                AudioAction::Stop { tx } => {
                    let _ = tx.send(());
                    let _ = event_tx.send(Event::AudioFinish { id });
                }
            },
        }
    }
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
pub(super) struct Harness {
    pub(super) runtime: Rc<LuaRuntime<EmptyPoster>>,
    pub(super) event_rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
    /// Lets a test simulate an `Event` the real main thread would send but that the fake
    /// handler doesn't model — e.g. `WindowSpawned`, which (unlike `WindowClosed`) is never
    /// the reply to a `LuaRequest`: in the real app it fires from `WindowState::show()` once
    /// a popup's media finishes decoding, which this harness has no equivalent of.
    pub(super) event_tx: UnboundedSender<Event>,
    pub(super) recorded: Arc<Mutex<Vec<Recorded>>>,
    pub(super) _pack_file: NamedTempFile,
    pub(super) _storage_dir: tempfile::TempDir,
}

impl Harness {
    /// Like `with_pack`, but also injects a custom `Experience` section (anchors/design
    /// values) as `__lewdware_experience` -- for exercising
    /// `default-modes/experience/src/main.lua` end to end the same way `with_pack` exercises
    /// Sandbox's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn with_pack_and_experience(
        sources: &[(&str, &str)],
        pack_file: NamedTempFile,
        content: Content,
        experience: Experience,
        mode_config: HashMap<String, OptionValue>,
        capabilities: Capabilities,
        volume: Volume,
    ) -> Self {
        ensure_temp_dir();

        let event_poster = EmptyPoster();

        let (media_manager, _metadata, _pack_id) =
            MediaManager::open(pack_file.path(), event_poster.clone(), None).unwrap();
        let name_of = {
            let media_manager = media_manager.clone();
            move |id: u64| {
                media_manager
                    .get_media_by_id(id)
                    .ok()
                    .flatten()
                    .map(|media| media.name)
            }
        };

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
                    // Resolved against the fixture's own media, exactly as the real engine
                    // does -- a slot holds an id, and the modes are handed a file name.
                    config: mode_config,
                    content: lua_view(&content, &name_of),
                    experience: lua_view(&experience, &name_of),
                    chrome: ChromeDefaults::default(),
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

    /// Like `with_config`, but for tests that need a specific pack fixture (e.g. several
    /// distinctly-tagged media rows) and/or behaviour-derived `content`/`mode_config` --
    /// exactly what `resolve_mode_config` would have produced for `Mode::Sandbox`. The
    /// content-group query-layer tests use this to exercise `lib/media.lua` against real
    /// media without going through the full pack-behaviour.json resolution path.
    pub(super) fn with_pack(
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

    pub(super) fn with_config(
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

    pub(super) fn new(sources: &[(&str, &str)], with_image: bool) -> Self {
        Self::with_config(sources, with_image, all_capabilities(), Volume::default())
    }

    /// Like [`Harness::new`], but with the fake handler gating `SetWallpaper`/`OpenLink`/
    /// `ShowNotification` on `capabilities` exactly like `LewdwareApp` does -- lets a test
    /// simulate a hostile mode running with one or more capabilities denied.
    pub(super) fn with_capabilities(
        sources: &[(&str, &str)],
        with_image: bool,
        capabilities: Capabilities,
    ) -> Self {
        Self::with_config(sources, with_image, capabilities, Volume::default())
    }

    /// Like [`Harness::new`], but with the fake handler scaling `SpawnVideo`/`SpawnAudio`
    /// volume by `volume` exactly like `LewdwareApp` does -- lets a test check master volume
    /// is applied at spawn time.
    pub(super) fn with_volume(sources: &[(&str, &str)], with_image: bool, volume: Volume) -> Self {
        Self::with_config(sources, with_image, all_capabilities(), volume)
    }

    /// Simulate an `Event` the fake handler doesn't produce itself — see the field doc on
    /// [`Harness::event_tx`].
    pub(super) fn send_event(&self, event: Event) {
        let _ = self.event_tx.send(event);
    }

    pub(super) fn run_entrypoint(&mut self, entrypoint: &str) -> mlua::Result<()> {
        let result = self.runtime.run_entrypoint(entrypoint.to_string());
        self.pump_events();
        result
    }

    /// Delivers any `Event`s the fake handler has produced so far (e.g. `WindowClosed` from a
    /// `close()` request) — the real counterpart of the main thread's `about_to_wait`/
    /// `user_event` calling `LuaRuntime::handle_event`.
    pub(super) fn pump_events(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            self.runtime.handle_event(event).unwrap();
        }
    }

    /// Advances paused virtual time so `after()`/`every()` timers due within `duration` fire,
    /// then delivers any resulting events.
    pub(super) async fn advance(&mut self, duration: Duration) {
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

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.recorded.lock().unwrap().clone()
    }

    /// Evaluates a Lua expression against the running mode's global state and returns it as
    /// a number -- e.g. a counter a test's own wrapped API function incremented (see
    /// `wrapped_default_mode_sources`). Not needed for anything the fake handler already
    /// tracks via `Recorded`; only for effects it doesn't (e.g. `show_notification`, which
    /// carries no `Recorded` variant since nothing else needs to assert on it).
    pub(super) fn eval_number(&self, expr: &str) -> f64 {
        self.runtime.lua.load(expr).eval().unwrap()
    }

    /// Like `eval_number`, but for a string-valued global.
    pub(super) fn eval_string(&self, expr: &str) -> String {
        self.runtime.lua.load(expr).eval().unwrap()
    }
}
