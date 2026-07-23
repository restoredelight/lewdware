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
    behaviour::{Behaviour, effective_options},
    db::migrate,
    mode::{self, Metadata, ModeEntry, OptionType, OptionValue, Permission, ShowWhen},
    read_pack::{read_pack_metadata, RecommendedMode},
    schedule::{QuietHours, ScheduleConfig, Window},
    user_config::{self, AppConfig, Capabilities, Key, Mode, Volume, WallpaperConfig},
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
    /// Carried here for the same reason `schedule` is: `save_config` rebuilds a whole fresh
    /// `AppConfig` from this DTO, so a field that isn't round-tripped is silently reset. Leaving it
    /// out would wipe the user's chosen restore image on any unrelated save.
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
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
            wallpaper: c.wallpaper,
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
            wallpaper: dto.wallpaper,
            volume: dto.volume,
            schedule: dto.schedule.into(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MonitorDto {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
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
    /// Permissions this option says it uses. Whether the requirement is *live* -- the option
    /// visible under `show_when` and, for a boolean/optional, switched on -- is decided in
    /// `PackMode.svelte`, which is where current values and `show_when` are already evaluated.
    pub needs_permissions: Vec<Permission>,
}

#[derive(Serialize, Clone, Debug)]
pub struct OptionGroupDto {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub show_when: Option<ShowWhen>,
    /// See `ModeOptionDto::needs_permissions`. A group's requirement is live when the group is visible and
    /// at least one option inside it is.
    pub needs_permissions: Vec<Permission>,
    pub entries: Vec<OptionEntryDto>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind")]
pub enum OptionEntryDto {
    Option(ModeOptionDto),
    Group(OptionGroupDto),
}

/// What `get_mode_options` returns: the selected mode's option tree, plus the permissions the
/// mode uses unconditionally (`Metadata::needs_permissions`), which belong to no single option and so have
/// nowhere in the tree to hang off.
#[derive(Serialize, Clone, Debug)]
pub struct ModeOptionsDto {
    pub needs_permissions: Vec<Permission>,
    pub entries: Vec<OptionEntryDto>,
    /// Pack-derived facts (`pack_has_web_links`, etc.) a mode option's `show_when` can reference.
    /// They are not options -- no value is stored for them -- but the UI needs them alongside the
    /// live option values to evaluate visibility. A default mode reports every fact (all false when
    /// no pack is loaded); custom modes, which never consult behaviour data, get an empty map. See
    /// `shared::behaviour::EffectiveSchema::pack_has`.
    pub pack_has: IndexMap<String, OptionValue>,
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
///
/// Returns the mode's unconditional `needs_permissions` alongside them: it comes off the same `Metadata`,
/// and every caller that wants one generally wants the other. Also returns the pack-derived
/// `pack_has_*` facts that drive `show_when` visibility -- a default mode reports every fact (all
/// false with no pack loaded), custom modes get an empty map.
fn effective_entries_for_mode(
    mode: &Mode,
    state: &AppState,
) -> Option<(
    IndexMap<String, ModeEntry>,
    Vec<Permission>,
    IndexMap<String, OptionValue>,
)> {
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

    let needs_permissions = mode_meta.needs_permissions.clone();

    if matches!(mode, Mode::Sandbox | Mode::Experience) {
        let behaviour = state
            .pack
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| p.behaviour.clone())
            .unwrap_or_default();
        let schema = effective_options(&mode_meta, &behaviour);
        Some((schema.entries, needs_permissions, schema.pack_has))
    } else {
        Some((mode_meta.entries, needs_permissions, IndexMap::new()))
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

fn get_mode_options_for(config: &AppConfig, state: &AppState) -> ModeOptionsDto {
    let Some((entries, needs_permissions, pack_has)) = effective_entries_for_mode(&config.mode, state)
    else {
        return ModeOptionsDto {
            needs_permissions: Vec::new(),
            entries: Vec::new(),
            pack_has: IndexMap::new(),
        };
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
                        needs_permissions: opt.needs_permissions.clone(),
                    })
                }
                ModeEntry::Group(group) => OptionEntryDto::Group(OptionGroupDto {
                    key: key.clone(),
                    label: group.label.clone(),
                    description: group.description.clone(),
                    show_when: group.show_when.clone(),
                    needs_permissions: group.needs_permissions.clone(),
                    entries: build_entries(&group.entries, stored),
                }),
            })
            .collect()
    }

    ModeOptionsDto {
        needs_permissions,
        entries: build_entries(&entries, &stored),
        pack_has,
    }
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

