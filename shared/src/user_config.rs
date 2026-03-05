use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::{Result, anyhow};
use dioxus_stores::Store;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::mode::OptionValue;

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Store, Clone)]
pub struct AppConfig {
    pub pack_path: Option<PathBuf>,
    pub uploaded_modes: Vec<PathBuf>,
    pub mode: Mode,
    #[serde_as(as = "Vec<(_, _)>")]
    pub mode_options: HashMap<Mode, HashMap<String, OptionValue>>,
    pub tags: Option<Vec<String>>,
    pub panic_button: Key,
    pub disabled_monitors: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub enum DefaultMode {
    Main,
}

impl DefaultMode {
    pub fn mode(&self) -> &'static str {
        match self {
            DefaultMode::Main => "default",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub enum Mode {
    Default(String),
    Pack { id: u64, mode: String },
    File { path: PathBuf, mode: String },
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Default("default".to_string())
    }
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
        if self.modifiers.ctrl  { parts.push("Ctrl");  }
        if self.modifiers.alt   { parts.push("Alt");   }
        if self.modifiers.shift { parts.push("Shift"); }
        if self.modifiers.meta  { parts.push("Meta");  }
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
            tags: None,
            panic_button: Key {
                name: "Escape".to_string(),
                code: "Escape".to_string(),
                modifiers: Modifiers {
                    shift: true,
                    ..Default::default()
                },
            },
            disabled_monitors: Vec::new(),
        }
    }
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_path()?;

    Ok(fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
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

    println!("{}", temp_config_path.display());

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
