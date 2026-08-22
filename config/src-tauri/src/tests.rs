use std::collections::HashMap;
use std::path::PathBuf;

use indexmap::IndexMap;
use shared::{
    behaviour::{Behaviour, ContentGroup},
    db::migrate,
    encode::FileType,
    mode::{self, Metadata, ModeEntry, OptionType, OptionValue, Permission, StoredValue},
    user_config::{AppConfig, AudioDeviceChoice, Mode},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

use std::sync::Mutex;

use crate::dto::{ConfigDto, OptionEntryDto};
use crate::modes::{apply_config_dto, build_mode_groups, get_mode_options_for, load_pack};
use crate::state::{AppState, LoadedPack, PackModeEntry};

fn empty_metadata(name: &str) -> Metadata {
    Metadata {
        name: name.to_string(),
        description: None,
        version: None,
        author: None,
        entrypoint: "main.lua".to_string(),
        entries: Default::default(),
        files: HashMap::new(),
        needs_permissions: Vec::new(),
    }
}

/// The bug this guards: mode option values are written by `set_mode_option` and persisted as
/// it goes, but the frontend's copy of the config is a snapshot from page load, so it never
/// learns about those writes. Rebuilding the whole `AppConfig` from that snapshot reverted
/// every option the user had set that session — the visible symptom being that changing the
/// theme (or the volume, or the mode) silently reset the mode's settings.
#[test]
fn saving_frontend_settings_keeps_the_options_the_backend_owns() {
    let pack = Uuid::new_v4();
    let stored = || HashMap::from([("popup_frequency".to_string(), StoredValue::Float(4.0))]);

    let current = AppConfig {
        theme: "plain".to_string(),
        mode_options: HashMap::from([(Mode::Sandbox, stored())]),
        experience_options: HashMap::from([(pack, stored())]),
        uploaded_modes: vec![PathBuf::from("/modes/custom.lwmode")],
        ..Default::default()
    };

    // What the frontend sends when the user changes one unrelated setting: its own fields,
    // and nothing at all about the options.
    let dto = ConfigDto {
        theme: "breeze".to_string(),
        ..AppConfig::default().into()
    };

    let saved = apply_config_dto(&current, dto);

    assert_eq!(saved.theme, "breeze", "the frontend's own field is taken");
    assert_eq!(saved.mode_options, current.mode_options);
    assert_eq!(saved.experience_options, current.experience_options);
    assert_eq!(saved.uploaded_modes, current.uploaded_modes);
}

/// The frontend cannot send option values even if it wanted to — they are not in the DTO —
/// which is what makes the case above unrepresentable rather than merely handled.
#[test]
fn the_dto_carries_no_option_values() {
    let dto = ConfigDto {
        ..AppConfig {
            mode_options: HashMap::from([(
                Mode::Sandbox,
                HashMap::from([("k".to_string(), StoredValue::Bool(true))]),
            )]),
            ..Default::default()
        }
        .into()
    };

    let json = serde_json::to_value(&dto).unwrap();
    let object = json.as_object().expect("the DTO is a JSON object");
    assert!(!object.contains_key("mode_options"), "{object:?}");
    assert!(!object.contains_key("experience_options"), "{object:?}");
}

/// The mirror of the bug above, for a field the *frontend* owns: `save_config` rebuilds a
/// whole fresh `AppConfig` from the DTO, so a frontend field missing from `ConfigDto` is reset
/// to its default by any unrelated save -- picking an output device and then touching the
/// volume slider would silently put it back to "system default".
#[test]
fn saving_keeps_the_chosen_audio_device() {
    let current = AppConfig {
        audio_device: Some(AudioDeviceChoice {
            id: "pulseaudio:some-sink".to_string(),
            name: "Some speakers".to_string(),
        }),
        ..Default::default()
    };

    // The frontend's snapshot, as it would come back with some other setting changed.
    let dto = ConfigDto {
        theme: "breeze".to_string(),
        ..current.clone().into()
    };

    let saved = apply_config_dto(&current, dto);

    assert_eq!(saved.audio_device, current.audio_device);
}

/// A `config.json` written before output devices existed has no `audio_device` key at all, and
/// must still load rather than failing the whole config.
#[test]
fn a_config_without_an_audio_device_loads_as_the_system_default() {
    let json = serde_json::to_value(AppConfig::default()).unwrap();
    let mut object = json.as_object().unwrap().clone();
    object.remove("audio_device");

    let config: AppConfig = serde_json::from_value(object.into()).expect("loads without it");

    assert_eq!(config.audio_device, None);
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

/// Writes a minimal `.lwpack` holding `behaviour` and one media file, so `load_pack` can be
/// exercised against a real file rather than a hand-built `LoadedPack`.
fn write_pack(dir: &std::path::Path, behaviour: &Behaviour, media: &[(&str, &str)]) -> PathBuf {
    use std::io::Write;

    let db_path = dir.join("index.db");
    {
        let mut conn = rusqlite::Connection::open(&db_path).unwrap();
        migrate(&mut conn).unwrap();
        for (name, file_type) in media {
            conn.execute(
                "INSERT INTO media (file_name, file_type, offset, length, hash)
                 VALUES (?, ?, 0, 0, x'00')",
                rusqlite::params![name, file_type],
            )
            .unwrap();
        }
        // Through the real write path: the media slots live in `behaviour_content` now, so a
        // hand-written blob would arrive with the wallpaper missing.
        let mut conn = conn;
        let tx = conn.transaction().unwrap();
        shared::behaviour::storage::write(&tx, behaviour).unwrap();
        tx.commit().unwrap();
    }
    let db_bytes = std::fs::read(&db_path).unwrap();

    // `Metadata` in scope here is the *mode* one; the pack's is a different type.
    let metadata = shared::pack::Metadata {
        name: "Test pack".to_string(),
        ..Default::default()
    };
    let metadata_bytes = metadata.to_buf().unwrap();
    let mut header = shared::pack::Header::new();
    header.metadata_offset = shared::pack::HEADER_SIZE as u64;
    header.metadata_length = metadata_bytes.len() as u64;
    header.index_offset = header.metadata_offset + header.metadata_length;
    header.index_length = db_bytes.len() as u64;

    let pack_path = dir.join("test.lwpack");
    let mut file = std::fs::File::create(&pack_path).unwrap();
    file.write_all(&header.to_buf().unwrap()).unwrap();
    file.write_all(&metadata_bytes).unwrap();
    file.write_all(&db_bytes).unwrap();
    file.flush().unwrap();
    pack_path
}

/// A behaviour whose wallpaper slot points at `media` -- `write_pack` inserts its media in
/// order, so the first file it is given is id 1.
fn behaviour_with_wallpaper(media: u64) -> Behaviour {
    Behaviour {
        content: shared::behaviour::Content {
            wallpaper: Some(media),
            ..Default::default()
        },
        ..Behaviour::new()
    }
}

/// `pack_has_wallpaper` is the whole reason the wallpaper toggle is offered, and it is now
/// answered by looking the referenced file up in the pack rather than by trusting the
/// reference. That lookup runs at load time against the pack's own index, so it is only ever
/// as right as `load_pack`'s query -- exercised here end to end, from a real file.
#[test]
fn a_wallpaper_reference_that_resolves_shows_the_toggle() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_pack(
        dir.path(),
        &behaviour_with_wallpaper(1),
        &[("bg.png", "image")],
    );

    let pack = load_pack(path).unwrap();
    assert_eq!(pack.referenced_media.get(&1), Some(&FileType::Image));

    let state = test_state(Some(pack));
    let config = AppConfig {
        mode: Mode::Sandbox,
        ..Default::default()
    };
    assert_eq!(
        get_mode_options_for(&config, &state)
            .pack_has
            .get("pack_has_wallpaper"),
        Some(&OptionValue::Boolean(true)),
    );
}

/// The other half of the same fact: a slot the pack can't honour must report false rather than
/// offer a toggle that does nothing.
///
/// Only the wrong-media-type case is reachable now. A slot naming a file the pack does not
/// have used to be the other half of this test, and the foreign key on `behaviour_content`
/// has since made it unrepresentable -- deleting the file nulls the slot, and writing a slot
/// that points at nothing is refused (`shared::behaviour::storage`).
#[test]
fn a_wallpaper_reference_of_the_wrong_type_hides_the_toggle() {
    for file_type in ["video", "audio"] {
        let dir = tempfile::tempdir().unwrap();
        let path = write_pack(
            dir.path(),
            &behaviour_with_wallpaper(1),
            &[("bg.png", file_type)],
        );

        let state = test_state(Some(load_pack(path).unwrap()));
        let config = AppConfig {
            mode: Mode::Sandbox,
            ..Default::default()
        };
        assert_eq!(
            get_mode_options_for(&config, &state)
                .pack_has
                .get("pack_has_wallpaper"),
            Some(&OptionValue::Boolean(false)),
            "a {file_type} should not satisfy a wallpaper slot",
        );
    }
}

fn loaded_pack(behaviour: Behaviour, modes: Vec<PackModeEntry>) -> LoadedPack {
    LoadedPack {
        _db_file: NamedTempFile::new().unwrap(),
        id: Uuid::new_v4(),
        metadata: shared::pack::Metadata::default(),
        modes,
        behaviour,
        referenced_media: HashMap::new(),
        recommended_mode: None,
    }
}

#[test]
fn mode_groups_include_authored_descriptions() {
    let mut metadata = empty_metadata("Described mode");
    metadata.description = Some("Explains how this mode behaves.".to_string());
    let pack = loaded_pack(Behaviour::new(), vec![PackModeEntry { id: 7, metadata }]);
    let state = test_state(Some(pack));

    let groups = build_mode_groups(&state);
    let entry = groups
        .iter()
        .find(|group| group.source == "pack")
        .and_then(|group| group.entries.first())
        .expect("the embedded mode is listed");

    assert_eq!(
        entry.description.as_deref(),
        Some("Explains how this mode behaves.")
    );
}

#[test]
fn numeric_presentation_reaches_the_dto() {
    let mut sandbox = empty_metadata("Sandbox");
    sandbox.entries.insert(
        "popup_frequency".to_string(),
        ModeEntry::Option(mode::ModeOption {
            label: "Popup frequency".to_string(),
            description: None,
            suffix: Some("seconds".to_string()),
            logarithmic: true,
            option_type: OptionType::Number {
                default: 1.0,
                min: None,
                max: None,
                step: None,
                clamp: false,
                slider: false,
            },
            optional: false,
            enabled_by_default: false,
            show_when: None,
            needs_permissions: Vec::new(),
        }),
    );

    let mut state = test_state(None);
    state.sandbox_mode = sandbox;
    let dto = get_mode_options_for(
        &AppConfig {
            mode: Mode::Sandbox,
            ..Default::default()
        },
        &state,
    );

    let OptionEntryDto::Option(option) = find_entry(&dto.entries, "popup_frequency").unwrap()
    else {
        panic!("expected an option entry");
    };
    assert_eq!(option.suffix.as_deref(), Some("seconds"));
    assert!(option.logarithmic);
}

fn test_state(pack: Option<LoadedPack>) -> AppState {
    AppState {
        config: Mutex::new(AppConfig::default()),
        pack: Mutex::new(pack),
        uploaded: Mutex::new(Vec::new()),
        sandbox_mode: empty_metadata("Sandbox"),
        experience_mode: empty_metadata("Sequence"),
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
                    suffix: None,
                    logarithmic: false,
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

    let OptionEntryDto::Option(opt) = find_entry(&dto.entries, "wallpaper_enabled").unwrap() else {
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

/// A `content_group.*` key exists only in the *synthesized* schema
/// (`effective_options`), not in the mode's own -- so a stored value for one is only
/// honoured if the lookup goes through `effective_entries_for_mode`. Storing the
/// non-default and reading it back is what proves it does.
#[test]
fn content_group_key_resolves_against_the_synthesized_schema() {
    let state = test_state(Some(loaded_pack(
        behaviour_with_one_content_group(),
        vec![],
    )));
    let mut config = AppConfig {
        mode: Mode::Sandbox,
        ..Default::default()
    };
    config
        .mode_options
        .entry(Mode::Sandbox)
        .or_default()
        .insert("content_group.kinky".to_string(), StoredValue::Bool(false));

    let entries = get_mode_options_for(&config, &state).entries;

    let Some(OptionEntryDto::Option(opt)) = find_entry(&entries, "content_group.kinky") else {
        panic!("no content_group.kinky option in {entries:?}");
    };
    assert_eq!(opt.option_type, OptionType::Boolean { default: true });
    assert_eq!(opt.value, OptionValue::Boolean(false));
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

// ─── crowding ──────────────────────────────────────────────────────────────────

fn rate_rule_dto(
    from: (u32, u32),
    to: (u32, u32),
    count: u32,
    minutes: u32,
) -> crate::dto::RuleDto {
    use shared::schedule::{Frequency, Range, SessionLength, TimeOfDay, Trigger};
    crate::dto::RuleDto {
        id: Uuid::new_v4(),
        days: [true; 7],
        trigger: Trigger::Rate {
            range: Range::Between {
                from: TimeOfDay::new(from.0, from.1),
                to: TimeOfDay::new(to.0, to.1),
            },
            frequency: Frequency::PerDay { count },
        },
        length: SessionLength::Fixed { minutes },
        overrides: Default::default(),
    }
}

fn schedule_dto(rules: Vec<crate::dto::RuleDto>, cooldown_minutes: u32) -> crate::dto::ScheduleDto {
    crate::dto::ScheduleDto {
        enabled: true,
        rules,
        quiet_hours: Vec::new(),
        grace_notification: false,
        cooldown_minutes,
        panic_cooldown_minutes: 120,
    }
}

/// A comfortable rule says nothing at all. The check is advisory, and an advisory that fires on
/// ordinary configurations is noise the user learns to ignore.
#[test]
fn a_comfortable_rule_is_not_reported() {
    let schedule = schedule_dto(vec![rate_rule_dto((9, 0), (17, 0), 3, 20)], 30);
    assert!(crate::commands::schedule::schedule_crowding(schedule).is_empty());
}

/// The shape the delivery grid measured at 8%: it *fits* -- 370 minutes of 480 -- and is still
/// hopeless, so a check that only asked whether it fitted would say nothing.
#[test]
fn a_budget_that_fits_but_crowds_is_reported_without_calling_it_impossible() {
    let schedule = schedule_dto(vec![rate_rule_dto((9, 0), (17, 0), 8, 20)], 30);
    let reported = crate::commands::schedule::schedule_crowding(schedule);

    assert_eq!(reported.len(), 1);
    assert!(!reported[0].impossible);
    assert!((reported[0].occupancy - 370.0 / 480.0).abs() < 1e-9);
    assert!(reported[0].comfortable_count < 8);
}

#[test]
fn a_budget_larger_than_its_window_is_reported_as_impossible() {
    let schedule = schedule_dto(vec![rate_rule_dto((9, 0), (11, 0), 6, 20)], 30);
    let reported = crate::commands::schedule::schedule_crowding(schedule);

    assert_eq!(reported.len(), 1);
    assert!(reported[0].impossible);
    // 6 * 20 + 5 * 30 = 270 minutes wanted, and the range holds 120.
    assert_eq!(reported[0].required_minutes, 270.0);
    assert_eq!(reported[0].available_minutes, 120.0);
}

/// One entry per crowded rule, so the warning can attach to the rule the user would edit.
#[test]
fn each_crowded_rule_is_reported_against_its_own_id() {
    let tight = rate_rule_dto((9, 0), (11, 0), 6, 20);
    let roomy = rate_rule_dto((9, 0), (17, 0), 2, 20);
    let tight_id = tight.id;

    let reported =
        crate::commands::schedule::schedule_crowding(schedule_dto(vec![roomy, tight], 30));
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].rule_id, tight_id);
}
