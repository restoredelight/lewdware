use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use uuid::Uuid;

use crate::mode::StoredValue;
use crate::schedule::ScheduleConfig;
use crate::theme::{AppearanceChoice, ThemeChoice};

pub use crate::theme::{AppearanceChoice as ConfigAppearance, ThemeChoice as ConfigTheme};

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppConfig {
    pub pack_path: Option<PathBuf>,
    pub uploaded_modes: Vec<PathBuf>,
    pub mode: Mode,
    #[serde_as(as = "Vec<(_, _)>")]
    pub mode_options: HashMap<Mode, HashMap<String, StoredValue>>,
    #[serde(default)]
    pub theme: ThemeChoice,
    #[serde(default)]
    pub appearance: AppearanceChoice,
    pub stop_button: Key,
    pub disabled_monitors: Vec<String>,
    #[serde(default)]
    pub monitor_regions: HashMap<String, crate::monitor::MonitorRegion>,
    pub permissions: Permissions,
    #[serde(default)]
    pub wallpaper: WallpaperConfig,
    pub volume: Volume,
    #[serde(default)]
    pub audio_device: Option<AudioDeviceChoice>,
    #[serde(default)]
    pub schedule: ScheduleConfig,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub set_wallpaper: bool,
    pub open_links: bool,
    pub send_notifications: bool,
}

