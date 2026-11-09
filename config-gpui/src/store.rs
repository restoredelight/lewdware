//! The app's state, held in one gpui entity that every view reads. Ported from
//! `config/src/lib/store.svelte.ts` and `config/src-tauri/src/state.rs`, which were the same state
//! split across the IPC boundary.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use gpui_kit::{AsyncApp, Context, Task, WeakEntity};
use std::collections::HashMap;

use shared::audio::AudioDeviceInfo;
use shared::mode::{self, resolve_options, Metadata, OptionValue, StoredValue};
use shared::monitor::{MonitorInfo, MonitorRegion};
use shared::theme::{AppearanceChoice, ThemeChoice};
use shared::user_config::{
    self, AppConfig, AudioDeviceChoice, Key, Mode, Permissions, Volume, WallpaperRestore,
};
use tokio::sync::mpsc;

use crate::backend::hardware;
use crate::backend::modes;
use crate::backend::pack::{load_mode_file, load_pack, LoadedPack, UploadedModeEntry};
use crate::backend::schedule;
use crate::backend::supervisor::{self, EngineStatus};
use crate::backend::updates;
use crate::runtime;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    PackMode,
    Safety,
    Monitors,
    Audio,
    WindowStyle,
    Scheduling,
    Diagnostics,
}

impl Tab {
    pub const SETTINGS: [Tab; 6] = [
        Tab::PackMode,
        Tab::Safety,
        Tab::Monitors,
        Tab::Audio,
        Tab::WindowStyle,
        Tab::Scheduling,
    ];
    pub const UTILITY: [Tab; 1] = [Tab::Diagnostics];

    pub fn label(self) -> &'static str {
        match self {
            Tab::PackMode => "Pack & mode",
            Tab::Safety => "Safety",
            Tab::Monitors => "Monitors",
            Tab::Audio => "Audio",
            Tab::WindowStyle => "Window style",
            Tab::Scheduling => "Scheduling",
            Tab::Diagnostics => "Diagnostics",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EngineAction {
    Launch,
    Stop,
}

/// Ordered by priority, lowest first: only the highest-priority message is shown.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum FeedbackKind {
    /// A brief neutral note, for an action with no other visible effect (a clipboard copy).
    /// Dismisses itself.
    Confirm,
    Progress,
    Warning,
    Error,
}

/// `shared-ui/taskFeedback`: the one transient status message on show. Success is silent, so a
/// task that succeeds clears its message rather than replacing it.
#[derive(Clone, Debug)]
pub struct Feedback {
    pub task: &'static str,
    pub kind: FeedbackKind,
    pub message: String,
}

/// The two built-in modes, read from the copies compiled into the app.
pub struct BuiltinModes {
    pub sandbox: Metadata,
    pub experience: Metadata,
}

impl BuiltinModes {
    fn load() -> Self {
        let read = |bytes: &'static [u8], what: &str| {
            mode::read_mode_metadata(&mut Cursor::new(bytes))
                .unwrap_or_else(|error| panic!("failed to load embedded {what} mode: {error}"))
                .1
        };
        Self {
            sandbox: read(
                include_bytes!("../../default-modes/sandbox/build/Sandbox.lwmode"),
                "Sandbox",
            ),
            experience: read(
                include_bytes!("../../default-modes/experience/build/Sequence.lwmode"),
                "Sequence",
            ),
        }
    }
}

/// What loading reads off disk, all at once, before anything renders.
struct Loaded {
    config: AppConfig,
    pack: Option<LoadedPack>,
    uploaded: Vec<UploadedModeEntry>,
}

fn load_from_disk() -> Loaded {
    // A missing or unreadable config starts from defaults, as the Tauri app does.
    let config = user_config::load_config().unwrap_or_default();
    let pack = config.pack_path.as_ref().and_then(|path| {
        load_pack(path.clone())
            .inspect_err(|error| tracing::error!("failed to load pack: {error}"))
            .ok()
    });
    let uploaded = config
        .uploaded_modes
        .iter()
        .filter_map(|path| {
            load_mode_file(path.clone())
                .inspect_err(|e| tracing::error!("failed to load mode {}: {e}", path.display()))
                .ok()
        })
        .collect();
    Loaded {
        config,
        pack,
        uploaded,
    }
}

