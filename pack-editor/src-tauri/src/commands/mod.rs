//! Every `#[tauri::command]` the frontend can invoke, grouped by the part of the editor it
//! serves. `lib.rs` names them by path in `generate_handler!`.

pub(crate) mod behaviour;
pub(crate) mod files;
pub(crate) mod labels;
pub(crate) mod lifecycle;
pub(crate) mod metadata;
pub(crate) mod modes;
pub(crate) mod recents;
pub(crate) mod server;
pub(crate) mod slots;
pub(crate) mod update;
pub(crate) mod upload;

use serde::{Deserialize, Serialize};
use shared::read_pack::{Metadata, RecommendedMode};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MetadataDto {
    pub name: String,
    pub creator: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    // Not yet editable from the pack editor UI (no Modes tab) -- carried through opaquely so
    // saving an unrelated metadata edit (e.g. renaming the pack) doesn't clobber a
    // recommended_mode set by, e.g., the converter. `#[serde(default)]` so the frontend's
    // pre-Modes-tab payloads (which don't send this key) still deserialize.
    #[serde(default)]
    pub recommended_mode: Option<RecommendedMode>,
}

impl From<Metadata> for MetadataDto {
    fn from(m: Metadata) -> Self {
        Self {
            name: m.name,
            creator: m.creator,
            description: m.description,
            version: m.version,
            recommended_mode: m.recommended_mode,
        }
    }
}

impl From<MetadataDto> for Metadata {
    fn from(d: MetadataDto) -> Self {
        Self {
            name: d.name,
            creator: d.creator,
            description: d.description,
            version: d.version,
            recommended_mode: d.recommended_mode,
        }
    }
}