impl Permissions {
    /// Whether a permission a mode declares in its schema is currently granted.
    pub fn allows(&self, permission: crate::mode::Permission) -> bool {
        match permission {
            crate::mode::Permission::SetWallpaper => self.set_wallpaper,
            crate::mode::Permission::OpenLinks => self.open_links,
            crate::mode::Permission::SendNotifications => self.send_notifications,
        }
    }
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            set_wallpaper: true,
            open_links: false,
            send_notifications: true,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WallpaperRestore {
    /// Whatever it was before.
    #[default]
    Original,
    /// A fixed image, for desktops that cannot. This is what makes wallpaper changes possible at
    /// all on bare compositors -- without it the engine refuses, rather than making a change it
    /// could never undo.
    Image { path: PathBuf },
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
pub struct WallpaperConfig {
    #[serde(default)]
    pub restore: WallpaperRestore,
}

impl WallpaperConfig {
    pub fn fallback_image(&self) -> Option<&std::path::Path> {
        match &self.restore {
            WallpaperRestore::Original => None,
            WallpaperRestore::Image { path } => Some(path),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Volume {
    pub video: f32,
    pub audio: f32,
}

impl Default for Volume {
    fn default() -> Self {
        Self {
            video: 1.0,
            audio: 1.0,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct AudioDeviceChoice {
    /// `cpal::DeviceId`'s `Display` form
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Default)]
pub enum Mode {
    #[default]
    Sandbox,
    Experience {
        pack: Option<Uuid>,
    },
    Pack {
        id: u64,
    },
    File {
        path: PathBuf,
    },
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Key {
    pub name: String,
    pub code: String,
    pub modifiers: Modifiers,
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.modifiers.ctrl {
            parts.push("Ctrl");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        if self.modifiers.meta {
            parts.push("Meta");
        }
        parts.push(&self.name);
        write!(f, "{}", parts.join(" + "))
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            pack_path: None,
            uploaded_modes: Vec::new(),
            mode: Mode::default(),
            mode_options: HashMap::new(),
            theme: ThemeChoice::default(),
            appearance: AppearanceChoice::default(),
            stop_button: Key {
                name: "Escape".to_string(),
                code: "Escape".to_string(),
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            },
            disabled_monitors: Vec::new(),
            monitor_regions: HashMap::new(),
            permissions: Permissions::default(),
            wallpaper: WallpaperConfig::default(),
            volume: Volume::default(),
            audio_device: None,
            schedule: ScheduleConfig::default(),
        }
    }
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;

    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path()?;
    let temp_config_path = path.with_added_extension("tmp");

    fs::write(&temp_config_path, serde_json::to_string(config)?)?;
    fs::rename(temp_config_path, path)?;

    Ok(())
}

pub async fn save_config_async(config: AppConfig) -> Result<()> {
    let path = config_path()?;
    let temp_config_path = path.with_added_extension("tmp");

    tracing::info!("{}", temp_config_path.display());

    tokio::fs::write(
        &temp_config_path,
        tokio::task::spawn_blocking(move || serde_json::to_string(&config)).await??,
    )
    .await?;
    tokio::fs::rename(temp_config_path, path).await?;

    Ok(())
}

fn config_path() -> Result<PathBuf> {
    let mut config_path = dirs::config_dir()
        .ok_or_else(|| anyhow!("Could not find a valid config dir for this OS"))?;

    config_path.push("lewdware");

    fs::create_dir_all(&config_path)?;

    config_path.push("config.json");

    Ok(config_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appconfig_json_roundtrip() {
        let original = AppConfig::default();
        let json = serde_json::to_string(&original).unwrap();
        let decoded: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            serde_json::to_value(&original).unwrap(),
            serde_json::to_value(&decoded).unwrap()
        );
    }

    #[test]
    fn key_display_no_modifiers() {
        let key = Key {
            name: "Escape".to_string(),
            code: "Escape".to_string(),
            modifiers: Modifiers::default(),
        };
        assert_eq!(key.to_string(), "Escape");
    }

    #[test]
    fn key_display_with_modifiers() {
        let key = Key {
            name: "Escape".to_string(),
            code: "Escape".to_string(),
            modifiers: Modifiers {
                shift: true,
                ..Default::default()
            },
        };
        assert_eq!(key.to_string(), "Shift + Escape");
    }

    #[test]
    fn key_display_all_modifiers() {
        let key = Key {
            name: "F1".to_string(),
            code: "F1".to_string(),
            modifiers: Modifiers {
                ctrl: true,
                alt: true,
                shift: true,
                meta: true,
            },
        };
        assert_eq!(key.to_string(), "Ctrl + Alt + Shift + Meta + F1");
    }

    #[test]
    fn default_volume_is_unattenuated() {
        let config = AppConfig::default();
        assert_eq!(config.volume.video, 1.0);
        assert_eq!(config.volume.audio, 1.0);
    }

    /// Opening a browser is the one capability whose effect leaves the desktop session, so it
    /// starts denied. Only new installs are affected: `capabilities` has no `#[serde(default)]`,
    /// so an existing `config.json` always carries the user's own answer.
    #[test]
    fn open_link_is_the_only_capability_denied_by_default() {
        let config = AppConfig::default();
        assert!(config.permissions.set_wallpaper);
        assert!(!config.permissions.open_links);
        assert!(config.permissions.send_notifications);
    }

    #[test]
    fn allows_maps_each_declared_permission_to_its_toggle() {
        use crate::mode::Permission;

        let capabilities = Permissions {
            set_wallpaper: true,
            open_links: false,
            send_notifications: true,
        };
        assert!(capabilities.allows(Permission::SetWallpaper));
        assert!(!capabilities.allows(Permission::OpenLinks));
        assert!(capabilities.allows(Permission::SendNotifications));
    }

    /// An existing user who allowed link opening keeps that answer -- the new default must not
    /// reach back into configs already on disk.
    #[test]
    fn an_existing_config_keeps_its_own_open_link_answer() {
        let json = serde_json::json!({
            "pack_path": null, "uploaded_modes": [], "mode": "Sandbox",
            "mode_options": [],
            "stop_button": {
                "name": "Escape", "code": "Escape",
                "modifiers": { "alt": false, "ctrl": false, "shift": true, "meta": false }
            },
            "disabled_monitors": [],
            "capabilities": { "set_wallpaper": true, "open_links": true, "send_notifications": true },
            "volume": { "video": 1.0, "audio": 1.0 }
        });

        let config: AppConfig = serde_json::from_value(json).unwrap();
        assert!(config.permissions.open_links);
    }

    /// Same risk as `schedule` below, one field later: nobody's `config.json` has a
    /// `"monitor_regions"` key, and a monitor with no entry means the whole screen.
    #[test]
    fn config_json_without_monitor_regions_still_loads() {
        let json = serde_json::json!({
            "pack_path": null, "uploaded_modes": [], "mode": "Sandbox",
            "mode_options": [],
            "stop_button": {
                "name": "Escape", "code": "Escape",
                "modifiers": { "alt": false, "ctrl": false, "shift": true, "meta": false }
            },
            "disabled_monitors": [],
            "capabilities": { "set_wallpaper": true, "open_links": true, "send_notifications": true },
            "volume": { "video": 1.0, "audio": 1.0 }
        });

        let config: AppConfig =
            serde_json::from_value(json).expect("should decode without a monitor_regions key");
        assert!(config.monitor_regions.is_empty());
    }

    /// The shape the Svelte side writes, and the engine reads back.
    #[test]
    fn a_monitor_region_round_trips_by_monitor_id() {
        let mut config = AppConfig::default();
        config.monitor_regions.insert(
            "eDP-1".to_owned(),
            crate::monitor::MonitorRegion {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        );

        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: AppConfig = serde_json::from_str(&encoded).unwrap();

        assert_eq!(
            decoded.monitor_regions.get("eDP-1").copied(),
            config.monitor_regions.get("eDP-1").copied()
        );
    }

    #[test]
    fn default_stop_button_is_shift_escape() {
        let config = AppConfig::default();
        assert_eq!(config.stop_button.code, "Escape");
        assert!(config.stop_button.modifiers.shift);
        assert!(!config.stop_button.modifiers.ctrl);
    }

    #[test]
    fn default_schedule_is_disabled_and_empty() {
        let config = AppConfig::default();
        assert!(!config.schedule.enabled);
        assert!(config.schedule.rules.is_empty());
        assert!(config.schedule.quiet_hours.is_empty());
        assert!(config.schedule.grace_notification);
    }

    /// The actual risk with adding a field to `AppConfig`: every pre-existing user's
    /// `config.json` has no `"schedule"` key at all. `#[serde(default)]` must keep it loading.
    #[test]
    fn config_json_without_a_schedule_key_still_loads() {
        let json = serde_json::json!({
            "pack_path": null,
            "uploaded_modes": [],
            "mode": "Sandbox",
            "mode_options": [],
            "stop_button": {
                "name": "Escape",
                "code": "Escape",
                "modifiers": { "alt": false, "ctrl": false, "shift": true, "meta": false }
            },
            "disabled_monitors": [],
            "capabilities": { "set_wallpaper": true, "open_links": true, "send_notifications": true },
            "volume": { "video": 1.0, "audio": 1.0 }
        });

        let config: AppConfig =
            serde_json::from_value(json).expect("should decode without a schedule key");
        assert_eq!(config.schedule, ScheduleConfig::default());
    }
}

#[cfg(test)]
mod wallpaper_config_tests {
    use super::*;

    /// Every pre-existing `config.json` predates this field, so a config without it must still
    /// load -- and must load as "restore the original", not as some half-set image.
    #[test]
    fn config_without_a_wallpaper_key_still_loads() {
        let json = r#"{
            "pack_path": null, "uploaded_modes": [], "mode": "Sandbox",
            "mode_options": [], "stop_button": {"name":"Escape","code":"Escape",
            "modifiers":{"alt":false,"ctrl":false,"shift":false,"meta":false}},
            "disabled_monitors": [],
            "capabilities": {"set_wallpaper": true, "open_links": true, "send_notifications": true},
            "volume": {"video": 1.0, "audio": 1.0}
        }"#;

        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.wallpaper.restore, WallpaperRestore::Original);
        assert_eq!(config.wallpaper.fallback_image(), None);
    }

    /// The shape the Svelte `WallpaperRestore` union has to match.
    #[test]
    fn restore_target_serialises_as_a_tagged_union() {
        let original = serde_json::to_string(&WallpaperRestore::Original).unwrap();
        assert_eq!(original, r#"{"kind":"original"}"#);

        let image = serde_json::to_string(&WallpaperRestore::Image {
            path: PathBuf::from("/a/b.png"),
        })
        .unwrap();
        assert_eq!(image, r#"{"kind":"image","path":"/a/b.png"}"#);
    }

    #[test]
    fn a_chosen_image_is_offered_as_the_fallback() {
        let config = WallpaperConfig {
            restore: WallpaperRestore::Image {
                path: PathBuf::from("/a/b.png"),
            },
        };
        assert_eq!(
            config.fallback_image(),
            Some(std::path::Path::new("/a/b.png"))
        );
    }
}
