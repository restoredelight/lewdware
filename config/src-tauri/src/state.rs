//! What the app holds while it runs: the loaded pack, the modes uploaded to it, and the
//! live config.

use std::{path::PathBuf, sync::Mutex};

use shared::{
    behaviour::Behaviour,
    mode::Metadata,
    pack::{Metadata as PackMetadata, RecommendedMode},
    user_config::AppConfig,
};
use tempfile::NamedTempFile;
use uuid::Uuid;

pub struct PackModeEntry {
    pub id: u64,
    pub metadata: Metadata,
}

pub struct UploadedModeEntry {
    pub path: PathBuf,
    pub metadata: Metadata,
}

pub struct LoadedPack {
    pub _db_file: NamedTempFile,
    pub id: Uuid,
    pub metadata: PackMetadata,
    pub modes: Vec<PackModeEntry>,
    /// The pack's behaviour.json, if it has one -- `Behaviour::new()` (empty) otherwise. Used
    /// only to synthesize the built-in default modes' content-group toggles (see
    /// `effective_entries_for_mode`); custom modes never consult this.
    pub behaviour: Behaviour,
    /// Which mode the pack author suggests -- see `pick_pack`'s preselection and
    /// `build_mode_groups`'s "(recommended)" marker.
    pub recommended_mode: Option<RecommendedMode>,
}

pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub pack: Mutex<Option<LoadedPack>>,
    pub uploaded: Mutex<Vec<UploadedModeEntry>>,
    pub sandbox_mode: Metadata,
    pub experience_mode: Metadata,
}

pub type State<'a> = tauri::State<'a, AppState>;
