use std::collections::HashMap;

use indexmap::IndexMap;

use crate::mode::{
    Metadata, ModeEntry, ModeGroup, ModeOption, OptionType, OptionValue, StoredValue,
};

use crate::encode::FileType;

use super::schema::{Behaviour, Content};

/// Prefix for synthesized content-group toggle option keys (see `effective_options`). Lets a
/// caller recognize which resolved keys are pack-derived (and so need per-pack storage scoping,
/// per `behaviour-design/default-mode.md`'s Ownership section) by convention.
pub const CONTENT_GROUP_KEY_PREFIX: &str = "content_group.";

const CONTENT_GROUPS_ENTRY_KEY: &str = "content_groups";
const CONTENT_GROUPS_ENTRY_LABEL: &str = "Content";

/// A mode's options schema, extended with behaviour-derived data: `entries` is the mode's own
/// schema plus a synthesized "Content" group of per-content-group toggles (if the pack declares
/// any); `pack_has` are facts (not options — no defaults, never stored) for evaluating a mode
/// option's `show_when` against pack-adaptive visibility (`pack_has_prompts`, etc.). See
/// `behaviour-design/behaviour-tab.md`'s resolver section.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveSchema {
    pub entries: IndexMap<String, ModeEntry>,
    pub pack_has: IndexMap<String, OptionValue>,
}

/// Resolves a media id against the pack's own media, for the `pack_has_*` facts that ask about a
/// file rather than a list (`pack_has_wallpaper`/`pack_has_splash`). `None` means the pack has no
/// such file -- which, now that slots hold ids, means it was deleted rather than merely renamed.
///
/// A function rather than a concrete index because the two callers reach the pack's media very
/// differently: the engine has a live `MediaManager`, while `config` only holds the pack's
/// extracted index db. See `no_media` for the "no pack loaded at all" case.
pub trait MediaLookup {
    fn file_type(&self, media: u64) -> Option<FileType>;
}

impl<F: Fn(u64) -> Option<FileType>> MediaLookup for F {
    fn file_type(&self, media: u64) -> Option<FileType> {
        self(media)
    }
}

/// A `MediaLookup` for "there is no pack": every id resolves to nothing, so every media-dependent
/// `pack_has_*` fact is false.
pub fn no_media(_media: u64) -> Option<FileType> {
    None
}

/// Synthesizes the schema a default mode actually presents for a given pack: the mode's own
/// options plus one boolean toggle per content group (rendered as a single "Content" checklist),
/// and the `pack_has_*` facts used to drive `show_when` visibility elsewhere in the schema.
/// Injects no option *defaults* from behaviour data — a content group's `enabled_by_default`
/// becomes the synthesized option's own schema default, not a behaviour-data override.
///
/// `media` answers what the wallpaper/splash references actually resolve to; see
/// `pack_has_constants`.
pub fn effective_options(
    mode_schema: &Metadata,
    behaviour: &Behaviour,
    media: &dyn MediaLookup,
) -> EffectiveSchema {
    let mut entries = mode_schema.entries.clone();

    if !behaviour.content.content_groups.is_empty() {
        let mut group_entries = IndexMap::new();
        for group in &behaviour.content.content_groups {
            group_entries.insert(
                format!("{CONTENT_GROUP_KEY_PREFIX}{}", group.id),
                ModeEntry::Option(ModeOption {
                    label: group.label.clone(),
                    description: group.description.clone(),
                    option_type: OptionType::Boolean {
                        default: group.enabled_by_default,
                    },
                    optional: false,
                    enabled_by_default: false,
                    show_when: None,
                    needs_permissions: Vec::new(),
                }),
            );
        }
        entries.insert(
            CONTENT_GROUPS_ENTRY_KEY.to_string(),
            ModeEntry::Group(ModeGroup {
                label: CONTENT_GROUPS_ENTRY_LABEL.to_string(),
                description: None,
                show_when: None,
                needs_permissions: Vec::new(),
                entries: group_entries,
            }),
        );
    }

    let pack_has = pack_has_constants(&behaviour.content, behaviour.experience.is_some(), media);

    EffectiveSchema { entries, pack_has }
}

