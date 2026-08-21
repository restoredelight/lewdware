use std::{collections::HashMap, path::PathBuf};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use shared::{
    mode::{OptionType, OptionValue, Permission, ShowWhen},
    monitor::MonitorRegion,
    schedule::{QuietHours, Rule, ScheduleConfig, SessionLength, SessionOverrides, Trigger},
    user_config::{AppConfig, AudioDeviceChoice, Capabilities, Key, Mode, Volume, WallpaperConfig},
};
use uuid::Uuid;

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

/// The v2 schedule types are reused here rather than mirrored, unlike v1's `WindowDto`. The
/// mirror only ever existed to keep `PathBuf` off the wire and to pin the frontend's contract;
/// `Rule`'s trigger, length and quiet hours carry no paths and serialise as tagged unions the
/// frontend can switch on directly, so a copy of each would be duplication rather than insulation.
/// `SessionOverrides` is the one part that does hold paths, so it keeps a DTO.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SessionOverridesDto {
    #[serde(default)]
    pub mode: Option<ModeIdDto>,
    #[serde(default)]
    pub pack_path: Option<String>,
}

impl From<SessionOverrides> for SessionOverridesDto {
    fn from(o: SessionOverrides) -> Self {
        SessionOverridesDto {
            mode: o.mode.map(Into::into),
            pack_path: o.pack_path.map(|p| p.to_string_lossy().into_owned()),
        }
    }
}

