use serde::{Deserialize, Serialize};

/// The current behaviour.json schema version. Bump when making a breaking change to this
/// document's shape; consumers should warn (not fail) when reading a document whose `version`
/// is newer than this (see `behaviour-design/behaviour-tab.md`: "Consumers ignore unknown
/// fields; dev mode warns on major mismatch").
pub const CURRENT_VERSION: u32 = 1;

/// A pack's behaviour.json document: the private data contract consumed by the engine's
/// built-in default modes (Sandbox & Experience). Stored as a `pack_data` row named
/// `"behaviour"`, serialized as pretty-printed JSON (not the `ciborium` binary format used for
/// `.lwmode`/`.lwpack` framing) so it stays diffable in golden-file tests and hand-inspectable
/// while debugging — see `behaviour-design/behaviour-tab.md`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Behaviour {
    pub version: u32,
    #[serde(default)]
    pub content: Content,
    /// Present iff the pack has an `experience` section — this is exactly what
    /// `pack_has_experience` and `RecommendedMode`'s Sandbox/Experience default key off, so
    /// presence must stay structurally distinguishable from "an empty section".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<Experience>,
}

impl Behaviour {
    pub fn new() -> Self {
        Self {
            version: CURRENT_VERSION,
            content: Content::default(),
            experience: None,
        }
    }

    /// Whether this document declares a schema version newer than the engine understands.
    pub fn is_from_newer_engine(&self) -> bool {
        self.version > CURRENT_VERSION
    }

    pub fn to_json_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec_pretty(self)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

impl Default for Behaviour {
    fn default() -> Self {
        Self::new()
    }
}

/// Data read by both default modes: captions, prompts, notifications, subliminals, web links,
/// wallpaper/splash tags, and the content groups a user can toggle. See
/// `behaviour-design/behaviour-tab.md` and `behaviour-design/default-mode.md` (Ownership).
///
/// Wallpaper/splash are pure tagged media -- `wallpaper_tags`/`splash_tags` name which tags
/// identify that media, rather than carrying the media itself. Both are opt-in like every other
/// field here: empty means the pack doesn't use engine-managed wallpaper/splash at all, full
/// stop -- there is deliberately no mechanical fallback tag (e.g. assuming anything tagged
/// `"wallpaper"` is wallpaper media), since a pack author using that word for an unrelated
/// organizational tag would otherwise get surprise behaviour they never asked for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Content {
    #[serde(default)]
    pub content_groups: Vec<ContentGroup>,
    #[serde(default)]
    pub captions: Vec<TextItem>,
    #[serde(default)]
    pub prompts: Vec<TextItem>,
    #[serde(default)]
    pub prompt_settings: PromptSettings,
    #[serde(default)]
    pub notifications: Vec<TextItem>,
    #[serde(default)]
    pub subliminals: Vec<TextItem>,
    #[serde(default)]
    pub web_links: Vec<WebLink>,
    /// Tags identifying wallpaper media. Empty means the pack has no wallpaper feature at all --
    /// see this struct's doc comment.
    #[serde(default)]
    pub wallpaper_tags: Vec<String>,
    /// Tags identifying splash media. See `wallpaper_tags`'s doc comment -- same reasoning.
    #[serde(default)]
    pub splash_tags: Vec<String>,
}

/// A single content-pool entry, taggable independently of any other entry in the same pool
/// (unlike Edgeware's one-mood-per-item model, an item here can carry any number of tags — or
/// none, matching lewdware's general tag model).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextItem {
    pub text: String,
    /// Which media tags this applies to; empty means "applies regardless of media/context"
    /// (subsumes Edgeware's separate `default` bucket).
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebLink {
    pub url: String,
    /// Suffixes randomly appended to `url` when opened (e.g. search-query terms) — carried
    /// over from Edgeware's `Web{url, args}`, which is how a pack varies what a link opens
    /// without always hitting the exact same page. Empty means open `url` unmodified.
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PromptSettings {
    /// Submit-button label override, rendered via `popup.dialog`.
    #[serde(default)]
    pub submit_label: Option<String>,
}

/// A named, described, user-toggleable set of tags. See `behaviour-design/default-mode.md`
/// (Ownership): "named, described sets of tags with `enabled_by_default`". The resolver
/// synthesizes one boolean mode option per group (see `behaviour::resolver`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContentGroup {
    /// Stable id used as the synthesized option key's suffix; the converter uses the Edgeware
    /// mood name.
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// OR'd: media matching any of these tags belongs to the group.
    pub tags: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled_by_default: bool,
}

fn default_true() -> bool {
    true
}