/// Lists monitors by asking the engine, rather than reading them from this process.
///
/// This app is a native Wayland Tauri app; the engine forces winit onto XWayland because Wayland
/// can't position windows. The two disagree about both monitor names and geometry (see
/// `shared::monitor`), so anything measured here would be written into `disabled_monitors` and
/// then never match what the engine compares it against -- which is exactly the bug this replaces.
///
/// Deliberately no fallback to `app_handle.available_monitors()`: silently wrong identities are
/// what made disabling a monitor a no-op, and an error the user can see beats a control that
/// quietly does nothing.
#[tauri::command]
async fn get_monitors(state: State<'_>) -> Result<Vec<MonitorDto>, String> {
    let disabled = state.config.lock().unwrap().disabled_monitors.clone();

    let mut command = tokio::process::Command::from(
        shared::child::find_engine_binary()
            .ok_or_else(|| "could not find the lewdware-engine binary".to_string())?,
    );
    command
        .arg(shared::monitor::LIST_MONITORS_FLAG)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    // The probe opens an event loop, so cap it rather than letting a wedged display server hang
    // the settings window.
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| "timed out asking the engine for the monitor list".to_string())?
        .map_err(|e| format!("could not run the engine to list monitors: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "the engine could not list monitors: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let listed: Vec<shared::monitor::MonitorInfo> = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("could not read the engine's monitor list: {e}"))?;

    let mut monitors: Vec<_> = listed
        .into_iter()
        .map(|monitor| MonitorDto {
            disabled: disabled.contains(&monitor.id),
            id: monitor.id,
            name: monitor.name,
            width: monitor.width,
            height: monitor.height,
            primary: monitor.primary,
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
fn get_mode_options(state: State<'_>) -> ModeOptionsDto {
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
    let (entries, _, _) = effective_entries_for_mode(mode, state)?;
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

/// Whether this desktop can have its wallpaper put back after a pack changes it.
///
/// Read once when the Permissions page mounts. It runs a real snapshot (a `dbus-send` on KDE, a
/// `gsettings` read on GNOME), which is why it isn't polled -- the answer only changes if the user
/// switches desktop session, at which point the page is being re-opened anyway.
#[tauri::command]
async fn wallpaper_support() -> Result<WallpaperSupportDto, String> {
    let snapshot = tokio::task::spawn_blocking(|| shared::wallpaper::snapshot(None))
        .await
        .map_err(|e| e.to_string())?;

    Ok(WallpaperSupportDto {
        can_restore_original: snapshot.is_restorable(),
    })
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WallpaperSupportDto {
    /// `false` means the user has to nominate an image to restore to, or wallpaper changes stay
    /// off. See `shared::wallpaper::Snapshot::is_restorable`.
    pub can_restore_original: bool,
}

/// The restore image, as a `data:` URL for `<img src>`.
///
/// Inlined rather than served over Tauri's asset protocol so there is no protocol scope or CSP to
/// configure -- this is one small image on one settings page, shown once.
#[tauri::command]
async fn wallpaper_restore_preview(path: String) -> Result<Option<String>, String> {
    let preview = tokio::task::spawn_blocking(move || -> Option<String> {
        use base64::Engine;

        let bytes = std::fs::read(&path).ok()?;
        // The file is whatever the user picked, so guess the type from its extension rather than
        // assuming PNG.
        let mime = match std::path::Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            Some("bmp") => "image/bmp",
            _ => "image/png",
        };

        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Some(format!("data:{mime};base64,{encoded}"))
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(preview)
}

/// Prompts for an image and adopts it as the restore target.
///
/// The file is copied into app data rather than referenced where it sits: a restore image that the
/// user later deletes or moves would fail exactly when it is needed, stranding the wallpaper the
/// setting exists to protect.
#[tauri::command]
async fn pick_restore_image(app_handle: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let picked = app_handle
        .dialog()
        .file()
        .add_filter("Image", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());

    let Some(picked) = picked else {
        return Ok(None);
    };

    let adopted = tokio::task::spawn_blocking(move || -> anyhow::Result<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| anyhow::anyhow!("could not locate the local data directory"))?
            .join("lewdware");
        std::fs::create_dir_all(&dir)?;

        let extension = picked
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let destination = dir.join(format!("restore-wallpaper.{extension}"));

        // Copying onto the previous choice would leave a stale file behind whenever the extension
        // changes, and the config would still point at the old one.
        for stale in ["png", "jpg", "jpeg", "webp", "bmp", "gif"] {
            let stale = dir.join(format!("restore-wallpaper.{stale}"));
            if stale != destination {
                let _ = std::fs::remove_file(stale);
            }
        }

        std::fs::copy(&picked, &destination)?;
        Ok(destination)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    Ok(Some(adopted.to_string_lossy().into_owned()))
}

/// The bundled near-black placeholder, materialised on disk.
///
/// Offered as the starting point so the restore image is never left unset once the user opts in.
#[tauri::command]
async fn default_restore_image() -> Result<String, String> {
    tokio::task::spawn_blocking(shared::wallpaper::default_restore_image)
        .await
        .map_err(|e| e.to_string())?
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
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

#[derive(Serialize, Clone)]
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

impl EngineStatusDto {
    fn stopped() -> Self {
        Self {
            running: false,
            error: None,
            warning: None,
        }
    }
}

fn engine_status_from(info: &shared::ipc::StatusInfo) -> EngineStatusDto {
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
        warning: info.warning.clone(),
    }
}

#[tauri::command]
async fn lewdware_running() -> EngineStatusDto {
    match shared::ipc::request(&shared::ipc::Request::Status).await {
        Ok(shared::ipc::Response::Status(info)) => engine_status_from(&info),
        _ => EngineStatusDto::stopped(),
    }
}

/// Pushed to the webview as the `supervisor:status` event whenever the supervisor's state
/// changes (and once with a "stopped" payload when it goes away).
#[derive(Serialize, Clone)]
pub struct SupervisorStatusDto {
    engine: EngineStatusDto,
    schedule: ScheduleStatusDto,
}

/// Follows the supervisor's status stream for the lifetime of the app, forwarding every update
/// to the webview. When no supervisor is reachable (it self-terminates when idle), retries
/// quietly — the connect attempt against a local socket is cheap.
async fn forward_supervisor_status(app: tauri::AppHandle) {
    use tauri::Emitter;
    loop {
        if let Ok(mut subscription) = shared::ipc::subscribe().await {
            while let Ok(info) = subscription.next().await {
                let _ = app.emit(
                    "supervisor:status",
                    SupervisorStatusDto {
                        engine: engine_status_from(&info),
                        schedule: schedule_status_from(&info),
                    },
                );
            }
        }
        // No stream (or it just ended). That usually means the supervisor exited — but a
        // supervisor predating `Subscribe` drops the stream while still answering one-shot
        // requests, so confirm with `Status` before reporting stopped. Against such a
        // supervisor this loop degrades into accurate 2s polling instead of lying.
        let payload = match shared::ipc::request(&shared::ipc::Request::Status).await {
            Ok(shared::ipc::Response::Status(info)) => SupervisorStatusDto {
                engine: engine_status_from(&info),
                schedule: schedule_status_from(&info),
            },
            _ => SupervisorStatusDto {
                engine: EngineStatusDto::stopped(),
                schedule: ScheduleStatusDto {
                    enabled: false,
                    next_session: None,
                },
            },
        };
        let _ = app.emit("supervisor:status", payload);
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

// ─── Scheduling ────────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct ScheduleStatusDto {
    enabled: bool,
    /// RFC3339, or `null` if nothing's scheduled -- the frontend renders it via `Date`.
    next_session: Option<String>,
}

fn schedule_status_from(info: &shared::ipc::StatusInfo) -> ScheduleStatusDto {
    ScheduleStatusDto {
        enabled: info.schedule.enabled,
        next_session: info.schedule.next_session.map(|t| t.to_rfc3339()),
    }
}

#[tauri::command]
async fn get_schedule_status() -> ScheduleStatusDto {
    match shared::ipc::request(&shared::ipc::Request::Status).await {
        Ok(shared::ipc::Response::Status(info)) => schedule_status_from(&info),
        _ => ScheduleStatusDto {
            enabled: false,
            next_session: None,
        },
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
            tauri::async_runtime::spawn(forward_supervisor_status(app.handle().clone()));
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
            wallpaper_support,
            wallpaper_restore_preview,
            pick_restore_image,
            default_restore_image,
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
            needs_permissions: Vec::new(),
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

    /// `needs_permissions` is only useful if it survives the trip from the mode's schema to the DTO the UI
    /// reads -- both the per-entry declarations and the mode-wide one, which hangs off no option
    /// and so travels beside the tree rather than in it.
    #[test]
    fn declared_permissions_reach_the_dto() {
        let mut sandbox = empty_metadata("Sandbox");
        sandbox.needs_permissions = vec![Permission::SendNotifications];
        sandbox.entries.insert(
            "links".to_string(),
            ModeEntry::Group(mode::ModeGroup {
                label: "Web links".to_string(),
                description: None,
                show_when: None,
                needs_permissions: vec![Permission::OpenLinks],
                entries: IndexMap::from([(
                    "wallpaper_enabled".to_string(),
                    ModeEntry::Option(mode::ModeOption {
                        label: "Change wallpaper".to_string(),
                        description: None,
                        option_type: OptionType::Boolean { default: true },
                        optional: false,
                        enabled_by_default: false,
                        show_when: None,
                        needs_permissions: vec![Permission::SetWallpaper],
                    }),
                )]),
            }),
        );

        let mut state = test_state(None);
        state.sandbox_mode = sandbox;

        let config = AppConfig {
            mode: Mode::Sandbox,
            ..Default::default()
        };
        let dto = get_mode_options_for(&config, &state);

        assert_eq!(dto.needs_permissions, vec![Permission::SendNotifications]);

        let OptionEntryDto::Group(group) = find_entry(&dto.entries, "links").unwrap() else {
            panic!("expected a group entry");
        };
        assert_eq!(group.needs_permissions, vec![Permission::OpenLinks]);

        let OptionEntryDto::Option(opt) = find_entry(&dto.entries, "wallpaper_enabled").unwrap()
        else {
            panic!("expected an option entry");
        };
        assert_eq!(opt.needs_permissions, vec![Permission::SetWallpaper]);
    }

    /// A mode option's `show_when` can key off pack-derived facts (`pack_has_web_links`, etc.), so
    /// those facts have to survive the trip to the DTO -- the UI evaluates visibility against them
    /// alongside the live option values. Without them, any option gated on a `pack_has_*` fact
    /// silently never renders.
    #[test]
    fn pack_has_facts_reach_the_dto_for_default_mode() {
        use shared::behaviour::WebLink;

        let behaviour = Behaviour {
            content: shared::behaviour::Content {
                web_links: vec![WebLink {
                    url: "https://example.com".to_string(),
                    args: Vec::new(),
                    tags: Vec::new(),
                }],
                ..Default::default()
            },
            ..Behaviour::new()
        };
        let state = test_state(Some(loaded_pack(behaviour, vec![])));
        let config = AppConfig {
            mode: Mode::Sandbox,
            ..Default::default()
        };

        let dto = get_mode_options_for(&config, &state);

        assert_eq!(
            dto.pack_has.get("pack_has_web_links"),
            Some(&OptionValue::Boolean(true)),
        );
        assert_eq!(
            dto.pack_has.get("pack_has_prompts"),
            Some(&OptionValue::Boolean(false)),
        );
    }

    /// A default mode with no pack loaded still reports every fact (as false) rather than an empty
    /// map, so an option gated on `pack_has_web_links: true` correctly resolves to hidden. Custom
    /// modes, which never consult behaviour data, get the empty map instead.
    #[test]
    fn pack_has_facts_all_false_for_default_mode_without_a_pack() {
        let state = test_state(None);
        let config = AppConfig {
            mode: Mode::Sandbox,
            ..Default::default()
        };

        let dto = get_mode_options_for(&config, &state);

        assert_eq!(
            dto.pack_has.get("pack_has_web_links"),
            Some(&OptionValue::Boolean(false)),
        );
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

        let entries = get_mode_options_for(&config, &state).entries;

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

        let entries = get_mode_options_for(&config, &state).entries;

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

        let entries = get_mode_options_for(&config, &state).entries;

        assert!(find_entry(&entries, "content_groups").is_none());
    }
}