/// The `pack_has_*` facts.
///
/// The list-backed ones are "the pack declares any", but wallpaper and splash are single media
/// references, and a reference is only a promise. These two ask whether the referenced file is
/// really there and really usable: a `wallpaper` pointing at a file that was since deleted, or at
/// a video (`lewdware.wallpaper.set` takes an image), would otherwise show the user a toggle
/// that cannot do anything. Splash accepts video too -- Edgeware's `loading_splash` is very
/// often an animated GIF, which probes as a video once encoded.
fn pack_has_constants(
    content: &Content,
    has_experience: bool,
    media: &dyn MediaLookup,
) -> IndexMap<String, OptionValue> {
    let mut map = IndexMap::new();
    map.insert(
        "pack_has_captions".to_string(),
        OptionValue::Boolean(!content.captions.is_empty()),
    );
    map.insert(
        "pack_has_prompts".to_string(),
        OptionValue::Boolean(!content.prompts.is_empty()),
    );
    map.insert(
        "pack_has_notifications".to_string(),
        OptionValue::Boolean(!content.notifications.is_empty()),
    );
    map.insert(
        "pack_has_subliminals".to_string(),
        OptionValue::Boolean(!content.subliminals.is_empty()),
    );
    map.insert(
        "pack_has_web_links".to_string(),
        OptionValue::Boolean(!content.web_links.is_empty()),
    );
    map.insert(
        "pack_has_content_groups".to_string(),
        OptionValue::Boolean(!content.content_groups.is_empty()),
    );
    map.insert(
        "pack_has_experience".to_string(),
        OptionValue::Boolean(has_experience),
    );
    let resolves_to = |slot: Option<u64>, accepted: &[FileType]| {
        slot.and_then(|media_id| media.file_type(media_id))
            .is_some_and(|file_type| accepted.contains(&file_type))
    };
    map.insert(
        "pack_has_wallpaper".to_string(),
        OptionValue::Boolean(resolves_to(content.wallpaper, &[FileType::Image])),
    );
    map.insert(
        "pack_has_splash".to_string(),
        OptionValue::Boolean(resolves_to(
            content.splash,
            &[FileType::Image, FileType::Video],
        )),
    );
    map
}

