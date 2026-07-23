use std::collections::HashMap;

use indexmap::IndexMap;

use crate::mode::{Metadata, ModeEntry, ModeGroup, ModeOption, OptionType, OptionValue};

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

/// Synthesizes the schema a default mode actually presents for a given pack: the mode's own
/// options plus one boolean toggle per content group (rendered as a single "Content" checklist),
/// and the `pack_has_*` facts used to drive `show_when` visibility elsewhere in the schema.
/// Injects no option *defaults* from behaviour data — a content group's `enabled_by_default`
/// becomes the synthesized option's own schema default, not a behaviour-data override.
pub fn effective_options(mode_schema: &Metadata, behaviour: &Behaviour) -> EffectiveSchema {
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

    let pack_has = pack_has_constants(&behaviour.content, behaviour.experience.is_some());

    EffectiveSchema { entries, pack_has }
}

fn pack_has_constants(content: &Content, has_experience: bool) -> IndexMap<String, OptionValue> {
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
    map.insert(
        "pack_has_wallpaper".to_string(),
        OptionValue::Boolean(!content.wallpaper_tags.is_empty()),
    );
    map.insert(
        "pack_has_splash".to_string(),
        OptionValue::Boolean(!content.splash_tags.is_empty()),
    );
    map
}

/// Resolves the values a default mode actually runs with: stored user values win where present
/// and type-matching, otherwise each option (including synthesized content-group toggles) falls
/// back to its own schema default. Behaviour data supplies no value defaults of its own beyond
/// what's already baked into the synthesized schema (`effective_options`).
pub fn effective_config(
    schema: &EffectiveSchema,
    stored: &HashMap<String, OptionValue>,
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
        let schema = effective_options(&empty_mode_schema(), &Behaviour::new());
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

        let schema = effective_options(&empty_mode_schema(), &behaviour);

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
        let schema = effective_options(&empty_mode_schema(), &Behaviour::new());
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

        let schema = effective_options(&empty_mode_schema(), &behaviour);

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

    #[test]
    fn pack_has_wallpaper_false_when_not_declared() {
        // No mechanical fallback: a pack that never sets wallpaper_tags/splash_tags in
        // behaviour.json gets `pack_has_wallpaper`/`pack_has_splash` = false, even if it happens
        // to tag some media "wallpaper"/"splash" -- see `Content`'s doc comment on why there's no
        // guessing here (opt-in only).
        let schema = effective_options(&empty_mode_schema(), &Behaviour::new());
        assert_eq!(
            schema.pack_has.get("pack_has_wallpaper"),
            Some(&OptionValue::Boolean(false))
        );
        assert_eq!(
            schema.pack_has.get("pack_has_splash"),
            Some(&OptionValue::Boolean(false))
        );
    }

    #[test]
    fn pack_has_wallpaper_true_once_author_declares_tags() {
        let mut behaviour = Behaviour::new();
        behaviour.content.wallpaper_tags = vec!["bg".to_string()];
        behaviour.content.splash_tags = vec!["intro".to_string()];

        let schema = effective_options(&empty_mode_schema(), &behaviour);
        assert_eq!(
            schema.pack_has.get("pack_has_wallpaper"),
            Some(&OptionValue::Boolean(true))
        );
        assert_eq!(
            schema.pack_has.get("pack_has_splash"),
            Some(&OptionValue::Boolean(true))
        );
    }

    #[test]
    fn effective_config_prefers_stored_value_for_real_option() {
        let schema = effective_options(&mode_schema_with_option(), &Behaviour::new());
        let mut stored = HashMap::new();
        stored.insert("popup_frequency".to_string(), OptionValue::Number(5.0));

        let resolved = effective_config(&schema, &stored);

        assert_eq!(
            resolved.get("popup_frequency"),
            Some(&OptionValue::Number(5.0))
        );
    }

    #[test]
    fn effective_config_falls_back_to_default_when_missing_or_mismatched() {
        let schema = effective_options(&mode_schema_with_option(), &Behaviour::new());
        let mut stored = HashMap::new();
        stored.insert(
            "popup_frequency".to_string(),
            OptionValue::String("not a number".to_string()),
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
        let schema = effective_options(&empty_mode_schema(), &behaviour);
        let key = format!("{CONTENT_GROUP_KEY_PREFIX}kinky");

        // No stored value yet -> falls back to the group's `enabled_by_default`.
        let resolved_default = effective_config(&schema, &HashMap::new());
        assert_eq!(
            resolved_default.get(&key),
            Some(&OptionValue::Boolean(true))
        );

        // User explicitly disabled it -> stored value wins.
        let mut stored = HashMap::new();
        stored.insert(key.clone(), OptionValue::Boolean(false));
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
        let schema = effective_options(&mode_schema_with_option(), &behaviour);

        assert!(schema.entries.contains_key("popup_frequency"));
        assert!(schema.entries.contains_key(CONTENT_GROUPS_ENTRY_KEY));
    }
}
