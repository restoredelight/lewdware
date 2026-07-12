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
/// values, the transition timeline). Deliberately an empty stub in this milestone — M4 owns
/// its contents (`design/release-plan.md`). Any fields added later must be `#[serde(default)]`
/// so this document's shape stays additive-only and never needs restructuring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Experience {}

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
            experience: Some(Experience::default()),
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
