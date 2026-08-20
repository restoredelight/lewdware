//! Reading a pack or a mode file off disk, and assembling the mode and option lists the
//! frontend renders from what they declare.

use std::{
    collections::HashMap,
    io::{Cursor, Read, Seek, SeekFrom},
    path::PathBuf,
};

use indexmap::IndexMap;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, OptionalExtension};
use shared::{
    behaviour::effective_options,
    db::migrate,
    encode::FileType,
    mode::{self, ModeEntry, OptionValue, Permission, StoredValue},
    pack::{read_pack_metadata, RecommendedMode},
    user_config::{self, AppConfig, Mode},
};
use tempfile::NamedTempFile;

use crate::dto::{
    ConfigDto, ModeEntryDto, ModeGroupDto, ModeIdDto, ModeOptionDto, ModeOptionsDto,
    OptionEntryDto, OptionGroupDto,
};
use crate::state::{AppState, LoadedPack, PackModeEntry, UploadedModeEntry};

pub fn load_pack(path: PathBuf) -> anyhow::Result<LoadedPack> {
    let mut file = std::fs::File::open(&path)?;
    let (header, metadata) = read_pack_metadata(&mut file)?;

    let mut db_file = NamedTempFile::new()?;
    file.seek(SeekFrom::Start(header.index_offset))?;
    let mut db_data = (&mut file).take(header.index_length);
    std::io::copy(&mut db_data, db_file.as_file_mut())?;

    let manager = SqliteConnectionManager::file(db_file.path());
    let pool = Pool::builder().build(manager)?;
    let mut conn = pool.get()?;
    migrate(&mut conn)?;

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

    // Assembled from the `behaviour_*` tables and what is left of the blob -- see
    // `shared::behaviour::storage`. A pack whose document cannot be read at all still opens; it
    // simply offers no behaviour-derived options, which is what an empty document means anyway.
    let behaviour = shared::behaviour::storage::read(&conn)
        .inspect_err(|error| tracing::warn!("Could not read the pack's behaviour: {error}"))
        .unwrap_or_default();

    let mut referenced_media = HashMap::new();
    let mut media_stmt = conn.prepare("SELECT file_type FROM media WHERE id = ?")?;
    for id in behaviour.referenced_media_ids() {
        let file_type: Option<String> = media_stmt
            .query_row(params![id], |row| row.get(0))
            .optional()?;
        // An unrecognized `file_type` reads the same as a missing file: whatever it is, the
        // wallpaper/splash features can't use it.
        if let Some(file_type) = file_type.as_deref().and_then(parse_file_type) {
            referenced_media.insert(id, file_type);
        }
    }
    drop(media_stmt);

    Ok(LoadedPack {
        _db_file: db_file,
        id: header.id,
        modes,
        behaviour,
        referenced_media,
        recommended_mode: metadata.recommended_mode,
    })
}

pub fn parse_file_type(value: &str) -> Option<FileType> {
    match value {
        "image" => Some(FileType::Image),
        "video" => Some(FileType::Video),
        "audio" => Some(FileType::Audio),
        _ => None,
    }
}

pub fn load_mode_file(path: PathBuf) -> anyhow::Result<UploadedModeEntry> {
    let mut file = std::fs::File::open(&path)?;
    let (_, metadata) = mode::read_mode_metadata(&mut file)?;
    Ok(UploadedModeEntry { path, metadata })
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

pub fn build_mode_groups(state: &AppState) -> Vec<ModeGroupDto> {
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
    //
    // A pack that ships a timeline may also relabel the timeline mode (e.g. the Edgeware
    // converter emits "Corruption"); that label stands in for the mode's own name below. We
    // ignore a label on an empty timeline -- there'd be no progression to name.
    let (recommended, experience_label) = {
        let pack = state.pack.lock().unwrap();
        let recommended = pack.as_ref().and_then(|p| p.recommended_mode.clone());
        let experience_label = pack.as_ref().and_then(|p| {
            p.behaviour
                .experience
                .as_ref()
                .and_then(|e| e.label.clone().filter(|_| !e.timeline.stages.is_empty()))
        });
        (recommended, experience_label)
    };

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
                    experience_label
                        .as_deref()
                        .unwrap_or(&state.experience_mode.name),
                    matches!(recommended, Some(RecommendedMode::Experience)),
                ),
            },
        ],
    });

    groups
}