impl From<SessionOverridesDto> for SessionOverrides {
    fn from(o: SessionOverridesDto) -> Self {
        SessionOverrides {
            mode: o.mode.map(Into::into),
            pack_path: o.pack_path.map(PathBuf::from),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RuleDto {
    /// Stable across list edits, so the supervisor's budget counters survive one. Serialises as a
    /// plain string; the frontend treats it as an opaque key.
    pub id: Uuid,
    pub days: [bool; 7],
    pub trigger: Trigger,
    pub length: SessionLength,
    #[serde(default)]
    pub overrides: SessionOverridesDto,
}

impl From<Rule> for RuleDto {
    fn from(r: Rule) -> Self {
        RuleDto {
            id: r.id,
            days: r.days,
            trigger: r.trigger,
            length: r.length,
            overrides: r.overrides.into(),
        }
    }
}

impl From<RuleDto> for Rule {
    fn from(r: RuleDto) -> Self {
        Rule {
            id: r.id,
            days: r.days,
            trigger: r.trigger,
            length: r.length,
            overrides: r.overrides.into(),
        }
    }
}

/// A frontend that predates these fields must not silently reset them, for the same reason
/// `default_theme_dto` exists: `save_config` rebuilds a whole `ScheduleConfig` from this DTO, so
/// an absent field would be written back as whatever `Default` says rather than what the user set.
fn default_cooldown_minutes() -> u32 {
    ScheduleConfig::default().cooldown_minutes
}

fn default_panic_cooldown_minutes() -> u32 {
    ScheduleConfig::default().panic_cooldown_minutes
}

/// Mirrors `shared::schedule::ScheduleConfig` 1:1. Round-trips through the ordinary
/// `get_config`/`save_config` commands exactly like `capabilities`/`volume` do -- see `ConfigDto`'s
/// own doc comment on `schedule` for why this matters (it's load-bearing, not polish).
/// `enabled` also drives OS autostart-at-login registration one-to-one -- see
/// `set_schedule_enabled`, the only command that ever changes it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScheduleDto {
    pub enabled: bool,
    pub rules: Vec<RuleDto>,
    pub quiet_hours: Vec<QuietHours>,
    pub grace_notification: bool,
    #[serde(default = "default_cooldown_minutes")]
    pub cooldown_minutes: u32,
    #[serde(default = "default_panic_cooldown_minutes")]
    pub panic_cooldown_minutes: u32,
}

impl From<ScheduleConfig> for ScheduleDto {
    fn from(s: ScheduleConfig) -> Self {
        ScheduleDto {
            enabled: s.enabled,
            rules: s.rules.into_iter().map(Into::into).collect(),
            quiet_hours: s.quiet_hours,
            grace_notification: s.grace_notification,
            cooldown_minutes: s.cooldown_minutes,
            panic_cooldown_minutes: s.panic_cooldown_minutes,
        }
    }
}

impl From<ScheduleDto> for ScheduleConfig {
    fn from(s: ScheduleDto) -> Self {
        ScheduleConfig {
            enabled: s.enabled,
            rules: s.rules.into_iter().map(Into::into).collect(),
            quiet_hours: s.quiet_hours,
            grace_notification: s.grace_notification,
            cooldown_minutes: s.cooldown_minutes,
            panic_cooldown_minutes: s.panic_cooldown_minutes,
        }
    }
}

/// A frontend that predates these fields (or a partial payload) must not silently reset the
/// user's look, so both mirror `AppConfig`'s own defaults rather than falling back to `String`'s.
pub fn default_theme_dto() -> String {
    AppConfig::default().theme
}

pub fn default_appearance_dto() -> String {
    AppConfig::default().appearance
}

/// The settings the **frontend** owns, as it sends them back.
///
/// Deliberately not the whole of an `AppConfig`. A field belongs here only if the frontend is the
/// thing that changes it; anything a backend command owns instead (`mode_options` via
/// `set_mode_option`, `uploaded_modes` via `upload_mode`) is left out and carried across by
/// [`apply_config_dto`]. The frontend's config is a *snapshot* taken when the page loaded, so a
/// backend-owned field sent back through here would arrive holding whatever was true then —
/// undoing every change made since. Leaving it out of the DTO is what makes that unrepresentable.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConfigDto {
    pub pack_path: Option<String>,
    pub mode: ModeIdDto,
    /// The window look every popup is drawn in, unless the running mode names one itself. See
    /// `AppConfig::theme` for why this is the user's setting rather than a mode option. Both this
    /// and `appearance` round-trip like every other field here, for the reason `wallpaper`
    /// documents below.
    #[serde(default = "default_theme_dto")]
    pub theme: String,
    #[serde(default = "default_appearance_dto")]
    pub appearance: String,
    pub panic_button: Key,
    pub disabled_monitors: Vec<String>,
    /// Round-tripped for the same reason `wallpaper` and `schedule` are: `save_config` rebuilds a
    /// whole `AppConfig` from this DTO, so a region left out here would be wiped by any unrelated
    /// save.
    #[serde(default)]
    pub monitor_regions: HashMap<String, MonitorRegion>,
    pub capabilities: Capabilities,
    /// Carried here for the same reason `schedule` is: `save_config` rebuilds a whole fresh
    /// `AppConfig` from this DTO, so a field that isn't round-tripped is silently reset. Leaving it
    /// out would wipe the user's chosen restore image on any unrelated save.
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
    pub volume: Volume,
    /// The chosen audio output; `None` for the system default. See `AppConfig::audio_device`.
    /// `#[serde(default)]` and round-tripped for the same reason as the fields above.
    #[serde(default)]
    pub audio_device: Option<AudioDeviceChoice>,
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
        ConfigDto {
            pack_path: c.pack_path.and_then(|p| p.to_str().map(str::to_string)),
            mode: c.mode.into(),
            theme: c.theme,
            appearance: c.appearance,
            panic_button: c.panic_button,
            disabled_monitors: c.disabled_monitors,
            monitor_regions: c.monitor_regions,
            capabilities: c.capabilities,
            wallpaper: c.wallpaper,
            volume: c.volume,
            audio_device: c.audio_device,
            schedule: c.schedule.into(),
        }
    }
}