/// Resolves the values a default mode actually runs with: stored user values win where present
/// and type-matching, otherwise each option (including synthesized content-group toggles) falls
/// back to its own schema default. Behaviour data supplies no value defaults of its own beyond
/// what's already baked into the synthesized schema (`effective_options`).
pub fn effective_config(
    schema: &EffectiveSchema,
    stored: &HashMap<String, StoredValue>,
) -> HashMap<String, OptionValue> {
    crate::mode::resolve_options(&schema.entries, stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behaviour::schema::{ContentGroup, TextItem};

    fn empty_mode_schema() -> Metadata {
        Metadata {
            name: "test-mode".to_string(),
            version: None,
            author: None,
            entrypoint: "main.lua".to_string(),
            entries: IndexMap::new(),
            files: HashMap::new(),
            needs_permissions: Vec::new(),
        }
    }

    fn mode_schema_with_option() -> Metadata {
        let mut entries = IndexMap::new();
        entries.insert(
            "popup_frequency".to_string(),
            ModeEntry::Option(ModeOption {
                label: "Popup frequency".to_string(),
                description: None,
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
        Metadata {
            name: "test-mode".to_string(),
            version: None,
            author: None,
            entrypoint: "main.lua".to_string(),
            entries,
            files: HashMap::new(),
            needs_permissions: Vec::new(),
        }
    }

    fn behaviour_with_groups(groups: Vec<ContentGroup>) -> Behaviour {
        let mut behaviour = Behaviour::new();
        behaviour.content.content_groups = groups;
        behaviour
    }

    #[test]
    fn no_content_groups_means_no_synthesized_entry() {
        let schema = effective_options(&empty_mode_schema(), &Behaviour::new(), &no_media);
        assert!(!schema.entries.contains_key(CONTENT_GROUPS_ENTRY_KEY));
    }

    #[test]
    fn content_groups_synthesize_one_boolean_option_each() {
        let behaviour = behaviour_with_groups(vec![
            ContentGroup {
                id: "vanilla".to_string(),
                label: "Vanilla".to_string(),
                description: None,
                tags: vec!["vanilla".to_string()],
                enabled_by_default: true,
            },
            ContentGroup {
                id: "kinky".to_string(),
                label: "Kinky".to_string(),
                description: None,
                tags: vec!["kinky".to_string()],
                enabled_by_default: false,
            },
        ]);

        let schema = effective_options(&empty_mode_schema(), &behaviour, &no_media);

        let ModeEntry::Group(group) = schema
            .entries
            .get(CONTENT_GROUPS_ENTRY_KEY)
            .expect("synthesized content_groups entry")
        else {
            panic!("expected a group entry");
        };

        let vanilla_key = format!("{CONTENT_GROUP_KEY_PREFIX}vanilla");
        let kinky_key = format!("{CONTENT_GROUP_KEY_PREFIX}kinky");

        let ModeEntry::Option(vanilla) = group.entries.get(&vanilla_key).unwrap() else {
            panic!("expected an option entry");
        };
        assert_eq!(vanilla.option_type, OptionType::Boolean { default: true });

        let ModeEntry::Option(kinky) = group.entries.get(&kinky_key).unwrap() else {
            panic!("expected an option entry");
        };
        assert_eq!(kinky.option_type, OptionType::Boolean { default: false });
    }

    #[test]
    fn pack_has_facts_reflect_empty_content() {
        let schema = effective_options(&empty_mode_schema(), &Behaviour::new(), &no_media);
        for key in [
            "pack_has_captions",
            "pack_has_prompts",
            "pack_has_notifications",
            "pack_has_subliminals",
            "pack_has_web_links",
            "pack_has_content_groups",
            "pack_has_experience",
            "pack_has_wallpaper",
            "pack_has_splash",
        ] {
            assert_eq!(
                schema.pack_has.get(key),
                Some(&OptionValue::Boolean(false)),
                "expected {key} to be false"
            );
        }
    }

    #[test]
    fn pack_has_facts_reflect_populated_content() {
        let mut behaviour = Behaviour::new();
        behaviour.content.captions = vec![TextItem {
            text: "hi".to_string(),
            tags: vec![],
        }];
        behaviour.content.content_groups = vec![ContentGroup {
            id: "kinky".to_string(),
            label: "Kinky".to_string(),
            description: None,
            tags: vec!["kinky".to_string()],
            enabled_by_default: true,
        }];
        behaviour.experience = Some(Default::default());
        // prompts/notifications/subliminals/web_links left empty deliberately, so each flag
        // is checked independently rather than all flipping together.

        let schema = effective_options(&empty_mode_schema(), &behaviour, &no_media);

        assert_eq!(
            schema.pack_has.get("pack_has_captions"),
            Some(&OptionValue::Boolean(true))
        );
        assert_eq!(
            schema.pack_has.get("pack_has_content_groups"),
            Some(&OptionValue::Boolean(true))
        );
        assert_eq!(
            schema.pack_has.get("pack_has_experience"),
            Some(&OptionValue::Boolean(true))
        );
        assert_eq!(
            schema.pack_has.get("pack_has_prompts"),
            Some(&OptionValue::Boolean(false))
        );
        assert_eq!(
            schema.pack_has.get("pack_has_notifications"),
            Some(&OptionValue::Boolean(false))
        );
        assert_eq!(
            schema.pack_has.get("pack_has_subliminals"),
            Some(&OptionValue::Boolean(false))
        );
        assert_eq!(
            schema.pack_has.get("pack_has_web_links"),
            Some(&OptionValue::Boolean(false))
        );
    }

    /// A `MediaLookup` over a fixed id -> type table.
    fn media_with<'a>(files: &'a [(u64, FileType)]) -> impl MediaLookup + 'a {
        move |media: u64| {
            files
                .iter()
                .find(|(candidate, _)| *candidate == media)
                .map(|&(_, file_type)| file_type)
        }
    }

    fn behaviour_with_slots(wallpaper: Option<u64>, splash: Option<u64>) -> Behaviour {
        let mut behaviour = Behaviour::new();
        behaviour.content.wallpaper = wallpaper;
        behaviour.content.splash = splash;
        behaviour
    }

    fn slot_facts(behaviour: &Behaviour, media: &dyn MediaLookup) -> (bool, bool) {
        let schema = effective_options(&empty_mode_schema(), behaviour, media);
        let fact = |key: &str| match schema.pack_has.get(key) {
            Some(&OptionValue::Boolean(value)) => value,
            other => panic!("expected a boolean {key}, got {other:?}"),
        };
        (fact("pack_has_wallpaper"), fact("pack_has_splash"))
    }

    #[test]
    fn pack_has_wallpaper_and_splash_false_when_no_slot_is_set() {
        // Opt-in only: a pack that never fills either slot gets both facts false, even if it
        // happens to ship a file called "wallpaper.png" -- see `Content`'s doc comment.
        let media = media_with(&[(1, FileType::Image)]);
        assert_eq!(slot_facts(&Behaviour::new(), &media), (false, false));
    }

    #[test]
    fn pack_has_wallpaper_and_splash_true_when_the_slots_resolve() {
        let media = media_with(&[
            (1, FileType::Image),
            (2, FileType::Image),
            (3, FileType::Video),
        ]);

        assert_eq!(
            slot_facts(&behaviour_with_slots(Some(1), Some(2)), &media),
            (true, true)
        );
        // An animated splash is a *video* once encoded, and still a splash.
        assert_eq!(
            slot_facts(&behaviour_with_slots(None, Some(3)), &media),
            (false, true)
        );
    }

    #[test]
    fn pack_has_wallpaper_and_splash_false_when_the_reference_does_not_resolve() {
        // The honesty fix: a reference is only a promise. A slot pointing at a file that isn't in
        // the pack (deleted since, or never imported), or at media of a type the feature can't
        // use, must not offer the user a toggle that can't do anything. `wallpaper.set` takes an
        // image; splash takes an image or a video; audio is neither.
        let media = media_with(&[(1, FileType::Video), (2, FileType::Audio)]);

        assert_eq!(
            slot_facts(&behaviour_with_slots(Some(404), Some(404)), &media),
            (false, false)
        );
        assert_eq!(
            slot_facts(&behaviour_with_slots(Some(1), None), &media),
            (false, false)
        );
        assert_eq!(
            slot_facts(&behaviour_with_slots(None, Some(2)), &media),
            (false, false)
        );
    }

    #[test]
    fn effective_config_prefers_stored_value_for_real_option() {
        let schema = effective_options(&mode_schema_with_option(), &Behaviour::new(), &no_media);
        let mut stored = HashMap::new();
        stored.insert("popup_frequency".to_string(), StoredValue::Float(5.0));

        let resolved = effective_config(&schema, &stored);

        assert_eq!(
            resolved.get("popup_frequency"),
            Some(&OptionValue::Number(5.0))
        );
    }

    #[test]
    fn effective_config_falls_back_to_default_when_missing_or_mismatched() {
        let schema = effective_options(&mode_schema_with_option(), &Behaviour::new(), &no_media);
        let mut stored = HashMap::new();
        stored.insert(
            "popup_frequency".to_string(),
            StoredValue::Str("not a number".to_string()),
        );

        let resolved = effective_config(&schema, &stored);

        assert_eq!(
            resolved.get("popup_frequency"),
            Some(&OptionValue::Number(1.0))
        );
    }

    #[test]
    fn effective_config_resolves_content_group_toggle() {
        let behaviour = behaviour_with_groups(vec![ContentGroup {
            id: "kinky".to_string(),
            label: "Kinky".to_string(),
            description: None,
            tags: vec!["kinky".to_string()],
            enabled_by_default: true,
        }]);
        let schema = effective_options(&empty_mode_schema(), &behaviour, &no_media);
        let key = format!("{CONTENT_GROUP_KEY_PREFIX}kinky");

        // No stored value yet -> falls back to the group's `enabled_by_default`.
        let resolved_default = effective_config(&schema, &HashMap::new());
        assert_eq!(
            resolved_default.get(&key),
            Some(&OptionValue::Boolean(true))
        );

        // User explicitly disabled it -> stored value wins.
        let mut stored = HashMap::new();
        stored.insert(key.clone(), StoredValue::Bool(false));
        let resolved_stored = effective_config(&schema, &stored);
        assert_eq!(
            resolved_stored.get(&key),
            Some(&OptionValue::Boolean(false))
        );
    }

    #[test]
    fn effective_options_preserves_the_mode_schemas_own_entries() {
        // Guards against effective_options silently dropping the mode's own options while
        // synthesizing content-group toggles.
        let behaviour = behaviour_with_groups(vec![ContentGroup {
            id: "kinky".to_string(),
            label: "Kinky".to_string(),
            description: None,
            tags: vec!["kinky".to_string()],
            enabled_by_default: true,
        }]);
        let schema = effective_options(&mode_schema_with_option(), &behaviour, &no_media);

        assert!(schema.entries.contains_key("popup_frequency"));
        assert!(schema.entries.contains_key(CONTENT_GROUPS_ENTRY_KEY));
    }
}
