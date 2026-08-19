//! What the app holds while it runs: the loaded pack, the modes uploaded to it, and the
//! live config.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
};

use shared::{
    behaviour::Behaviour, encode::FileType, mode::Metadata, read_pack::RecommendedMode,
    user_config::AppConfig,
};
use tempfile::NamedTempFile;
use uuid::Uuid;

pub(crate) struct PackModeEntry {
    pub(crate) id: u64,
    pub(crate) metadata: Metadata,
}

pub(crate) struct UploadedModeEntry {
    pub(crate) path: PathBuf,
    pub(crate) metadata: Metadata,
}

pub(crate) struct LoadedPack {
    pub(crate) _db_file: NamedTempFile,
    pub(crate) id: Uuid,
    pub(crate) modes: Vec<PackModeEntry>,
    /// The pack's behaviour.json, if it has one -- `Behaviour::new()` (empty) otherwise. Used
    /// only to synthesize the built-in default modes' content-group toggles (see
    /// `effective_entries_for_mode`); custom modes never consult this.
    pub(crate) behaviour: Behaviour,
    /// The type of each media file `behaviour` references by id (wallpaper/splash slots), for
    /// the `pack_has_*` facts -- see `shared::behaviour::MediaLookup`. Resolved once at load
    /// time, while the pack's index db is open; an id that isn't in the pack is simply absent.
    /// Config never needs the rest of the pack's media, so this stays keyed to what's referenced
    /// rather than mirroring the whole media table.
    pub(crate) referenced_media: HashMap<u64, FileType>,
    /// Which mode the pack author suggests -- see `pick_pack`'s preselection and
    /// `build_mode_groups`'s "(recommended)" marker.
    pub(crate) recommended_mode: Option<RecommendedMode>,
}

pub struct AppState {
    pub(crate) config: Mutex<AppConfig>,
    pub(crate) pack: Mutex<Option<LoadedPack>>,
    pub(crate) uploaded: Mutex<Vec<UploadedModeEntry>>,
    pub(crate) sandbox_mode: Metadata,
    pub(crate) experience_mode: Metadata,
}

pub type State<'a> = tauri::State<'a, AppState>;
