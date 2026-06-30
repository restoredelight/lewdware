use std::{
    collections::HashMap,
    io::{Cursor, Read, Seek, SeekFrom},
    path::PathBuf,
    process::{Child, Command},
    sync::Mutex,
};

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use serde::{Deserialize, Serialize};

// ─── Update check ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct UpdateManifest {
    version: String,
    download_page: String,
}

fn parse_version(v: &str) -> (u32, u32, u32) {
    let mut parts = v.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

#[tauri::command]
async fn check_for_update() -> Result<Option<String>, String> {
    let current = env!("CARGO_PKG_VERSION");
    let resp = reqwest::get("https://lewdware.net/download/latest.json")
        .await
        .map_err(|e| e.to_string())?;
    let manifest: UpdateManifest = resp.json().await.map_err(|e| e.to_string())?;
    if parse_version(&manifest.version) > parse_version(current) {
        Ok(Some(manifest.download_page))
    } else {
        Ok(None)
    }
}
use serde_json::Value as JsonValue;
use indexmap::IndexMap;
use shared::{
    db::migrate,
    mode::{self, ModeEntry, Metadata, OptionType, OptionValue, ShowWhen},
    read_pack::read_pack_metadata,
    user_config::{self, AppConfig, Key, Mode},
};
use tauri::{AppHandle, Manager};
use tempfile::NamedTempFile;
use tokio::sync::oneshot;

// ─── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(tag = "type")]
pub enum ModeIdDto {
    Default { mode: String },
    Pack { id: u64, mode: String },
    File { path: String, mode: String },
}

impl From<Mode> for ModeIdDto {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Default(mode) => ModeIdDto::Default { mode },
            Mode::Pack { id, mode } => ModeIdDto::Pack { id, mode },
            Mode::File { path, mode } => ModeIdDto::File {
                path: path.to_string_lossy().into_owned(),
                mode,
            },
        }
    }
}

