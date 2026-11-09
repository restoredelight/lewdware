//! The modes on offer and the options the selected one presents. Ported from
//! `config/src-tauri/src/modes.rs` and `commands/{modes,pack}.rs`.

use std::collections::HashMap;

use indexmap::IndexMap;
use shared::behaviour::effective_options;
use shared::mode::{Metadata, ModeEntry, OptionValue, Permission, StoredValue};
use shared::pack::RecommendedMode;
use shared::user_config::{AppConfig, Mode};

use crate::backend::pack::{LoadedPack, UploadedModeEntry};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Pack,
    Uploaded,
    Builtin,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Pack => "Pack",
            Source::Uploaded => "Uploaded",
            Source::Builtin => "Built-in",
        }
    }
}

pub struct ModeChoice {
    pub mode: Mode,
    pub name: String,
    pub description: Option<String>,
    pub source: Source,
}

/// Whether two modes are the same choice. Sequence is one choice whichever pack its options are
/// scoped to.
pub fn same_mode(a: &Mode, b: &Mode) -> bool {
    match (a, b) {
        (Mode::Experience { .. }, Mode::Experience { .. }) => true,
        _ => a == b,
    }
}

pub fn builtin_mode_label(name: &str, recommended: bool) -> String {
    if recommended {
        format!("{name} (recommended)")
    } else {
        name.to_string()
    }
}

/// Every mode the user can pick: the pack's own, then uploaded files, then the two built-ins.
pub fn mode_choices(
    pack: Option<&LoadedPack>,
    uploaded: &[UploadedModeEntry],
    sandbox: &Metadata,
    experience: &Metadata,
) -> Vec<ModeChoice> {
    let mut choices: Vec<ModeChoice> = pack
        .map(|pack| {
            pack.modes
                .iter()
                .map(|m| ModeChoice {
                    mode: Mode::Pack { id: m.id },
                    name: m.metadata.name.clone(),
                    description: m.metadata.description.clone(),
                    source: Source::Pack,
                })
                .collect()
        })
        .unwrap_or_default();

    choices.extend(uploaded.iter().map(|u| ModeChoice {
        mode: Mode::File {
            path: u.path.clone(),
        },
        name: u.metadata.name.clone(),
        description: u.metadata.description.clone(),
        source: Source::Uploaded,
    }));

    // The pack's recommendation nudges, it doesn't restrict. A pack with a timeline may relabel
    // the Sequence mode (the Edgeware converter calls it "Corruption").
    let recommended = pack.and_then(|p| p.recommended_mode.clone());
    let timeline_label = pack.and_then(|p| p.timeline_label());
    choices.push(ModeChoice {
        mode: Mode::Sandbox,
        name: builtin_mode_label(
            &sandbox.name,
            matches!(recommended, Some(RecommendedMode::Sandbox)),
        ),
        description: sandbox.description.clone(),
        source: Source::Builtin,
    });
    choices.push(ModeChoice {
        mode: Mode::Experience { pack: None },
        name: builtin_mode_label(
            timeline_label.unwrap_or(&experience.name),
            matches!(recommended, Some(RecommendedMode::Experience)),
        ),
        description: experience.description.clone(),
        source: Source::Builtin,
    });
    choices
}

/// The mode a newly picked pack starts on: an embedded pack mode if it has one, otherwise the
/// built-in its author recommends, otherwise Sandbox (a plain content pack recommends Sandbox).
pub fn initial_mode(pack: &LoadedPack) -> Mode {
    if let Some(first) = pack.modes.first() {
        return Mode::Pack { id: first.id };
    }
    match pack.recommended_mode {
        Some(RecommendedMode::Experience) => Mode::Experience {
            pack: Some(pack.id),
        },
        _ => Mode::Sandbox,
    }
}

/// What the selected mode presents, resolved once whenever the mode, pack or uploads change.
#[derive(Default)]
pub struct Schema {
    pub entries: IndexMap<String, ModeEntry>,
    /// Permissions the mode uses however it's configured. They belong to no single option.
    pub needs_permissions: Vec<Permission>,
    /// Pack-derived facts (`pack_has_*`) a `show_when` can reference. Not options: no value is
    /// stored for them. Empty for a custom mode.
    pub pack_has: IndexMap<String, OptionValue>,
}

/// For the built-in modes with a pack loaded, the schema passes through the pack's behaviour,
/// which adds the content-group toggles. Custom modes never see behaviour-derived options.
pub fn schema_for(
    mode: &Mode,
    pack: Option<&LoadedPack>,
    uploaded: &[UploadedModeEntry],
    sandbox: &Metadata,
    experience: &Metadata,
) -> Schema {
    let metadata = match mode {
        Mode::Sandbox => Some(sandbox),
        Mode::Experience { .. } => Some(experience),
        Mode::Pack { id } => pack
            .and_then(|p| p.modes.iter().find(|m| m.id == *id))
            .map(|m| &m.metadata),
        Mode::File { path } => uploaded
            .iter()
            .find(|u| &u.path == path)
            .map(|u| &u.metadata),
    };
    let Some(metadata) = metadata else {
        return Schema::default();
    };

    if matches!(mode, Mode::Sandbox | Mode::Experience { .. }) {
        let behaviour = pack.map(|p| p.behaviour.clone()).unwrap_or_default();
        let effective = effective_options(metadata, &behaviour);
        Schema {
            entries: effective.entries,
            needs_permissions: metadata.needs_permissions.clone(),
            pack_has: effective.pack_has,
        }
    } else {
        Schema {
            entries: metadata.entries.clone(),
            needs_permissions: metadata.needs_permissions.clone(),
            pack_has: IndexMap::new(),
        }
    }
}

/// The key the selected mode's options are stored under. Sequence's are scoped per pack; every
/// other mode's are global.
pub fn options_scope(mode: &Mode, pack: Option<&LoadedPack>) -> Mode {
    match mode {
        Mode::Experience { pack: scoped } => Mode::Experience {
            pack: scoped.or_else(|| pack.map(|p| p.id)),
        },
        other => other.clone(),
    }
}

pub fn stored_options<'a>(
    config: &'a AppConfig,
    pack: Option<&LoadedPack>,
) -> Option<&'a HashMap<String, StoredValue>> {
    config.mode_options.get(&options_scope(&config.mode, pack))
}