pub fn builtin_mode_label(name: &str, recommended: bool) -> String {
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
pub fn effective_entries_for_mode(
    mode: &Mode,
    state: &AppState,
) -> Option<EffectiveEntries> {
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
        let (behaviour, referenced_media) = state
            .pack
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| (p.behaviour.clone(), p.referenced_media.clone()))
            .unwrap_or_default();
        let media = |id: u64| referenced_media.get(&id).copied();
        let schema = effective_options(&mode_meta, &behaviour, &media);
        Some(EffectiveEntries {
            entries: schema.entries,
            needs_permissions,
            pack_has: schema.pack_has,
        })
    } else {
        Some(EffectiveEntries {
            entries: mode_meta.entries,
            needs_permissions,
            pack_has: IndexMap::new(),
        })
    }
}

/// What [`effective_entries_for_mode`] resolves a mode to.
pub struct EffectiveEntries {
    pub entries: IndexMap<String, ModeEntry>,
    /// The mode's unconditional permissions, off the same `Metadata` the entries came from.
    pub needs_permissions: Vec<Permission>,
    /// The pack-derived facts that drive `show_when` visibility; empty for a custom mode.
    pub pack_has: IndexMap<String, OptionValue>,
}

/// Resolves the values a mode's stored options should read from: `Mode::Experience` is scoped
/// per pack (`AppConfig::experience_options`), everything else globally
/// (`AppConfig::mode_options`) -- see `behaviour-design/default-mode.md`, Ownership. No pack
/// loaded means no scope to read Experience options from, so it falls back to empty (schema
/// defaults), the same as any other mode with nothing stored yet.
pub fn stored_options_for(
    mode: &Mode,
    config: &AppConfig,
    state: &AppState,
) -> HashMap<String, StoredValue> {
    if matches!(mode, Mode::Experience) {
        let pack_id = state.pack.lock().unwrap().as_ref().map(|p| p.id);
        pack_id
            .and_then(|id| config.experience_options.get(&id).cloned())
            .unwrap_or_default()
    } else {
        config.mode_options.get(mode).cloned().unwrap_or_default()
    }
}

pub fn get_mode_options_for(config: &AppConfig, state: &AppState) -> ModeOptionsDto {
    let Some(EffectiveEntries {
        entries,
        needs_permissions,
        pack_has,
    }) = effective_entries_for_mode(&config.mode, state)
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
        stored: &HashMap<String, StoredValue>,
    ) -> Vec<OptionEntryDto> {
        entries
            .iter()
            .map(|(key, entry)| match entry {
                ModeEntry::Option(opt) => {
                    // The frontend gets the *resolved* value, so what it renders is what the
                    // mode would run with -- not whatever shape the value has on disk.
                    let value = opt.resolve(stored.get(key));
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

pub fn save_to_disk(
    config: &AppConfig,
    uploaded: &[UploadedModeEntry],
) -> anyhow::Result<()> {
    let mut c = config.clone();
    c.uploaded_modes = uploaded.iter().map(|u| u.path.clone()).collect();
    user_config::save_config(&c)
}

/// Fold the frontend's settings onto the config this process already holds.
///
/// Split out of `save_config` so the rule can be stated once and tested without a Tauri `State`:
/// **the DTO supplies what the frontend owns; everything else is kept from `current`.**
///
/// The fields kept are the ones their own commands write and persist as they go —
/// `set_mode_option` for the two option maps, `upload_mode`/`remove_uploaded_mode` for the mode
/// list. The frontend never learns about those writes (it holds a snapshot from page load), so
/// taking them from the DTO reverted every mode option the user had set that session as soon as
/// anything else was saved — changing the theme, the volume, or switching mode.
pub fn apply_config_dto(current: &AppConfig, dto: ConfigDto) -> AppConfig {
    AppConfig {
        mode_options: current.mode_options.clone(),
        experience_options: current.experience_options.clone(),
        uploaded_modes: current.uploaded_modes.clone(),
        ..dto.into()
    }
}
