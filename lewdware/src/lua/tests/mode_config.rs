use shared::behaviour::Behaviour;

use super::{harness::*, *};

/// The engine reads the document through the media manager, and the document is assembled
/// from tables plus what is left of the blob -- so this covers the read path end to end from
/// a real pack file rather than the blob alone.
#[test]
fn media_manager_reads_the_assembled_behaviour_document() {
    let behaviour = behaviour_with_one_content_group();
    let pack_file = pack_fixture_with_data(&[], Some(&behaviour));
    let event_poster = EmptyPoster();
    let (media_manager, _metadata, _pack_id) =
        MediaManager::open(pack_file.path(), event_poster, None).unwrap();

    assert_eq!(media_manager.get_behaviour().unwrap(), behaviour);
}

/// A pack that has never had a behaviour document written reads as an empty one, not an error.
#[test]
fn media_manager_reads_a_default_document_when_the_pack_has_none() {
    let pack_file = pack_fixture_with_data(&[], None);
    let event_poster = EmptyPoster();
    let (media_manager, _metadata, _pack_id) =
        MediaManager::open(pack_file.path(), event_poster, None).unwrap();

    assert_eq!(media_manager.get_behaviour().unwrap(), Behaviour::new());
}

fn empty_default_mode_metadata() -> Metadata {
    Metadata {
        name: "test-mode".to_string(),
        description: None,
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
        content: Content {
            content_groups: vec![shared::behaviour::ContentGroup {
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

/// The core invariant `resolve_mode_config` exists for: for the built-in Sandbox mode, a
/// content group synthesizes into the resolved config (falling back to its
/// `enabled_by_default`, or a stored override when present) and its tags are handed back via
/// `Content` for the query layer -- see `default-modes/shared/lib/media.lua`.
#[test]
fn resolve_mode_config_synthesizes_content_group_toggle_for_sandbox_mode() {
    let behaviour = behaviour_with_one_content_group();
    let pack_file = pack_fixture_with_data(&[], Some(&behaviour));

    let event_poster = EmptyPoster();
    let (media_manager, _metadata, pack_id) =
        MediaManager::open(pack_file.path(), event_poster, None).unwrap();

    let metadata = empty_default_mode_metadata();
    let key = format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX);

    // No stored override -> falls back to the group's `enabled_by_default`.
    let (config, content, _experience) = resolve_mode_config(
        &metadata,
        &shared::user_config::Mode::Sandbox,
        &HashMap::new(),
        &HashMap::new(),
        pack_id,
        &media_manager,
    );
    assert_eq!(config.get(&key), Some(&OptionValue::Boolean(true)));
    let groups = content["content_groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["tags"], serde_json::json!(["kinky"]));

    // A stored override wins over the default.
    let mut stored = HashMap::new();
    stored.insert(key.clone(), StoredValue::Bool(false));
    let mut mode_options = HashMap::new();
    mode_options.insert(shared::user_config::Mode::Sandbox, stored);

    let (config, _, _experience) = resolve_mode_config(
        &metadata,
        &shared::user_config::Mode::Sandbox,
        &mode_options,
        &HashMap::new(),
        pack_id,
        &media_manager,
    );
    assert_eq!(config.get(&key), Some(&OptionValue::Boolean(false)));
}

/// The actual "custom modes never see the toggles" invariant (`behaviour-design/
/// default-mode.md`, Ownership): even against a pack whose behaviour.json declares content
/// groups, a mode embedded in that pack gets neither the synthesized option nor the content
/// data.
#[test]
fn resolve_mode_config_hides_content_groups_from_custom_modes() {
    let behaviour = behaviour_with_one_content_group();
    let pack_file = pack_fixture_with_data(&[], Some(&behaviour));

    let event_poster = EmptyPoster();
    let (media_manager, _metadata, pack_id) =
        MediaManager::open(pack_file.path(), event_poster, None).unwrap();

    let metadata = empty_default_mode_metadata();
    let key = format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX);

    for mode in [
        shared::user_config::Mode::Pack { id: 1 },
        shared::user_config::Mode::File {
            path: "some/mode.lwmode".into(),
        },
    ] {
        let (config, content, _experience) = resolve_mode_config(
            &metadata,
            &mode,
            &HashMap::new(),
            &HashMap::new(),
            pack_id,
            &media_manager,
        );

        assert_eq!(config.get(&key), None);
        assert_eq!(content, lua_view(&Content::default(), |_| None));
    }
}

/// A pack with no behaviour.json at all (the common case today) shouldn't panic or fail the
/// mode -- `resolve_mode_config` falls back to an empty `Behaviour`.
#[test]
fn resolve_mode_config_sandbox_mode_with_no_behaviour_data() {
    let pack_file = pack_fixture_with_data(&[], None);

    let event_poster = EmptyPoster();
    let (media_manager, _metadata, pack_id) =
        MediaManager::open(pack_file.path(), event_poster, None).unwrap();

    let metadata = empty_default_mode_metadata();

    let (config, content, _experience) = resolve_mode_config(
        &metadata,
        &shared::user_config::Mode::Sandbox,
        &HashMap::new(),
        &HashMap::new(),
        pack_id,
        &media_manager,
    );

    assert!(config.is_empty());
    assert_eq!(content, lua_view(&Content::default(), |_| None));
}

/// `Mode::Experience`'s stored options are scoped per pack (`AppConfig::experience_options`),
/// unlike every other mode's global `mode_options` -- see `behaviour-design/default-mode.md`,
/// Ownership. A stored value under a *different* pack's UUID must never leak into this one's
/// resolved config.
#[test]
fn resolve_mode_config_experience_mode_reads_scoped_experience_options() {
    let pack_file = pack_fixture_with_data(&[], None);

    let event_poster = EmptyPoster();
    let (media_manager, _metadata, pack_id) =
        MediaManager::open(pack_file.path(), event_poster, None).unwrap();

    let mut entries = indexmap::IndexMap::new();
    entries.insert(
        "pace".to_string(),
        shared::mode::ModeEntry::Option(shared::mode::ModeOption {
            label: "Pacing".to_string(),
            description: None,
            option_type: shared::mode::OptionType::Number {
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
    let metadata = Metadata {
        name: "Experience".to_string(),
        description: None,
        version: None,
        author: None,
        entrypoint: "main.lua".to_string(),
        entries,
        files: HashMap::new(),
        needs_permissions: Vec::new(),
    };

    let mut this_pack_options = HashMap::new();
    this_pack_options.insert("pace".to_string(), StoredValue::Float(2.0));
    let mut other_pack_options = HashMap::new();
    other_pack_options.insert("pace".to_string(), StoredValue::Float(9.0));

    let mut experience_options = HashMap::new();
    experience_options.insert(pack_id, this_pack_options);
    experience_options.insert(Uuid::new_v4(), other_pack_options);

    let (config, _content, _experience) = resolve_mode_config(
        &metadata,
        &shared::user_config::Mode::Experience,
        &HashMap::new(),
        &experience_options,
        pack_id,
        &media_manager,
    );

    assert_eq!(config.get("pace"), Some(&OptionValue::Number(2.0)));
}

/// Experience is the pack-behaviour-consuming default mode too -- content-group toggles
/// synthesize identically to Sandbox's (mirrors
/// `resolve_mode_config_synthesizes_content_group_toggle_for_sandbox_mode`).
#[test]
fn resolve_mode_config_experience_mode_synthesizes_content_group_toggle() {
    let behaviour = behaviour_with_one_content_group();
    let pack_file = pack_fixture_with_data(&[], Some(&behaviour));

    let event_poster = EmptyPoster();
    let (media_manager, _metadata, pack_id) =
        MediaManager::open(pack_file.path(), event_poster, None).unwrap();

    let metadata = empty_default_mode_metadata();
    let key = format!("{}kinky", shared::behaviour::CONTENT_GROUP_KEY_PREFIX);

    let (config, content, _experience) = resolve_mode_config(
        &metadata,
        &shared::user_config::Mode::Experience,
        &HashMap::new(),
        &HashMap::new(),
        pack_id,
        &media_manager,
    );
    assert_eq!(config.get(&key), Some(&OptionValue::Boolean(true)));
    assert_eq!(content["content_groups"].as_array().unwrap().len(), 1);
}