impl From<ConfigDto> for AppConfig {
    /// Backend-owned fields come out empty here and are filled in by [`apply_config_dto`], which
    /// is the only thing that should build an `AppConfig` from a DTO.
    fn from(dto: ConfigDto) -> Self {
        AppConfig {
            pack_path: dto.pack_path.map(PathBuf::from),
            uploaded_modes: Vec::new(),
            mode: dto.mode.into(),
            mode_options: HashMap::new(),
            experience_options: HashMap::new(),
            theme: dto.theme,
            appearance: dto.appearance,
            panic_button: dto.panic_button,
            disabled_monitors: dto.disabled_monitors,
            monitor_regions: dto.monitor_regions,
            capabilities: dto.capabilities,
            wallpaper: dto.wallpaper,
            volume: dto.volume,
            audio_device: dto.audio_device,
            schedule: dto.schedule.into(),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct ThemeCatalogueDto {
    pub themes: Vec<ThemeEntryDto>,
    pub appearances: Vec<shared::theme::AppearanceInfo>,
    /// The XDG portal's current answer, used when the picker previews `auto`. `None` means the
    /// desktop expressed no preference or could not be queried, matching the engine's light
    /// fallback.
    pub system_appearance: Option<shared::theme::Appearance>,
}

/// One selectable look, with everything needed to *draw* it in the picker.
///
/// The frontend renders a small live window from this — border, title bar, buttons, a text field —
/// so a user can see and poke a theme before choosing it. It is the same data the engine paints
/// with (`shared::theme`), not a description of it written twice.
#[derive(Serialize, Clone, Debug)]
pub struct ThemeEntryDto {
    /// The value stored in `AppConfig::theme`. An alias keeps its own name here — picking the
    /// merged card means "follow this machine", not "pin whatever it happens to be today".
    pub name: &'static str,
    /// What to call it in the picker. An alias borrows the label of whatever it resolves to
    /// *here*, because that is what the user is actually looking at.
    pub label: &'static str,
    /// Whether this look has a dark palette, answered for the look that will really be drawn —
    /// so an alias reports its resolution's answer rather than the catalogue's placeholder.
    pub supports_dark: bool,
    /// True for `native`/`native-retro`. The card says so, since the *label* no longer does.
    pub matches_system: bool,
    /// The concrete look an alias stands for on this machine, so a config that pins that look
    /// directly still lights up the merged card.
    pub resolves_to: Option<&'static str>,
    /// Both palettes up front: the preview redraws when the user flips light/dark, and there are
    /// only ten themes, so fetching one and asking for the other later buys nothing.
    pub light: ThemeLookDto,
    pub dark: ThemeLookDto,
}

#[derive(Serialize, Clone, Debug)]
pub struct ThemeLookDto {
    pub metrics: shared::theme::Metrics,
    pub chrome: shared::theme::Chrome,
    pub widgets: shared::theme::Widgets,
}

pub fn theme_look(
    theme: shared::theme::Theme,
    appearance: shared::theme::Appearance,
) -> ThemeLookDto {
    ThemeLookDto {
        metrics: theme.metrics(),
        chrome: theme.chrome(appearance),
        widgets: *theme.widgets(appearance),
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
    /// Desktop-space position, in the same physical pixels as `width`/`height`, so the picker can
    /// draw the monitors in their real arrangement rather than in a row.
    pub x: i32,
    pub y: i32,
    /// `width`/`height` are physical; a region resolves against *logical* pixels. Carried so the
    /// picker can show a region's size in the numbers a mode would actually see.
    pub scale_factor: f64,
    /// The area of this monitor popups may use. Whole monitor unless the user has narrowed it.
    pub region: MonitorRegion,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModeEntryDto {
    pub id: ModeIdDto,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PackMetadataDto {
    pub name: String,
    pub creator: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
}

impl From<&shared::pack::Metadata> for PackMetadataDto {
    fn from(metadata: &shared::pack::Metadata) -> Self {
        Self {
            name: metadata.name.clone(),
            creator: metadata.creator.clone(),
            description: metadata.description.clone(),
            version: metadata.version.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModeGroupDto {
    pub label: String,
    pub source: String,
    pub entries: Vec<ModeEntryDto>,
}

/// Outbound only -- `Serialize`, like the `OptionGroupDto` it sits alongside. Its `value` is
/// a resolved `OptionValue`, which deliberately cannot be deserialized: a value coming back
/// *in* from the frontend is a `StoredValue` (see `set_mode_option`).
#[derive(Serialize, Clone, Debug)]
pub struct ModeOptionDto {
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub suffix: Option<String>,
    pub logarithmic: bool,
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