pub struct Store {
    pub builtin: BuiltinModes,
    /// `None` until the first load finishes.
    pub config: Option<AppConfig>,
    pub pack: Option<LoadedPack>,
    pub uploaded: Vec<UploadedModeEntry>,
    pub load_error: Option<String>,
    pub active_tab: Tab,
    pub engine: EngineStatus,
    pub engine_action: Option<EngineAction>,
    pub feedback: Vec<Feedback>,
    /// Audio outputs, probed from the engine. Loaded when the Audio page is first shown rather than
    /// at startup: devices come and go, so a list read at launch would be stale by then.
    pub audio_devices: Option<Vec<AudioDeviceInfo>>,
    pub audio_devices_loading: bool,
    pub audio_devices_error: Option<String>,
    /// Monitors, probed from the engine at load. Slower than the rest of the load, so it has its
    /// own state and only the Monitors page waits on it.
    pub monitors: Vec<MonitorInfo>,
    pub monitors_loading: bool,
    pub monitors_error: Option<String>,
    /// The supervisor's view of the schedule; `None` while no supervisor is reachable.
    pub schedule_status: Option<shared::ipc::control::ScheduleStatus>,
    /// Enabling or disabling scheduling is in flight (it registers OS autostart).
    pub schedule_enable_pending: bool,
    pub schedule_enable_error: Option<String>,
    /// The download page for a newer release, once the check finds one and until dismissed.
    pub update_url: Option<String>,
    /// What the selected mode presents. See `refresh_schema`.
    pub schema: modes::Schema,
    /// A pack is being picked or opened.
    pub pack_busy: bool,
    /// A mode is being uploaded.
    pub mode_busy: bool,
    /// The window looks on offer. Static for the life of the app.
    pub themes: Vec<crate::backend::themes::ThemeEntry>,
    _status_task: Task<()>,
}