/// The `experience` section: Experience-mode-only data (frequency anchors, non-rate design
/// values, the transition timeline). Anchors and design values alone already make a
/// statically-designed pack; `timeline` is optional on top of that
/// (`behaviour-design/default-mode.md`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Experience {
    #[serde(default)]
    pub anchors: FrequencyAnchors,
    #[serde(default)]
    pub design: DesignValues,
    /// Present iff the pack designs a transition arc. Absent means every level-derived getter
    /// (`shared/lib`'s Lua timeline module) stays at level 0 (baseline) forever -- no timers, no
    /// tag/wallpaper narrowing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeline: Option<Timeline>,
}

/// Author-set events-per-time baselines for Experience's rate-based features, expressed as
/// seconds-between-events (matching Sandbox's `*_frequency` option convention -- see
/// `behaviour-design/behaviour-tab.md`). `None` means the pack doesn't drive that feature in
/// Experience at all: the process never starts, regardless of the user's pacing scalar --
/// distinct from "runs at some default rate", matching behaviour.json's "no defaults injection"
/// rule (`behaviour-design/behaviour-tab.md`'s resolver section) and rule 5's "empty means skip"
/// spirit generalized to "absent means this feature doesn't exist here".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FrequencyAnchors {
    #[serde(default)]
    pub popup: Option<f64>,
    #[serde(default)]
    pub web: Option<f64>,
    #[serde(default)]
    pub notification: Option<f64>,
    #[serde(default)]
    pub prompt: Option<f64>,
    #[serde(default)]
    pub subliminal: Option<f64>,
}

/// Author-set non-rate baselines -- values a pacing scalar has no meaning for (movement speed,
/// mitosis chance/count), consumed by Experience's processes exactly as Sandbox's user-set
/// equivalents are. `None` means the pack doesn't drive that feature in Experience -- same
/// absent-means-off convention as `FrequencyAnchors`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DesignValues {
    #[serde(default)]
    pub movement_speed_min: Option<f64>,
    #[serde(default)]
    pub movement_speed_max: Option<f64>,
    #[serde(default)]
    pub mitosis_chance: Option<f64>,
    #[serde(default)]
    pub mitosis_count: Option<u32>,
}

/// The transition arc: an ordered sequence of levels the Experience timeline advances through
/// over a session. Progress is session-scoped (no storage dependency) -- a fresh session always
/// starts at the implicit level 0 (baseline: anchors/design as authored, unrestricted tags,
/// `Content::wallpaper_tags`), matching Edgeware's corruption semantics. See
/// `behaviour-design/default-mode.md`, "Transitions v1".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Timeline {
    /// Ascending by `at_seconds`. Index 0 is "level 1" -- level 0 itself is implicit and needs no
    /// entry (every `Modifiers` field absent is exactly the baseline).
    #[serde(default)]
    pub levels: Vec<Level>,
}

/// One level of the timeline: a trigger plus the absolute modifier set it applies once reached.
/// Levels are absolute snapshots relative to *baseline*, never deltas relative to the previous
/// level -- authoring and testing stay order-independent (see `Modifiers`'s doc comment).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Level {
    /// Cumulative active-time seconds since session start at which this level is reached at the
    /// latest. Required (not `Option`) -- structurally guarantees interaction rule 6's "every
    /// trigger needs a time-based fallback": a level can never hard-depend on popup count alone,
    /// even if the user has popups entirely disabled.
    pub at_seconds: f64,
    /// Optional early-advance trigger: reached once this many cumulative popups have spawned
    /// (since session start), if that happens before `at_seconds`. `None` means time is the only
    /// trigger for this level.
    #[serde(default)]
    pub at_popups: Option<u32>,
    #[serde(default)]
    pub modifiers: Modifiers,
}