impl From<ModeIdDto> for Mode {
    fn from(dto: ModeIdDto) -> Self {
        match dto {
            ModeIdDto::Default { mode } => Mode::Default(mode),
            ModeIdDto::Pack { id, mode } => Mode::Pack { id, mode },
            ModeIdDto::File { path, mode } => Mode::File {
                path: PathBuf::from(path),
                mode,
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModeOptionsEntry {
    pub mode: ModeIdDto,
    pub options: HashMap<String, OptionValue>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigDto {
    pub pack_path: Option<String>,
    pub mode: ModeIdDto,
    pub mode_options: Vec<ModeOptionsEntry>,
    pub panic_button: Key,
    pub disabled_monitors: Vec<String>,
}

impl From<AppConfig> for ConfigDto {
    fn from(c: AppConfig) -> Self {
        let mode_options = c
            .mode_options
            .into_iter()
            .map(|(k, v)| ModeOptionsEntry {
                mode: k.into(),
                options: v,
            })
            .collect();

        ConfigDto {
            pack_path: c.pack_path.and_then(|p| p.to_str().map(str::to_string)),
            mode: c.mode.into(),
            mode_options,
            panic_button: c.panic_button,
            disabled_monitors: c.disabled_monitors,
        }
    }
}

impl From<ConfigDto> for AppConfig {
    fn from(dto: ConfigDto) -> Self {
        let mode_options = dto
            .mode_options
            .into_iter()
            .map(|e| (Mode::from(e.mode), e.options))
            .collect();

        AppConfig {
            pack_path: dto.pack_path.map(PathBuf::from),
            uploaded_modes: Vec::new(),
            mode: dto.mode.into(),
            mode_options,
            tags: None,
            panic_button: dto.panic_button,
            disabled_monitors: dto.disabled_monitors,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MonitorDto {
    pub id: String,
    pub name: String,
    pub primary: bool,
    pub disabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModeEntryDto {
    pub id: ModeIdDto,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModeGroupDto {
    pub label: String,
    pub source: String,
    pub entries: Vec<ModeEntryDto>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModeOptionDto {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub option_type: OptionType,
    pub value: OptionValue,
    pub optional: bool,
    pub show_when: Option<ShowWhen>,
}

#[derive(Serialize, Clone, Debug)]
pub struct OptionGroupDto {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub show_when: Option<ShowWhen>,
    pub entries: Vec<OptionEntryDto>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind")]
pub enum OptionEntryDto {
    Option(ModeOptionDto),
    Group(OptionGroupDto),
}

// ─── State ───────────────────────────────────────────────────────────────────

struct PackModeEntry {
    id: u64,
    metadata: Metadata,
}

struct UploadedModeEntry {
    path: PathBuf,
    metadata: Metadata,
}

struct LoadedPack {
    _db_file: NamedTempFile,
    modes: Vec<PackModeEntry>,
}

pub struct AppState {
    config: Mutex<AppConfig>,
    pack: Mutex<Option<LoadedPack>>,
    uploaded: Mutex<Vec<UploadedModeEntry>>,
    default_modes: Metadata,
    lewdware_process: Mutex<Option<Child>>,
}

pub type State<'a> = tauri::State<'a, AppState>;

// ─── Pack / mode loading ──────────────────────────────────────────────────────

fn load_pack(path: PathBuf) -> anyhow::Result<LoadedPack> {
    let mut file = std::fs::File::open(&path)?;
    let (header, _) = read_pack_metadata(&mut file)?;

    let mut db_file = NamedTempFile::new()?;
    file.seek(SeekFrom::Start(header.index_offset))?;
    let mut db_data = (&mut file).take(header.index_length);
    std::io::copy(&mut db_data, db_file.as_file_mut())?;

    let manager = SqliteConnectionManager::file(db_file.path());
    let pool = Pool::builder().build(manager)?;
    let conn = pool.get()?;
    migrate(&conn)?;

    let mut stmt = conn.prepare("SELECT id, file FROM modes")?;
    let rows: Vec<(u64, Vec<u8>)> = stmt
        .query_map([], |row| Ok((row.get("id")?, row.get("file")?)))?
        .collect::<rusqlite::Result<_>>()?;

    let mut modes = Vec::new();
    for (id, data) in rows {
        let mut cursor = Cursor::new(data);
        let (_, metadata) = mode::read_mode_metadata(&mut cursor)?;
        modes.push(PackModeEntry { id, metadata });
    }

    Ok(LoadedPack {
        _db_file: db_file,
        modes,
    })
}

fn load_mode_file(path: PathBuf) -> anyhow::Result<UploadedModeEntry> {
    let mut file = std::fs::File::open(&path)?;
    let (_, metadata) = mode::read_mode_metadata(&mut file)?;
    Ok(UploadedModeEntry { path, metadata })
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn build_mode_groups(state: &AppState) -> Vec<ModeGroupDto> {
    let mut groups = Vec::new();

    if let Some(pack) = state.pack.lock().unwrap().as_ref() {
        let label = pack
            .modes
            .first()
            .map(|m| m.metadata.name.clone())
            .unwrap_or_default();

        let entries: Vec<_> = pack
            .modes
            .iter()
            .flat_map(|m| {
                m.metadata.modes.iter().map(|(key, mode)| ModeEntryDto {
                    id: ModeIdDto::Pack {
                        id: m.id,
                        mode: key.clone(),
                    },
                    name: mode.name.clone(),
                })
            })
            .collect();

        if !entries.is_empty() {
            groups.push(ModeGroupDto {
                label,
                source: "pack".into(),
                entries,
            });
        }
    }

    let uploaded = state.uploaded.lock().unwrap();
    for entry in uploaded.iter() {
        let file_name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let label = format!("{} ({})", entry.metadata.name, file_name);
        let path_str = entry.path.to_string_lossy().into_owned();

        let entries: Vec<_> = entry
            .metadata
            .modes
            .iter()
            .map(|(key, mode)| ModeEntryDto {
                id: ModeIdDto::File {
                    path: path_str.clone(),
                    mode: key.clone(),
                },
                name: mode.name.clone(),
            })
            .collect();

        groups.push(ModeGroupDto {
            label,
            source: "uploaded".into(),
            entries,
        });
    }

    let entries: Vec<_> = state
        .default_modes
        .modes
        .iter()
        .map(|(key, mode)| ModeEntryDto {
            id: ModeIdDto::Default { mode: key.clone() },
            name: mode.name.clone(),
        })
        .collect();

    groups.push(ModeGroupDto {
        label: state.default_modes.name.clone(),
        source: "builtin".into(),
        entries,
    });

    groups
}

fn get_mode_options_for(config: &AppConfig, state: &AppState) -> Vec<OptionEntryDto> {
    let mode_meta = match &config.mode {
        Mode::Default(key) => state.default_modes.modes.get(key).cloned(),
        Mode::Pack { id, mode } => {
            let pack = state.pack.lock().unwrap();
            pack.as_ref().and_then(|p| {
                p.modes
                    .iter()
                    .find(|m| m.id == *id)
                    .and_then(|m| m.metadata.modes.get(mode).cloned())
            })
        }
        Mode::File { path, mode } => {
            let uploaded = state.uploaded.lock().unwrap();
            uploaded
                .iter()
                .find(|u| &u.path == path)
                .and_then(|u| u.metadata.modes.get(mode).cloned())
        }
    };

    let Some(mode_meta) = mode_meta else {
        return Vec::new();
    };

    let stored = config
        .mode_options
        .get(&config.mode)
        .cloned()
        .unwrap_or_default();

    fn build_entries(
        entries: &IndexMap<String, ModeEntry>,
        stored: &HashMap<String, OptionValue>,
    ) -> Vec<OptionEntryDto> {
        entries
            .iter()
            .map(|(key, entry)| match entry {
                ModeEntry::Option(opt) => {
                    let value = stored
                        .get(key)
                        .filter(|v| opt.matches_value(v))
                        .cloned()
                        .unwrap_or_else(|| opt.default_value());
                    OptionEntryDto::Option(ModeOptionDto {
                        key: key.clone(),
                        label: opt.label.clone(),
                        description: opt.description.clone(),
                        option_type: opt.option_type.clone(),
                        value,
                        optional: opt.optional,
                        show_when: opt.show_when.clone(),
                    })
                }
                ModeEntry::Group(group) => OptionEntryDto::Group(OptionGroupDto {
                    key: key.clone(),
                    label: group.label.clone(),
                    description: group.description.clone(),
                    show_when: group.show_when.clone(),
                    entries: build_entries(&group.entries, stored),
                }),
            })
            .collect()
    }

    build_entries(&mode_meta.entries, &stored)
}

fn save_to_disk(config: &AppConfig, uploaded: &[UploadedModeEntry]) -> anyhow::Result<()> {
    let mut c = config.clone();
    c.uploaded_modes = uploaded.iter().map(|u| u.path.clone()).collect();
    user_config::save_config(&c)
}

// ─── Commands ─────────────────────────────────────────────────────────────────

#[tauri::command]
fn get_config(state: State<'_>) -> ConfigDto {
    state.config.lock().unwrap().clone().into()
}

#[tauri::command]
fn save_config(state: State<'_>, config: ConfigDto) -> Result<(), String> {
    let mut current = state.config.lock().unwrap();
    let mut new_config: AppConfig = config.into();

    // Preserve fields managed separately from the DTO
    new_config.uploaded_modes = current.uploaded_modes.clone();
    new_config.tags = current.tags.clone();

    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&new_config, &uploaded).map_err(|e| e.to_string())?;
    *current = new_config;
    Ok(())
}

#[tauri::command]
async fn get_monitors(app_handle: AppHandle, state: State<'_>) -> Result<Vec<MonitorDto>, String> {
    let primary_name = app_handle
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .and_then(|m| m.name().cloned());

    let disabled = state.config.lock().unwrap().disabled_monitors.clone();

    let mut monitors: Vec<_> = app_handle
        .available_monitors()
        .map_err(|e| e.to_string())?
        .iter()
        .filter_map(|m| {
            let id = m.name()?.to_string();
            let primary = Some(&id) == primary_name.as_ref();
            let size = m.size();
            let name = format!("{id} ({}x{})", size.width, size.height);
            let is_disabled = disabled.contains(&id);
            Some(MonitorDto {
                id,
                name,
                primary,
                disabled: is_disabled,
            })
        })
        .collect();

    if let Some(pos) = monitors.iter().position(|m| m.primary) {
        monitors.swap(0, pos);
    }

    Ok(monitors)
}

#[tauri::command]
fn get_mode_groups(state: State<'_>) -> Vec<ModeGroupDto> {
    build_mode_groups(&state)
}

#[tauri::command]
fn get_mode_options(state: State<'_>) -> Vec<OptionEntryDto> {
    let config = state.config.lock().unwrap();
    get_mode_options_for(&config, &state)
}

#[tauri::command]
fn set_mode_option(state: State<'_>, key: String, value: JsonValue) -> Result<(), String> {
    let mut config = state.config.lock().unwrap();
    let mode = config.mode.clone();

    // Find the option type so we can coerce the value to the right variant
    let opt_type = get_option_type_for_key(&config, &mode, &key, &state);

    let typed_value = coerce_option_value(value, opt_type.as_ref())
        .ok_or_else(|| "invalid option value".to_string())?;

    config
        .mode_options
        .entry(mode)
        .or_default()
        .insert(key, typed_value);
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())
}

fn get_option_type_for_key(
    _config: &AppConfig,
    mode: &Mode,
    key: &str,
    state: &AppState,
) -> Option<OptionType> {
    let mode_meta = match mode {
        Mode::Default(k) => state.default_modes.modes.get(k).cloned(),
        Mode::Pack { id, mode } => {
            let pack = state.pack.lock().unwrap();
            pack.as_ref().and_then(|p| {
                p.modes
                    .iter()
                    .find(|m| m.id == *id)?
                    .metadata
                    .modes
                    .get(mode)
                    .cloned()
            })
        }
        Mode::File { path, mode } => {
            let uploaded = state.uploaded.lock().unwrap();
            uploaded
                .iter()
                .find(|u| &u.path == path)?
                .metadata
                .modes
                .get(mode)
                .cloned()
        }
    }?;

    mode_meta.get_option(key).map(|o| o.option_type.clone())
}

fn coerce_option_value(value: JsonValue, opt_type: Option<&OptionType>) -> Option<OptionValue> {
    match (opt_type, &value) {
        (_, JsonValue::Null) => Some(OptionValue::Null),
        (Some(OptionType::Enum { .. }), JsonValue::String(s)) => Some(OptionValue::Enum(s.clone())),
        (Some(OptionType::Integer { .. }), JsonValue::Number(n)) => {
            Some(OptionValue::Integer(n.as_i64()?))
        }
        (Some(OptionType::Number { .. }), JsonValue::Number(n)) => {
            Some(OptionValue::Number(n.as_f64()?))
        }
        (Some(OptionType::String { .. }), JsonValue::String(s)) => {
            Some(OptionValue::String(s.clone()))
        }
        (Some(OptionType::Boolean { .. }), JsonValue::Bool(b)) => Some(OptionValue::Boolean(*b)),
        // fallback: untagged deserialize
        _ => serde_json::from_value(value).ok(),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PickPackResult {
    pub pack_path: String,
    pub mode_groups: Vec<ModeGroupDto>,
    pub first_mode: Option<ModeIdDto>,
}

#[tauri::command]
async fn pick_pack(
    app_handle: AppHandle,
    state: State<'_>,
) -> Result<Option<PickPackResult>, String> {
    use tauri_plugin_dialog::DialogExt;

    let path = app_handle
        .dialog()
        .file()
        .add_filter("Pack", &["lwpack"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());

    let Some(path) = path else {
        return Ok(None);
    };

    let loaded = tokio::task::spawn_blocking({
        let path = path.clone();
        move || load_pack(path)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let first_mode = loaded.modes.first().and_then(|m| {
        m.metadata.modes.first().map(|(key, _)| ModeIdDto::Pack {
            id: m.id,
            mode: key.clone(),
        })
    });

    let pack_path_str = path.to_string_lossy().into_owned();
    *state.pack.lock().unwrap() = Some(loaded);

    let mut config = state.config.lock().unwrap();
    config.pack_path = Some(path);
    if let Some(ref m) = first_mode {
        config.mode = m.clone().into();
    }

    let groups = build_mode_groups(&state);
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())?;

    Ok(Some(PickPackResult {
        pack_path: pack_path_str,
        mode_groups: groups,
        first_mode,
    }))
}

#[tauri::command]
fn remove_pack(state: State<'_>) -> Result<(), String> {
    *state.pack.lock().unwrap() = None;
    let mut config = state.config.lock().unwrap();
    config.pack_path = None;
    if matches!(config.mode, Mode::Pack { .. }) {
        config.mode = Mode::default();
    }
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UploadModeResult {
    pub mode_groups: Vec<ModeGroupDto>,
}

#[tauri::command]
async fn upload_mode(
    app_handle: AppHandle,
    state: State<'_>,
) -> Result<Option<UploadModeResult>, String> {
    use tauri_plugin_dialog::DialogExt;

    let path = app_handle
        .dialog()
        .file()
        .add_filter("Mode", &["lwmode"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());

    let Some(path) = path else {
        return Ok(None);
    };

    {
        let uploaded = state.uploaded.lock().unwrap();
        if uploaded.iter().any(|u| u.path == path) {
            return Ok(Some(UploadModeResult {
                mode_groups: build_mode_groups(&state),
            }));
        }
    }

    let entry = tokio::task::spawn_blocking({
        let path = path.clone();
        move || load_mode_file(path)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    state.uploaded.lock().unwrap().push(entry);

    let mut config = state.config.lock().unwrap();
    config.uploaded_modes.push(path);
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())?;

    Ok(Some(UploadModeResult {
        mode_groups: build_mode_groups(&state),
    }))
}

#[tauri::command]
fn remove_uploaded_mode(state: State<'_>, path: String) -> Result<Vec<ModeGroupDto>, String> {
    let path = PathBuf::from(&path);

    state.uploaded.lock().unwrap().retain(|u| u.path != path);

    let mut config = state.config.lock().unwrap();
    config.uploaded_modes.retain(|p| p != &path);
    if let Mode::File { path: ref mp, .. } = config.mode.clone() {
        if mp == &path {
            config.mode = Mode::default();
        }
    }
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())?;

    Ok(build_mode_groups(&state))
}

// ─── Process management ───────────────────────────────────────────────────────

fn find_lewdware() -> Option<Command> {
    let bin_name = if cfg!(windows) {
        "lewdware-engine.exe"
    } else {
        "lewdware-engine"
    };

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe
            .canonicalize()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_owned()))
        {
            // Windows / macOS: engine sits next to the config binary.
            candidates.push(dir.join(bin_name));
            // Portable tar.gz on Linux: bin/lewdware → lib/lewdware/lewdware-engine.
            if let Some(root) = dir.parent() {
                candidates.push(root.join("lib").join("lewdware").join(bin_name));
            }
        }
    }

    // Linux package installs (deb/rpm): engine lives in /usr/lib/lewdware/.
    #[cfg(target_os = "linux")]
    candidates.push(PathBuf::from("/usr/lib/lewdware").join(bin_name));

    for path in candidates {
        if path.exists() {
            let mut cmd = Command::new(path);
            shared::utils::sanitize_child_env(&mut cmd);
            return Some(cmd);
        }
    }

    None
}

#[tauri::command]
fn launch_lewdware(state: State<'_>) -> Result<(), String> {
    let mut guard = state.lewdware_process.lock().unwrap();

    // No-op if already running.
    if let Some(child) = guard.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            return Ok(());
        }
    }

    let mut cmd = find_lewdware().ok_or("Could not find lewdware binary")?;
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    *guard = Some(child);
    Ok(())
}

#[tauri::command]
fn stop_lewdware(state: State<'_>) -> Result<(), String> {
    let child = state.lewdware_process.lock().unwrap().take();

    if let Some(mut child) = child {
        #[cfg(unix)]
        {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid;
            let _ = signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM);
        }
        #[cfg(not(unix))]
        let _ = child.kill();

        // Reap without blocking the command thread.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }

    Ok(())
}

#[tauri::command]
fn lewdware_running(state: State<'_>) -> bool {
    let mut guard = state.lewdware_process.lock().unwrap();
    match guard.as_mut() {
        Some(child) => matches!(child.try_wait(), Ok(None)),
        None => false,
    }
}

// ─── Input Monitoring (macOS) ─────────────────────────────────────────────────

#[tauri::command]
async fn input_monitoring_granted(#[allow(unused)] app_handle: AppHandle) -> Result<bool, String> {
    #[cfg(target_vendor = "apple")]
    {
        let (tx, rx) = oneshot::channel();

        app_handle
            .run_on_main_thread(move || {
                #[link(name = "CoreGraphics", kind = "framework")]
                unsafe extern "C-unwind" {
                    fn CGPreflightListenEventAccess() -> bool;
                }
                tx.send(unsafe { CGPreflightListenEventAccess() });
            })
            .map_err(|err| err.to_string())?;

        return rx.await.map_err(|err| err.to_string());
    }

    #[cfg(not(target_vendor = "apple"))]
    Ok(true)
}

#[tauri::command]
fn request_input_monitoring(#[allow(unused)] app_handle: AppHandle) -> Result<bool, String> {
    #[cfg(target_vendor = "apple")]
    {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        let granted = Arc::new(AtomicBool::new(false));
        let granted_clone = granted.clone();

        app_handle
            .run_on_main_thread(move || {
                #[link(name = "CoreGraphics", kind = "framework")]
                unsafe extern "C-unwind" {
                    fn CGRequestListenEventAccess() -> bool;
                }
                granted_clone.store(unsafe { CGRequestListenEventAccess() }, Ordering::Relaxed);
            })
            .map_err(|err| err.to_string())?;

        return Ok(granted.load(Ordering::Relaxed));
    }
    #[cfg(not(target_vendor = "apple"))]
    Ok(true)
}

#[tauri::command]
fn open_input_monitoring_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
            .spawn();
    }
}

// ─── Logs ─────────────────────────────────────────────────────────────────────

fn open_log_dir() -> Result<(), String> {
    let dir = shared::logging::log_dir().ok_or("Could not determine log directory")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn open_logs() -> Result<(), String> {
    open_log_dir()
}

// ─── Entry ────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let default_modes_bytes = include_bytes!("../../../default-modes/build/Default Modes.lwmode");
    let default_modes = mode::read_mode_metadata(&mut Cursor::new(default_modes_bytes))
        .expect("failed to load embedded default modes")
        .1;

    let _log_guard = shared::logging::init("config");

    let config = user_config::load_config().unwrap_or_default();

    let pack = config.pack_path.as_ref().and_then(|p| {
        load_pack(p.clone())
            .inspect_err(|e| tracing::error!("failed to load pack: {e}"))
            .ok()
    });

    let uploaded: Vec<UploadedModeEntry> = config
        .uploaded_modes
        .iter()
        .filter_map(|p| {
            load_mode_file(p.clone())
                .inspect_err(|e| tracing::error!("failed to load mode {}: {e}", p.display()))
                .ok()
        })
        .collect();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            config: Mutex::new(config),
            pack: Mutex::new(pack),
            uploaded: Mutex::new(uploaded),
            default_modes,
            lewdware_process: Mutex::new(None),
        })
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
                window.set_icon(icon)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            get_monitors,
            get_mode_groups,
            get_mode_options,
            set_mode_option,
            pick_pack,
            remove_pack,
            upload_mode,
            remove_uploaded_mode,
            launch_lewdware,
            stop_lewdware,
            lewdware_running,
            open_logs,
            check_for_update,
            input_monitoring_granted,
            request_input_monitoring,
            open_input_monitoring_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