impl Store {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let status_task = cx.spawn(Self::follow_supervisor);
        let mut store = Self {
            builtin: BuiltinModes::load(),
            config: None,
            pack: None,
            uploaded: Vec::new(),
            load_error: None,
            active_tab: Tab::PackMode,
            engine: EngineStatus::default(),
            engine_action: None,
            feedback: Vec::new(),
            audio_devices: None,
            audio_devices_loading: false,
            audio_devices_error: None,
            monitors: Vec::new(),
            monitors_loading: false,
            monitors_error: None,
            update_url: None,
            schedule_status: None,
            schedule_enable_pending: false,
            schedule_enable_error: None,
            schema: modes::Schema::default(),
            pack_busy: false,
            mode_busy: false,
            themes: crate::backend::themes::catalogue(),
            _status_task: status_task,
        };
        store.load(cx);
        store.check_for_update(cx);
        store
    }

    pub fn ready(&self) -> bool {
        self.config.is_some()
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.load_error = None;
        // Not awaited by the rest of the load: the monitor probe is slow and only one page needs it.
        self.load_monitors(cx);
        cx.spawn(async move |this, cx| {
            let result = runtime::blocking(load_from_disk).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(loaded) => {
                        this.config = Some(loaded.config);
                        this.pack = loaded.pack;
                        this.uploaded = loaded.uploaded;
                        this.refresh_schema();
                    }
                    Err(error) => this.load_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    async fn follow_supervisor(this: WeakEntity<Self>, cx: &mut AsyncApp) {
        let (sender, mut updates) = mpsc::unbounded_channel();
        // Detached on purpose: it exits by itself once `updates` is dropped with this task.
        runtime::spawn(supervisor::follow(sender));
        while let Some(info) = updates.recv().await {
            let engine = info
                .as_ref()
                .map(EngineStatus::from_info)
                .unwrap_or_default();
            let schedule = info.map(|info| info.schedule);
            if this
                .update(cx, |this, cx| {
                    if this.schedule_status != schedule {
                        this.schedule_status = schedule;
                        cx.notify();
                    }
                    this.apply_engine_status(engine, cx)
                })
                .is_err()
            {
                return;
            }
        }
    }

    /// Engine-level failures (a crash after launch, the supervisor giving up on restarts) arrive
    /// on the pushed status rather than from the launch call, so they go to the same feedback
    /// channel instead of being shown a second way.
    fn apply_engine_status(&mut self, engine: EngineStatus, cx: &mut Context<Self>) {
        if engine == self.engine {
            return;
        }
        match (&engine.running, &engine.error, &engine.warning) {
            (false, Some(error), _) => self.set_feedback(
                "engine",
                FeedbackKind::Error,
                format!("Lewdware stopped: {error}"),
            ),
            (true, _, Some(warning)) => {
                self.set_feedback("engine", FeedbackKind::Warning, warning.clone())
            }
            _ => {}
        }
        self.engine = engine;
        cx.notify();
    }

    pub fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    pub fn set_feedback(&mut self, task: &'static str, kind: FeedbackKind, message: String) {
        self.feedback.retain(|f| f.task != task);
        self.feedback.push(Feedback {
            task,
            kind,
            message,
        });
    }

    /// Shows a neutral note for a couple of seconds. Only for actions that change nothing visible.
    pub fn confirm(&mut self, task: &'static str, message: String, cx: &mut Context<Self>) {
        self.set_feedback(task, FeedbackKind::Confirm, message.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = this.update(cx, |this, cx| {
                // Unless something else has replaced it in the meantime.
                if this
                    .feedback
                    .iter()
                    .any(|f| f.task == task && f.message == message)
                {
                    this.dismiss_feedback(task, cx);
                }
            });
        })
        .detach();
    }

    /// Success is silent: it removes the task's message rather than showing one.
    pub fn clear_feedback(&mut self, task: &'static str) {
        self.feedback.retain(|f| f.task != task);
    }

    /// The highest-priority message (error > warning > progress, then the most recent), and how
    /// many others are waiting behind it.
    pub fn current_feedback(&self) -> Option<(&Feedback, usize)> {
        let top = self
            .feedback
            .iter()
            .enumerate()
            .max_by_key(|(index, f)| (f.kind, *index))
            .map(|(_, f)| f)?;
        Some((top, self.feedback.len() - 1))
    }

    pub fn dismiss_feedback(&mut self, task: &'static str, cx: &mut Context<Self>) {
        self.clear_feedback(task);
        cx.notify();
    }

    /// The file name of the selected pack.
    pub fn pack_name(&self) -> Option<String> {
        let path = self.config.as_ref()?.pack_path.as_ref()?;
        Some(
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        )
    }

    /// The selected mode's display name, as the mode list labels it.
    pub fn mode_name(&self) -> Option<String> {
        let config = self.config.as_ref()?;
        self.mode_choices()
            .into_iter()
            .find(|choice| modes::same_mode(&choice.mode, &config.mode))
            .map(|choice| choice.name)
    }

    pub fn mode_choices(&self) -> Vec<modes::ModeChoice> {
        modes::mode_choices(
            self.pack.as_ref(),
            &self.uploaded,
            &self.builtin.sandbox,
            &self.builtin.experience,
        )
    }

    /// Re-resolves what the selected mode presents. Called whenever the mode, the pack or the
    /// uploaded modes change -- synchronously, so options can never belong to a stale mode.
    fn refresh_schema(&mut self) {
        self.schema = match &self.config {
            Some(config) => modes::schema_for(
                &config.mode,
                self.pack.as_ref(),
                &self.uploaded,
                &self.builtin.sandbox,
                &self.builtin.experience,
            ),
            None => modes::Schema::default(),
        };
    }

    /// Every option's current value (the stored one read against the schema, or its default),
    /// plus the pack facts a `show_when` can test.
    pub fn option_values(&self) -> HashMap<String, OptionValue> {
        let empty = HashMap::new();
        let stored = self
            .config
            .as_ref()
            .and_then(|c| modes::stored_options(c, self.pack.as_ref()))
            .unwrap_or(&empty);
        let mut values = resolve_options(&self.schema.entries, stored);
        values.extend(
            self.schema
                .pack_has
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        values
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        let pack_id = self.pack.as_ref().map(|p| p.id);
        self.edit_config(cx, |config| {
            config.mode = match mode {
                Mode::Experience { .. } => Mode::Experience { pack: pack_id },
                other => other,
            }
        });
        self.refresh_schema();
    }

    /// Stores one option for the selected mode. A Sequence mode not yet scoped to a pack is pinned
    /// to the loaded one first, since that's where its options live.
    pub fn set_mode_option(&mut self, key: String, value: StoredValue, cx: &mut Context<Self>) {
        let pack_id = self.pack.as_ref().map(|p| p.id);
        self.edit_config(cx, |config| {
            if let Mode::Experience {
                pack: ref mut scoped @ None,
            } = config.mode
            {
                *scoped = pack_id;
            }
            let scope = config.mode.clone();
            config
                .mode_options
                .entry(scope)
                .or_default()
                .insert(key, value);
        });
    }

    pub fn pick_pack(&mut self, cx: &mut Context<Self>) {
        if self.pack_busy {
            return;
        }
        self.pack_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            // Choosing is not loading: no progress message until a file has been picked.
            let picked = runtime::run(async {
                rfd::AsyncFileDialog::new()
                    .set_title("Choose a pack")
                    .add_filter("Pack", &["lwpack"])
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            })
            .await
            .ok()
            .flatten();
            let Some(path) = picked else {
                let _ = this.update(cx, |this, cx| {
                    this.pack_busy = false;
                    cx.notify();
                });
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.set_feedback("pack", FeedbackKind::Progress, "Opening pack...".into());
                cx.notify();
            });
            let loaded = runtime::blocking({
                let path = path.clone();
                move || load_pack(path)
            })
            .await
            .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.pack_busy = false;
                match loaded {
                    Ok(pack) => {
                        this.clear_feedback("pack");
                        // A new pack resets the mode to what it suggests.
                        let mode = modes::initial_mode(&pack);
                        this.pack = Some(pack);
                        this.edit_config(cx, |config| {
                            config.pack_path = Some(path);
                            config.mode = mode;
                        });
                        this.refresh_schema();
                    }
                    Err(error) => this.set_feedback(
                        "pack",
                        FeedbackKind::Error,
                        format!("Couldn't open that pack: {error}"),
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn remove_pack(&mut self, cx: &mut Context<Self>) {
        self.pack = None;
        self.edit_config(cx, |config| {
            config.pack_path = None;
            match config.mode {
                Mode::Pack { .. } => config.mode = Mode::default(),
                Mode::Experience { ref mut pack } => *pack = None,
                _ => {}
            }
        });
        self.refresh_schema();
    }

    pub fn upload_mode(&mut self, cx: &mut Context<Self>) {
        if self.mode_busy {
            return;
        }
        self.mode_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let picked = runtime::run(async {
                rfd::AsyncFileDialog::new()
                    .set_title("Upload a mode")
                    .add_filter("Mode", &["lwmode"])
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            })
            .await
            .ok()
            .flatten();
            let already = picked.as_ref().is_some_and(|path| {
                this.update(cx, |this, _| this.uploaded.iter().any(|u| &u.path == path))
                    .unwrap_or(true)
            });
            let loaded = match picked {
                Some(path) if !already => Some(
                    runtime::blocking(move || load_mode_file(path))
                        .await
                        .and_then(|r| r),
                ),
                _ => None,
            };
            let _ = this.update(cx, |this, cx| {
                this.mode_busy = false;
                match loaded {
                    Some(Ok(entry)) => {
                        this.uploaded.push(entry);
                        this.save(cx);
                    }
                    Some(Err(error)) => this.set_feedback(
                        "mode",
                        FeedbackKind::Error,
                        format!("Couldn't upload mode: {error}"),
                    ),
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// If the removed mode was selected, falls back to Sandbox.
    pub fn remove_uploaded_mode(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.uploaded.retain(|u| u.path != path);
        self.edit_config(cx, |config| {
            if matches!(&config.mode, Mode::File { path: selected } if selected == &path) {
                config.mode = Mode::default();
            }
        });
        self.refresh_schema();
    }

    /// Writes the config to disk, with the uploaded mode list it implies. A Sequence mode that
    /// names no pack is pinned to the loaded one, as the Tauri `save_config` does.
    fn config_to_save(&self) -> Option<AppConfig> {
        let mut config = self.config.clone()?;
        config.uploaded_modes = self.uploaded.iter().map(|u| u.path.clone()).collect();
        if let Mode::Experience { ref mut pack } = config.mode {
            if pack.is_none() {
                *pack = self.pack.as_ref().map(|p| p.id);
            }
        }
        Some(config)
    }

    pub fn launch(&mut self, cx: &mut Context<Self>) {
        if self.engine_action.is_some() {
            return;
        }
        let Some(config) = self.config_to_save() else {
            return;
        };
        self.engine_action = Some(EngineAction::Launch);
        self.set_feedback(
            "engine",
            FeedbackKind::Progress,
            "Launching Lewdware...".into(),
        );
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = async {
                runtime::blocking(move || user_config::save_config(&config))
                    .await?
                    .map_err(|e| anyhow::anyhow!("settings could not be saved: {e}"))?;
                runtime::run(supervisor::launch()).await?
            }
            .await;
            // The push subscription may take a beat to reach a freshly spawned supervisor.
            let status = runtime::run(supervisor::status()).await.ok().flatten();
            let _ = this.update(cx, |this, cx| {
                this.engine_action = None;
                match result {
                    Ok(()) => this.clear_feedback("engine"),
                    Err(error) => this.set_feedback(
                        "engine",
                        FeedbackKind::Error,
                        format!("Lewdware failed to launch: {error}"),
                    ),
                }
                if let Some(info) = status {
                    this.schedule_status = Some(info.schedule.clone());
                    this.apply_engine_status(EngineStatus::from_info(&info), cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if self.engine_action.is_some() {
            return;
        }
        self.engine_action = Some(EngineAction::Stop);
        self.set_feedback(
            "engine",
            FeedbackKind::Progress,
            "Stopping Lewdware...".into(),
        );
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime::run(supervisor::stop()).await;
            let _ = this.update(cx, |this, cx| {
                this.engine_action = None;
                match result {
                    Ok(()) => this.clear_feedback("engine"),
                    Err(error) => this.set_feedback(
                        "engine",
                        FeedbackKind::Error,
                        format!("Lewdware couldn't be stopped: {error}"),
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    #[allow(dead_code)]
    pub fn pack_path(&self) -> Option<&PathBuf> {
        self.config.as_ref()?.pack_path.as_ref()
    }

    /// Persists the current config in the background. Saves can overlap (a toggle flipped twice
    /// quickly), so each carries a generation and an older one never overwrites a newer one.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        self.save_then(cx, false);
    }

    /// Saves, and once the write has landed, tells a running supervisor to reload its schedule if
    /// `reload_schedule` is set. The order matters: the supervisor reads the file.
    fn save_then(&mut self, cx: &mut Context<Self>, reload_schedule: bool) {
        static LATEST: AtomicU64 = AtomicU64::new(0);
        static WRITTEN: Mutex<u64> = Mutex::new(0);

        let Some(config) = self.config_to_save() else {
            return;
        };
        let generation = LATEST.fetch_add(1, Ordering::SeqCst) + 1;
        cx.spawn(async move |this, cx| {
            let result = runtime::blocking(move || {
                let mut written = WRITTEN.lock().unwrap();
                if *written > generation {
                    return Ok(());
                }
                user_config::save_config(&config)?;
                *written = generation;
                Ok::<_, anyhow::Error>(())
            })
            .await
            .and_then(|r| r);
            if result.is_ok() && reload_schedule {
                let _ = runtime::run(schedule::reload()).await;
            }
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.clear_feedback("save"),
                    Err(error) => this.set_feedback(
                        "save",
                        FeedbackKind::Error,
                        format!("Couldn't save settings: {error}"),
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Applies `edit` to the config and saves it.
    pub fn edit_config(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut AppConfig)) {
        let Some(config) = self.config.as_mut() else {
            return;
        };
        edit(config);
        cx.notify();
        self.save(cx);
    }

    pub fn set_stop_button(&mut self, key: Key, cx: &mut Context<Self>) {
        self.edit_config(cx, |config| config.stop_button = key);
    }

    pub fn set_permission(
        &mut self,
        permission: fn(&mut Permissions) -> &mut bool,
        allowed: bool,
        cx: &mut Context<Self>,
    ) {
        self.edit_config(cx, |config| *permission(&mut config.permissions) = allowed);
    }

    /// On a desktop that can't report its own wallpaper, choosing an image here is what makes the
    /// wallpaper permission usable at all.
    pub fn set_wallpaper_restore(&mut self, restore: WallpaperRestore, cx: &mut Context<Self>) {
        self.edit_config(cx, |config| config.wallpaper.restore = restore);
    }

    /// Updates the volume without saving, for a slider being dragged. Pair with `save` on release.
    pub fn preview_volume(
        &mut self,
        channel: fn(&mut Volume) -> &mut f32,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(config) = self.config.as_mut() {
            *channel(&mut config.volume) = value;
            cx.notify();
        }
    }

    /// `None` is the system default, which the engine re-resolves each time it opens a sink.
    pub fn set_audio_device(&mut self, device: Option<AudioDeviceChoice>, cx: &mut Context<Self>) {
        self.edit_config(cx, |config| config.audio_device = device);
    }

    pub fn load_audio_devices(&mut self, cx: &mut Context<Self>) {
        if self.audio_devices_loading {
            return;
        }
        self.audio_devices_loading = true;
        self.audio_devices_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime::run(hardware::audio_devices())
                .await
                .and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.audio_devices_loading = false;
                match result {
                    Ok(devices) => this.audio_devices = Some(devices),
                    Err(error) => this.audio_devices_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Store {
    pub fn load_monitors(&mut self, cx: &mut Context<Self>) {
        if self.monitors_loading {
            return;
        }
        self.monitors_loading = true;
        self.monitors_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime::run(hardware::monitors()).await.and_then(|r| r);
            let _ = this.update(cx, |this, cx| {
                this.monitors_loading = false;
                match result {
                    Ok(monitors) => this.monitors = monitors,
                    Err(error) => this.monitors_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn monitor_enabled(&self, id: &str) -> bool {
        self.config
            .as_ref()
            .is_none_or(|c| !c.disabled_monitors.iter().any(|m| m == id))
    }

    /// The part of a monitor popups may use; the whole screen when nothing is stored.
    pub fn monitor_region(&self, id: &str) -> MonitorRegion {
        self.config
            .as_ref()
            .and_then(|c| c.monitor_regions.get(id).copied())
            .unwrap_or(MonitorRegion::FULL)
    }

    pub fn set_monitor_enabled(&mut self, id: String, enabled: bool, cx: &mut Context<Self>) {
        self.edit_config(cx, |config| {
            config.disabled_monitors.retain(|m| m != &id);
            if !enabled {
                config.disabled_monitors.push(id);
            }
        });
    }

    /// `None` removes the entry rather than storing a full-screen rectangle, so a monitor the user
    /// never restricted stays absent from `config.json`.
    pub fn set_monitor_region(
        &mut self,
        id: String,
        region: Option<MonitorRegion>,
        cx: &mut Context<Self>,
    ) {
        self.edit_config(cx, |config| match region {
            Some(region) => {
                config.monitor_regions.insert(id, region);
            }
            None => {
                config.monitor_regions.remove(&id);
            }
        });
    }

    pub fn set_theme(&mut self, theme: ThemeChoice, cx: &mut Context<Self>) {
        self.edit_config(cx, |config| config.theme = theme);
    }

    pub fn set_appearance(&mut self, appearance: AppearanceChoice, cx: &mut Context<Self>) {
        self.edit_config(cx, |config| config.appearance = appearance);
    }

    /// Edits the schedule's content (rules, quiet hours, pacing) and saves, then has a running
    /// supervisor pick it up without waiting for its next wake.
    pub fn edit_schedule(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut shared::schedule::ScheduleConfig),
    ) {
        let Some(config) = self.config.as_mut() else {
            return;
        };
        edit(&mut config.schedule);
        let enabled = config.schedule.enabled;
        cx.notify();
        self.save_then(cx, enabled);
    }

    /// Turning scheduling on or off also registers or removes OS autostart for the supervisor,
    /// which can fail. The setting only changes once that has worked, so the switch never claims
    /// a state that isn't true; if the save then fails, the registration is put back.
    pub fn set_schedule_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let Some(mut config) = self.config_to_save() else {
            return;
        };
        if self.schedule_enable_pending {
            return;
        }
        let previous = config.schedule.enabled;
        config.schedule.enabled = enabled;
        self.schedule_enable_pending = true;
        self.schedule_enable_error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime::blocking(move || -> anyhow::Result<()> {
                schedule::set_autostart(enabled)?;
                if let Err(save_error) = user_config::save_config(&config) {
                    return Err(match schedule::set_autostart(previous) {
                        Ok(()) => save_error,
                        Err(rollback) => anyhow::anyhow!(
                            "{save_error}; also failed to restore autostart: {rollback}"
                        ),
                    });
                }
                Ok(())
            })
            .await
            .and_then(|r| r);
            if result.is_ok() {
                runtime::run(schedule::apply_enabled(enabled)).await.ok();
            }
            let status = runtime::run(supervisor::status()).await.ok().flatten();
            let _ = this.update(cx, |this, cx| {
                this.schedule_enable_pending = false;
                match result {
                    Ok(()) => {
                        if let Some(config) = this.config.as_mut() {
                            config.schedule.enabled = enabled;
                        }
                        this.schedule_status = status.map(|info| info.schedule);
                    }
                    Err(error) => {
                        this.schedule_enable_error = Some(error.to_string());
                        this.set_feedback(
                            "schedule-enabled",
                            FeedbackKind::Error,
                            format!("Couldn't update scheduling: {error}"),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Ends a running pause early, then asks the supervisor rather than assuming: it owns the
    /// cooldown.
    pub fn resume_schedule(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let result = runtime::run(schedule::resume()).await.and_then(|r| r);
            let status = runtime::run(supervisor::status()).await.ok().flatten();
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.set_feedback(
                        "schedule-resume",
                        FeedbackKind::Error,
                        format!("Couldn't resume the schedule: {error}"),
                    );
                }
                this.schedule_status = status.map(|info| info.schedule);
                cx.notify();
            });
        })
        .detach();
    }

    /// A failed check (offline, say) is silent: there's nothing the user needs to do about it.
    fn check_for_update(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(url))) = runtime::run(updates::check()).await {
                let _ = this.update(cx, |this, cx| {
                    this.update_url = Some(url);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn dismiss_update(&mut self, cx: &mut Context<Self>) {
        self.update_url = None;
        cx.notify();
    }
}