/// A level's modifier set: "tag changes + wallpaper + relative modifiers on baselines + absolute
/// writes on mode parameters" (`behaviour-design/default-mode.md`, Ownership). Every field is
/// absolute *relative to baseline* -- an absent field means "same as baseline", not "same as the
/// previous level" -- so a level's effective params are a pure function of that level alone, with
/// no dependency on transition history (order-independent: reaching level 3 directly produces the
/// same params as passing through 1 and 2 first).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Modifiers {
    /// Multiplies every baseline this level touches: rate anchors (`FrequencyAnchors`) *and*
    /// non-rate design values (`DesignValues`) alike -- see "Modifier composition" in
    /// `behaviour-design/default-mode.md`. The user's pacing scalar composes on top for rates only
    /// (unaffected by this field). `None` => 1.0 (baseline, unmodified).
    #[serde(default)]
    pub modifier: Option<f64>,
    /// Absolute active tag set from this level (mode parameter): an `any`-style eligibility
    /// restriction on media/content queries, the same mechanism `Content::wallpaper_tags` already
    /// uses. `None` => unrestricted (baseline: the pack's full tag vocabulary).
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Absolute wallpaper-tag override for this level (mode parameter). `None` => no override --
    /// `Content::wallpaper_tags` (baseline) stays in effect.
    #[serde(default)]
    pub wallpaper_tags: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_behaviour() -> Behaviour {
        Behaviour {
            version: CURRENT_VERSION,
            content: Content {
                content_groups: vec![ContentGroup {
                    id: "kinky".to_string(),
                    label: "Kinky".to_string(),
                    description: Some("Kinky-tagged content".to_string()),
                    tags: vec!["kinky".to_string()],
                    enabled_by_default: true,
                }],
                captions: vec![
                    TextItem {
                        text: "Obey.".to_string(),
                        tags: vec!["kinky".to_string()],
                    },
                    TextItem {
                        text: "Untagged caption".to_string(),
                        tags: vec![],
                    },
                ],
                prompts: vec![TextItem {
                    text: "Type below".to_string(),
                    tags: vec![],
                }],
                prompt_settings: PromptSettings {
                    submit_label: Some("Submit".to_string()),
                },
                notifications: vec![TextItem {
                    text: "A notification".to_string(),
                    tags: vec![],
                }],
                subliminals: vec![TextItem {
                    text: "Obey".to_string(),
                    tags: vec!["hypno".to_string()],
                }],
                web_links: vec![WebLink {
                    url: "https://duckduckgo.com/?q=".to_string(),
                    args: vec!["edgeware packs".to_string(), "rule 34".to_string()],
                    tags: vec![],
                }],
                wallpaper_tags: vec!["bg".to_string()],
                splash_tags: vec![],
            },
            experience: Some(Experience {
                anchors: FrequencyAnchors {
                    popup: Some(5.0),
                    web: Some(300.0),
                    notification: None,
                    prompt: Some(90.0),
                    subliminal: None,
                },
                design: DesignValues {
                    movement_speed_min: Some(50.0),
                    movement_speed_max: Some(150.0),
                    mitosis_chance: Some(0.5),
                    mitosis_count: Some(2),
                },
                timeline: Some(Timeline {
                    levels: vec![
                        Level {
                            at_seconds: 300.0,
                            at_popups: Some(20),
                            modifiers: Modifiers {
                                modifier: Some(1.5),
                                tags: Some(vec!["kinky".to_string()]),
                                wallpaper_tags: None,
                            },
                        },
                        Level {
                            at_seconds: 900.0,
                            at_popups: None,
                            modifiers: Modifiers {
                                modifier: Some(3.0),
                                tags: Some(vec!["kinky".to_string(), "hypno".to_string()]),
                                wallpaper_tags: Some(vec!["corrupted-bg".to_string()]),
                            },
                        },
                    ],
                }),
            }),
        }
    }

    #[test]
    fn full_roundtrip() {
        let original = sample_behaviour();
        let bytes = original.to_json_bytes().unwrap();
        let decoded = Behaviour::from_json_bytes(&bytes).unwrap();
        assert_eq!(original, decoded);
    }

    #[test]
    fn wallpaper_and_splash_tags_are_opt_in_with_no_mechanical_default() {
        // No mechanical fallback: an author who never declares wallpaper_tags/splash_tags gets an
        // empty list, not an assumed "wallpaper"/"splash" tag -- see `Content`'s doc comment.
        let content = Content::default();
        assert_eq!(content.wallpaper_tags, Vec::<String>::new());
        assert_eq!(content.splash_tags, Vec::<String>::new());
    }

    #[test]
    fn minimal_document_roundtrips_to_defaults() {
        let decoded = Behaviour::from_json_bytes(br#"{"version":1}"#).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.content, Content::default());
        assert_eq!(decoded.experience, None);
    }

    #[test]
    fn experience_anchors_and_design_values_roundtrip() {
        let original = sample_behaviour().experience.unwrap();
        let bytes = serde_json::to_vec(&original).unwrap();
        let decoded: Experience = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(original, decoded);
        assert_eq!(decoded.anchors.popup, Some(5.0));
        assert_eq!(decoded.anchors.notification, None);
        assert_eq!(decoded.design.mitosis_count, Some(2));
    }

    #[test]
    fn experience_section_with_no_anchors_or_design_still_present() {
        // A `experience: {}` document (just enough to be recommended Experience) is a valid,
        // if inert, statically-designed pack: every anchor absent means every rate feature
        // simply doesn't run -- not an error (see `FrequencyAnchors`'s doc comment).
        let decoded = Behaviour::from_json_bytes(br#"{"version":1,"experience":{}}"#).unwrap();
        let experience = decoded
            .experience
            .expect("experience section should be present");
        assert_eq!(experience.anchors, FrequencyAnchors::default());
        assert_eq!(experience.design, DesignValues::default());
        assert_eq!(experience.timeline, None);
    }

    #[test]
    fn timeline_roundtrips_with_levels() {
        let original = sample_behaviour().experience.unwrap().timeline.unwrap();
        let bytes = serde_json::to_vec(&original).unwrap();
        let decoded: Timeline = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(original, decoded);
        assert_eq!(decoded.levels.len(), 2);
        assert_eq!(decoded.levels[0].at_seconds, 300.0);
        assert_eq!(decoded.levels[0].at_popups, Some(20));
        assert_eq!(decoded.levels[1].at_popups, None);
        assert_eq!(decoded.levels[1].modifiers.modifier, Some(3.0));
    }

    #[test]
    fn timeline_absent_by_default_and_survives_roundtrip() {
        // An experience section with anchors/design but no timeline (the common case pre-Transitions
        // authoring, or a purely statically-designed pack) must keep `timeline: None`, not
        // synthesize `Some(Timeline::default())` -- same discipline as `Behaviour::experience`
        // itself (see `experience_presence_is_distinguishable_from_absence`).
        let mut experience = sample_behaviour().experience.unwrap();
        experience.timeline = None;
        let bytes = serde_json::to_vec(&experience).unwrap();
        let decoded: Experience = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.timeline, None);
    }

    #[test]
    fn level_modifiers_default_to_baseline_when_absent() {
        // Every `Modifiers` field is absolute-relative-to-baseline, not sticky across levels: a
        // level that sets nothing is exactly the baseline (modifier 1.0 conceptually, unrestricted
        // tags, no wallpaper override) -- see `Modifiers`'s doc comment on order-independence.
        let level: Level = serde_json::from_str(r#"{"at_seconds": 60.0}"#).unwrap();
        assert_eq!(level.at_popups, None);
        assert_eq!(level.modifiers, Modifiers::default());
        assert_eq!(level.modifiers.modifier, None);
        assert_eq!(level.modifiers.tags, None);
        assert_eq!(level.modifiers.wallpaper_tags, None);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let bytes = br#"{
            "version": 1,
            "totallyUnknownTopLevelField": 123,
            "content": {
                "captions": [{"text": "hi", "tags": [], "unknownItemField": true}],
                "unknownContentField": "whatever"
            }
        }"#;
        let decoded = Behaviour::from_json_bytes(bytes).unwrap();
        assert_eq!(decoded.content.captions.len(), 1);
        assert_eq!(decoded.content.captions[0].text, "hi");
    }

    #[test]
    fn empty_tags_roundtrip_distinctly_from_tagged() {
        let original = Behaviour {
            version: CURRENT_VERSION,
            content: Content {
                captions: vec![
                    TextItem {
                        text: "applies to everything".to_string(),
                        tags: vec![],
                    },
                    TextItem {
                        text: "only kinky".to_string(),
                        tags: vec!["kinky".to_string()],
                    },
                ],
                ..Default::default()
            },
            experience: None,
        };
        let decoded = Behaviour::from_json_bytes(&original.to_json_bytes().unwrap()).unwrap();
        assert!(decoded.content.captions[0].tags.is_empty());
        assert_eq!(decoded.content.captions[1].tags, vec!["kinky".to_string()]);
    }

    #[test]
    fn experience_presence_is_distinguishable_from_absence() {
        let with_experience = Behaviour {
            experience: Some(Experience::default()),
            ..Behaviour::new()
        };
        let without_experience = Behaviour::new();

        assert!(with_experience.experience.is_some());
        assert!(without_experience.experience.is_none());

        // And it round-trips: an absent section stays absent, not synthesized as Some(default).
        let decoded =
            Behaviour::from_json_bytes(&without_experience.to_json_bytes().unwrap()).unwrap();
        assert_eq!(decoded.experience, None);
    }

    #[test]
    fn is_from_newer_engine() {
        let mut behaviour = Behaviour::new();
        assert!(!behaviour.is_from_newer_engine());
        behaviour.version = CURRENT_VERSION + 1;
        assert!(behaviour.is_from_newer_engine());
    }
}
