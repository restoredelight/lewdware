use std::{
    collections::HashMap,
    io::{Cursor, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Mutex,
};

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::OptionalExtension;
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
use indexmap::IndexMap;
use serde_json::Value as JsonValue;
use shared::{
    behaviour::{effective_options, Behaviour},
    db::migrate,
    mode::{self, Metadata, ModeEntry, OptionType, OptionValue, ShowWhen},
    read_pack::{read_pack_metadata, RecommendedMode},
    schedule::{QuietHours, ScheduleConfig, Window},
    user_config::{self, AppConfig, Capabilities, Key, Mode, Volume},
};
use tauri::{AppHandle, Manager};
use tempfile::NamedTempFile;
use uuid::Uuid;

// ─── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(tag = "type")]
pub enum ModeIdDto {
    Sandbox,
    Experience,
    Pack { id: u64 },
    File { path: String },
}

impl From<Mode> for ModeIdDto {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Sandbox => ModeIdDto::Sandbox,
            Mode::Experience => ModeIdDto::Experience,
            Mode::Pack { id } => ModeIdDto::Pack { id },
            Mode::File { path } => ModeIdDto::File {
                path: path.to_string_lossy().into_owned(),
            },
        }
    }
}

impl From<ModeIdDto> for Mode {
    fn from(dto: ModeIdDto) -> Self {
        match dto {
            ModeIdDto::Sandbox => Mode::Sandbox,
            ModeIdDto::Experience => Mode::Experience,
            ModeIdDto::Pack { id } => Mode::Pack { id },
            ModeIdDto::File { path } => Mode::File {
                path: PathBuf::from(path),
            },
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModeOptionsEntry {
    pub mode: ModeIdDto,
    pub options: HashMap<String, OptionValue>,
}

/// A `Mode::Experience` options entry, keyed by pack UUID (string form for JS-friendliness) --
/// see `AppConfig::experience_options`'s doc comment.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ExperienceOptionsEntry {
    pub pack_id: String,
    pub options: HashMap<String, OptionValue>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowDto {
    pub days: [bool; 7],
    pub start_hour: u32,
    pub start_minute: u32,
    pub duration_minutes: u32,
    pub jitter_minutes: u32,
}

impl From<Window> for WindowDto {
    fn from(w: Window) -> Self {
        WindowDto {
            days: w.days,
            start_hour: w.start_hour,
            start_minute: w.start_minute,
            duration_minutes: w.duration_minutes,
            jitter_minutes: w.jitter_minutes,
        }
    }
}

impl From<WindowDto> for Window {
    fn from(w: WindowDto) -> Self {
        Window {
            days: w.days,
            start_hour: w.start_hour,
            start_minute: w.start_minute,
            duration_minutes: w.duration_minutes,
            jitter_minutes: w.jitter_minutes,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QuietHoursDto {
    pub days: [bool; 7],
    pub start_hour: u32,
    pub start_minute: u32,
    pub end_hour: u32,
    pub end_minute: u32,
}

impl From<QuietHours> for QuietHoursDto {
    fn from(q: QuietHours) -> Self {
        QuietHoursDto {
            days: q.days,
            start_hour: q.start_hour,
            start_minute: q.start_minute,
            end_hour: q.end_hour,
            end_minute: q.end_minute,
        }
    }
}

impl From<QuietHoursDto> for QuietHours {
    fn from(q: QuietHoursDto) -> Self {
        QuietHours {
            days: q.days,
            start_hour: q.start_hour,
            start_minute: q.start_minute,
            end_hour: q.end_hour,
            end_minute: q.end_minute,
        }
    }
}

/// Mirrors `shared::schedule::ScheduleConfig` 1:1. Round-trips through the ordinary
/// `get_config`/`save_config` commands exactly like `capabilities`/`volume` do -- see `ConfigDto`'s
/// own doc comment on `schedule` for why this matters (it's load-bearing, not polish).
/// `enabled` also drives OS autostart-at-login registration one-to-one -- see
/// `set_schedule_enabled`, the only command that ever changes it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScheduleDto {
    pub enabled: bool,
    pub windows: Vec<WindowDto>,
    pub quiet_hours: Vec<QuietHoursDto>,
    pub grace_notification: bool,
}

impl From<ScheduleConfig> for ScheduleDto {
    fn from(s: ScheduleConfig) -> Self {
        ScheduleDto {
            enabled: s.enabled,
            windows: s.windows.into_iter().map(Into::into).collect(),
            quiet_hours: s.quiet_hours.into_iter().map(Into::into).collect(),
            grace_notification: s.grace_notification,
        }
    }
}

impl From<ScheduleDto> for ScheduleConfig {
    fn from(s: ScheduleDto) -> Self {
        ScheduleConfig {
            enabled: s.enabled,
            windows: s.windows.into_iter().map(Into::into).collect(),
            quiet_hours: s.quiet_hours.into_iter().map(Into::into).collect(),
            grace_notification: s.grace_notification,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigDto {
    pub pack_path: Option<String>,
    pub mode: ModeIdDto,
    pub mode_options: Vec<ModeOptionsEntry>,
    pub experience_options: Vec<ExperienceOptionsEntry>,
    pub panic_button: Key,
    pub disabled_monitors: Vec<String>,
    pub capabilities: Capabilities,
    pub volume: Volume,
    /// A normal `ConfigDto` field, round-tripping through `get_config`/`save_config` like every
    /// other setting here -- deliberately, not an oversight: `save_config` reconstructs a whole
    /// fresh `AppConfig` from whatever this DTO carries (that's already why `uploaded_modes` has
    /// to be manually preserved below), so if `schedule` weren't here, any unrelated save (e.g.
    /// `Permissions.svelte` flipping a capability) would silently reset the user's schedule back
    /// to `ScheduleConfig::default()`. The one exception is `enabled`: it also drives OS autostart
    /// registration and a resident-supervisor reload, so it's changed only via the dedicated
    /// `set_schedule_enabled` command, never through this generic save path.
    pub schedule: ScheduleDto,
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

        let experience_options = c
            .experience_options
            .into_iter()
            .map(|(pack_id, options)| ExperienceOptionsEntry {
                pack_id: pack_id.to_string(),
                options,
            })
            .collect();

        ConfigDto {
            pack_path: c.pack_path.and_then(|p| p.to_str().map(str::to_string)),
            mode: c.mode.into(),
            mode_options,
            experience_options,
            panic_button: c.panic_button,
            disabled_monitors: c.disabled_monitors,
            capabilities: c.capabilities,
            volume: c.volume,
            schedule: c.schedule.into(),
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

        let experience_options = dto
            .experience_options
            .into_iter()
            .filter_map(|e| Some((Uuid::parse_str(&e.pack_id).ok()?, e.options)))
            .collect();

        AppConfig {
            pack_path: dto.pack_path.map(PathBuf::from),
            uploaded_modes: Vec::new(),
            mode: dto.mode.into(),
            mode_options,
            experience_options,
            panic_button: dto.panic_button,
            disabled_monitors: dto.disabled_monitors,
            capabilities: dto.capabilities,
            volume: dto.volume,
            schedule: dto.schedule.into(),
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
    id: Uuid,
    modes: Vec<PackModeEntry>,
    /// The pack's behaviour.json, if it has one -- `Behaviour::new()` (empty) otherwise. Used
    /// only to synthesize the built-in default modes' content-group toggles (see
    /// `effective_entries_for_mode`); custom modes never consult this.
    behaviour: Behaviour,
    /// Which mode the pack author suggests -- see `pick_pack`'s preselection and
    /// `build_mode_groups`'s "(recommended)" marker.
    recommended_mode: Option<RecommendedMode>,
}

pub struct AppState {
    config: Mutex<AppConfig>,
    pack: Mutex<Option<LoadedPack>>,
    uploaded: Mutex<Vec<UploadedModeEntry>>,
    sandbox_mode: Metadata,
    experience_mode: Metadata,
}

pub type State<'a> = tauri::State<'a, AppState>;

// ─── Pack / mode loading ──────────────────────────────────────────────────────

fn load_pack(path: PathBuf) -> anyhow::Result<LoadedPack> {
    let mut file = std::fs::File::open(&path)?;
    let (header, metadata) = read_pack_metadata(&mut file)?;

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

    let behaviour_bytes: Option<Vec<u8>> = conn
        .query_row(
            "SELECT blob FROM pack_data WHERE name = 'behaviour'",
            [],
            |row| row.get("blob"),
        )
        .optional()?;
    let behaviour = behaviour_bytes
        .and_then(|bytes| Behaviour::from_json_bytes(&bytes).ok())
        .unwrap_or_default();

    Ok(LoadedPack {
        _db_file: db_file,
        id: header.id,
        modes,
        behaviour,
        recommended_mode: metadata.recommended_mode,
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
            .map(|m| ModeEntryDto {
                id: ModeIdDto::Pack { id: m.id },
                name: m.metadata.name.clone(),
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

        let entries = vec![ModeEntryDto {
            id: ModeIdDto::File { path: path_str },
            name: entry.metadata.name.clone(),
        }];

        groups.push(ModeGroupDto {
            label,
            source: "uploaded".into(),
            entries,
        });
    }

    // The pack author's recommendation nudges (not restricts) the choice -- see `pick_pack`'s
    // preselection and `behaviour-design/default-mode.md`'s "explicit choice, but nudged" UX.
    // Absent a pack, or an override to a custom mode, neither builtin entry is marked.
    let recommended = state
        .pack
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|pack| pack.recommended_mode.clone());

    groups.push(ModeGroupDto {
        label: "Default Modes".into(),
        source: "builtin".into(),
        entries: vec![
            ModeEntryDto {
                id: ModeIdDto::Sandbox,
                name: builtin_mode_label(
                    &state.sandbox_mode.name,
                    matches!(recommended, Some(RecommendedMode::Sandbox)),
                ),
            },
            ModeEntryDto {
                id: ModeIdDto::Experience,
                name: builtin_mode_label(
                    &state.experience_mode.name,
                    matches!(recommended, Some(RecommendedMode::Experience)),
                ),
            },
        ],
    });

    groups
}

fn builtin_mode_label(name: &str, recommended: bool) -> String {
    if recommended {
        format!("{name} (recommended)")
    } else {
        name.to_string()
    }
}

/// Resolves the entries a mode actually presents: for `Mode::Sandbox`/`Mode::Experience` (the
/// engine's two built-in default modes) with a pack loaded, the raw schema is passed through
/// `shared::behaviour::effective_options` alongside that pack's behaviour.json, synthesizing the
/// content-group checklist (see `behaviour-design/default-mode.md`, Ownership); every other case
/// (custom modes, or a default mode with no pack loaded) gets the schema's own entries
/// unchanged -- custom modes never see behaviour-derived toggles. Shared by
/// `get_mode_options_for` and `get_option_type_for_key` so both agree on what a key resolves to.
fn effective_entries_for_mode(
    mode: &Mode,
    state: &AppState,
) -> Option<IndexMap<String, ModeEntry>> {
    let mode_meta = match mode {
        Mode::Sandbox => Some(state.sandbox_mode.clone()),
        Mode::Experience => Some(state.experience_mode.clone()),
        Mode::Pack { id } => {
            let pack = state.pack.lock().unwrap();
            pack.as_ref()
                .and_then(|p| p.modes.iter().find(|m| m.id == *id))
                .map(|m| m.metadata.clone())
        }
        Mode::File { path } => {
            let uploaded = state.uploaded.lock().unwrap();
            uploaded
                .iter()
                .find(|u| &u.path == path)
                .map(|u| u.metadata.clone())
        }
    }?;

    if matches!(mode, Mode::Sandbox | Mode::Experience) {
        let behaviour = state
            .pack
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.behaviour.clone())
            .unwrap_or_default();
        Some(effective_options(&mode_meta, &behaviour).entries)
    } else {
        Some(mode_meta.entries)
    }
}

fn find_option_type(entries: &IndexMap<String, ModeEntry>, key: &str) -> Option<OptionType> {
    for (k, entry) in entries {
        match entry {
            ModeEntry::Option(opt) if k == key => return Some(opt.option_type.clone()),
            ModeEntry::Group(group) => {
                if let Some(t) = find_option_type(&group.entries, key) {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

/// Resolves the values a mode's stored options should read from: `Mode::Experience` is scoped
/// per pack (`AppConfig::experience_options`), everything else globally
/// (`AppConfig::mode_options`) -- see `behaviour-design/default-mode.md`, Ownership. No pack
/// loaded means no scope to read Experience options from, so it falls back to empty (schema
/// defaults), the same as any other mode with nothing stored yet.
fn stored_options_for(
    mode: &Mode,
    config: &AppConfig,
    state: &AppState,
) -> HashMap<String, OptionValue> {
    if matches!(mode, Mode::Experience) {
        let pack_id = state.pack.lock().unwrap().as_ref().map(|p| p.id);
        pack_id
            .and_then(|id| config.experience_options.get(&id).cloned())
            .unwrap_or_default()
    } else {
        config.mode_options.get(mode).cloned().unwrap_or_default()
    }
}

fn get_mode_options_for(config: &AppConfig, state: &AppState) -> Vec<OptionEntryDto> {
    let Some(entries) = effective_entries_for_mode(&config.mode, state) else {
        return Vec::new();
    };

    let stored = stored_options_for(&config.mode, config, state);

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

    build_entries(&entries, &stored)
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

    if matches!(mode, Mode::Experience) {
        let pack_id = state
            .pack
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.id)
            .ok_or_else(|| "no pack loaded".to_string())?;
        config
            .experience_options
            .entry(pack_id)
            .or_default()
            .insert(key, typed_value);
    } else {
        config
            .mode_options
            .entry(mode)
            .or_default()
            .insert(key, typed_value);
    }
    let uploaded = state.uploaded.lock().unwrap();
    save_to_disk(&config, &uploaded).map_err(|e| e.to_string())
}

fn get_option_type_for_key(
    _config: &AppConfig,
    mode: &Mode,
    key: &str,
    state: &AppState,
) -> Option<OptionType> {
    let entries = effective_entries_for_mode(mode, state)?;
    find_option_type(&entries, key)
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

    // Preselect a mode for the newly-picked pack, same "new pack resets mode selection"
    // precedent for both cases: an embedded pack mode wins if present; otherwise nudge toward
    // whichever built-in default mode the pack author recommends (falling back to Sandbox --
    // see `behaviour-design/default-mode.md`: "a plain content pack recommends Sandbox").
    let first_mode = loaded
        .modes
        .first()
        .map(|m| ModeIdDto::Pack { id: m.id })
        .or(Some(match loaded.recommended_mode {
            Some(RecommendedMode::Experience) => ModeIdDto::Experience,
            _ => ModeIdDto::Sandbox,
        }));

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
//
// The config app no longer owns the engine `Child` directly -- it talks to the resident
// supervisor over IPC (starting it on demand), which is now the sole owner of session
// lifecycle, wallpaper safety, and the panic key. See `design/scheduling.md`.

#[tauri::command]
async fn launch_lewdware() -> Result<(), String> {
    shared::ipc::ensure_supervisor_running()
        .await
        .map_err(|e| e.to_string())?;

    match shared::ipc::request(&shared::ipc::Request::StartSession {
        mode_path: None,
        dev: false,
    })
    .await
    .map_err(|e| e.to_string())?
    {
        shared::ipc::Response::Error { message } => Err(message),
        // `Busy` means a session is already running -- matches the old no-op-if-already-running
        // idempotency.
        shared::ipc::Response::Ok
        | shared::ipc::Response::Busy { .. }
        | shared::ipc::Response::Status(_) => Ok(()),
    }
}

#[tauri::command]
async fn stop_lewdware() -> Result<(), String> {
    // Best-effort: if no supervisor is reachable, there's nothing running to stop either --
    // matches the old code's no-op-safe handling of a possibly-`None` tracked `Child`.
    let _ = shared::ipc::request(&shared::ipc::Request::StopSession).await;
    Ok(())
}

#[derive(Serialize)]
pub struct EngineStatusDto {
    running: bool,
    /// Why the engine failed to start, if the last launch we made died before reaching a
    /// running state (e.g. a bad pack, a stale mode reference, a mode-script error), or why it
    /// stopped restarting after repeated crashes.
    error: Option<String>,
    /// A non-fatal issue noticed at startup (e.g. a mode built for an older API version). Only
    /// set while `running` is true.
    warning: Option<String>,
}

#[tauri::command]
async fn lewdware_running() -> EngineStatusDto {
    let Ok(shared::ipc::Response::Status(info)) =
        shared::ipc::request(&shared::ipc::Request::Status).await
    else {
        return EngineStatusDto {
            running: false,
            error: None,
            warning: None,
        };
    };

    let error = match &info.session {
        shared::ipc::SessionState::GaveUp { last_error, .. } => last_error
            .clone()
            .or_else(|| Some("Crashed repeatedly and was not restarted".to_string())),
        _ => info.last_exit.as_ref().and_then(|exit| exit.error.clone()),
    };

    EngineStatusDto {
        running: matches!(
            info.session,
            shared::ipc::SessionState::Starting
                | shared::ipc::SessionState::Running { .. }
                | shared::ipc::SessionState::RestartPending { .. }
        ),
        error,
        warning: info.warning,
    }
}

// ─── Scheduling ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ScheduleStatusDto {
    enabled: bool,
    /// RFC3339, or `null` if nothing's scheduled -- the frontend renders it via `Date`.
    next_session: Option<String>,
}

#[tauri::command]
async fn get_schedule_status() -> ScheduleStatusDto {
    let Ok(shared::ipc::Response::Status(info)) =
        shared::ipc::request(&shared::ipc::Request::Status).await
    else {
        return ScheduleStatusDto {
            enabled: false,
            next_session: None,
        };
    };

    ScheduleStatusDto {
        enabled: info.schedule.enabled,
        next_session: info.schedule.next_session.map(|t| t.to_rfc3339()),
    }
}

/// The single "enable scheduling" toggle: registers/deregisters the supervisor for OS
/// autostart-at-login (never a separate option -- the supervisor should just always be running
/// when scheduling is on), only persisting `schedule.enabled` once that OS call succeeds (it must
/// never claim a registration state that isn't actually true), then best-effort activates the
/// change in an already-running supervisor -- ensuring one is up (and reloading it) if enabling,
/// or just reloading a reachable one (never spawning one solely to tell it to stop) if disabling.
#[tauri::command]
async fn set_schedule_enabled(state: State<'_>, enabled: bool) -> Result<(), String> {
    let result = if enabled {
        shared::autostart::enable()
    } else {
        shared::autostart::disable()
    };
    result.map_err(|e| e.to_string())?;

    {
        let mut config = state.config.lock().unwrap();
        config.schedule.enabled = enabled;
        let uploaded = state.uploaded.lock().unwrap();
        save_to_disk(&config, &uploaded).map_err(|e| e.to_string())?;
    }

    if enabled {
        let _ = shared::ipc::ensure_supervisor_running().await;
    }
    let _ = shared::ipc::request(&shared::ipc::Request::ReloadConfig).await;

    Ok(())
}

/// Pings an already-saved schedule *content* change (windows/quiet-hours/grace-notification, not
/// `enabled` itself -- see `set_schedule_enabled`) through to a resident supervisor, if the
/// schedule is enabled (starting one on demand otherwise would just spawn a supervisor for
/// nothing). Called in addition to (never instead of) the ordinary `save_config`, which already
/// persisted the change; a failure here is best-effort and shouldn't undo that save.
#[tauri::command]
async fn reload_supervisor_schedule(state: State<'_>) -> Result<(), String> {
    let enabled = state.config.lock().unwrap().schedule.enabled;
    if !enabled {
        return Ok(());
    }

    shared::ipc::ensure_supervisor_running()
        .await
        .map_err(|e| e.to_string())?;

    match shared::ipc::request(&shared::ipc::Request::ReloadConfig)
        .await
        .map_err(|e| e.to_string())?
    {
        shared::ipc::Response::Error { message } => Err(message),
        _ => Ok(()),
    }
}

// ─── Input Monitoring (macOS) ─────────────────────────────────────────────────

#[tauri::command]
async fn input_monitoring_granted(#[allow(unused)] app_handle: AppHandle) -> Result<bool, String> {
    #[cfg(target_vendor = "apple")]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();

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
    let sandbox_mode_bytes = include_bytes!("../../../default-modes/sandbox/build/Sandbox.lwmode");
    let sandbox_mode = mode::read_mode_metadata(&mut Cursor::new(sandbox_mode_bytes))
        .expect("failed to load embedded Sandbox mode")
        .1;

    let experience_mode_bytes =
        include_bytes!("../../../default-modes/experience/build/Experience.lwmode");
    let experience_mode = mode::read_mode_metadata(&mut Cursor::new(experience_mode_bytes))
        .expect("failed to load embedded Experience mode")
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
            sandbox_mode,
            experience_mode,
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
            get_schedule_status,
            set_schedule_enabled,
            reload_supervisor_schedule,
            open_logs,
            check_for_update,
            input_monitoring_granted,
            request_input_monitoring,
            open_input_monitoring_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::behaviour::ContentGroup;

    fn empty_metadata(name: &str) -> Metadata {
        Metadata {
            name: name.to_string(),
            version: None,
            author: None,
            entrypoint: "main.lua".to_string(),
            entries: Default::default(),
            files: HashMap::new(),
        }
    }

    fn behaviour_with_one_content_group() -> Behaviour {
        Behaviour {
            content: shared::behaviour::Content {
                content_groups: vec![ContentGroup {
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

    fn loaded_pack(behaviour: Behaviour, modes: Vec<PackModeEntry>) -> LoadedPack {
        LoadedPack {
            _db_file: NamedTempFile::new().unwrap(),
            id: Uuid::new_v4(),
            modes,
            behaviour,
            recommended_mode: None,
        }
    }

    fn test_state(pack: Option<LoadedPack>) -> AppState {
        AppState {
            config: Mutex::new(AppConfig::default()),
            pack: Mutex::new(pack),
            uploaded: Mutex::new(Vec::new()),
            sandbox_mode: empty_metadata("Sandbox"),
            experience_mode: empty_metadata("Experience"),
        }
    }

    /// Recursively finds an entry by key -- mirrors how `PackMode.svelte` walks the tree.
    fn find_entry<'a>(entries: &'a [OptionEntryDto], key: &str) -> Option<&'a OptionEntryDto> {
        for entry in entries {
            match entry {
                OptionEntryDto::Option(opt) if opt.key == key => return Some(entry),
                OptionEntryDto::Group(group) if group.key == key => return Some(entry),
                OptionEntryDto::Group(group) => {
                    if let Some(found) = find_entry(&group.entries, key) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }

    #[test]
    fn default_mode_with_pack_content_group_renders_content_checklist() {
        let state = test_state(Some(loaded_pack(
            behaviour_with_one_content_group(),
            vec![],
        )));
        let config = AppConfig {
            mode: Mode::Sandbox,
            ..Default::default()
        };

        let entries = get_mode_options_for(&config, &state);

        let content_group_entry = find_entry(&entries, "content_groups")
            .expect("synthesized \"content_groups\" group should be present");
        let OptionEntryDto::Group(group) = content_group_entry else {
            panic!("expected a group entry");
        };
        let kinky = find_entry(&group.entries, "content_group.kinky")
            .expect("synthesized content_group.kinky option should be present");
        assert!(matches!(kinky, OptionEntryDto::Option(_)));
    }

    /// The actual invariant under test: a custom mode embedded in the same pack never sees the
    /// content-group toggle, even though the pack's behaviour.json declares one (`behaviour-
    /// design/default-mode.md`, Ownership: "the toggle is only shown where it is honored").
    #[test]
    fn custom_pack_mode_never_renders_content_checklist() {
        let mode_id = 1;
        let state = test_state(Some(loaded_pack(
            behaviour_with_one_content_group(),
            vec![PackModeEntry {
                id: mode_id,
                metadata: empty_metadata("Custom mode"),
            }],
        )));
        let config = AppConfig {
            mode: Mode::Pack { id: mode_id },
            ..Default::default()
        };

        let entries = get_mode_options_for(&config, &state);

        assert!(find_entry(&entries, "content_groups").is_none());
    }

    /// `get_option_type_for_key` previously only searched the mode's own raw schema, so it
    /// couldn't resolve a synthesized `content_group.*` key's type (silently papered over by
    /// `coerce_option_value`'s untagged-deserialize fallback for booleans specifically). Confirm
    /// it now resolves correctly via `effective_entries_for_mode`.
    #[test]
    fn content_group_key_resolves_its_option_type_for_default_mode() {
        let state = test_state(Some(loaded_pack(
            behaviour_with_one_content_group(),
            vec![],
        )));

        let opt_type = get_option_type_for_key(
            &AppConfig::default(),
            &Mode::Sandbox,
            "content_group.kinky",
            &state,
        );

        assert_eq!(opt_type, Some(OptionType::Boolean { default: true }));
    }

    /// No pack loaded at all -- `Mode::Sandbox` shouldn't panic, and (with nothing to
    /// synthesize) shouldn't render a content checklist.
    #[test]
    fn default_mode_with_no_pack_loaded_has_no_content_checklist() {
        let state = test_state(None);
        let config = AppConfig {
            mode: Mode::Sandbox,
            ..Default::default()
        };

        let entries = get_mode_options_for(&config, &state);

        assert!(find_entry(&entries, "content_groups").is_none());
    }
}
