use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// Data read by both default modes: captions, prompts, notifications, subliminals, web links,
/// the wallpaper/splash media references, and the content groups a user can toggle. See
/// `behaviour-design/behaviour-tab.md` and `behaviour-design/default-mode.md` (Ownership).
///
/// Wallpaper and splash are single media *references*, not tag queries: one named file each,
/// resolved at runtime through `lewdware.media.get`. They were tag lists once, which asked an
/// author to know that `wallpaper` was a mechanical tag and to keep it off ordinary media --
/// while every format they can come from is single-image anyway (Edgeware has one
/// `wallpaper.png`, one `loading_splash.*`, and one wallpaper per corruption level). Both stay
/// opt-in: `None` means the pack doesn't use engine-managed wallpaper/splash at all.
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
    /// Name of the media file used as the desktop wallpaper. `None` means the pack has no
    /// wallpaper feature at all -- see this struct's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<String>,
    /// Name of the media file shown as the startup splash. See `wallpaper`'s doc comment --
    /// same reasoning, except that a splash may also be a video (an animated GIF probes as one;
    /// see `shared/src/encode.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splash: Option<String>,
}

impl Content {
    /// Whether this document describes no content at all: no pools, no groups, and neither media
    /// slot filled.
    ///
    /// `prompt_settings` is deliberately not consulted -- it configures how prompts are presented,
    /// so it says nothing on its own about whether there are any.
    pub fn is_empty(&self) -> bool {
        self.content_groups.is_empty()
            && self.captions.is_empty()
            && self.prompts.is_empty()
            && self.notifications.is_empty()
            && self.subliminals.is_empty()
            && self.web_links.is_empty()
            && self.wallpaper.is_none()
            && self.splash.is_none()
    }
}

/// One place in a behaviour document that names a media file -- the address of a media slot,
/// independent of what (if anything) currently fills it. See `Behaviour::take_media_references`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaSlot {
    /// `Content::wallpaper`.
    Wallpaper,
    /// `Content::splash`.
    Splash,
    /// The wallpaper a timeline stage sets, addressed by stage id rather than by position: a
    /// slot may be filled long after it was read, and ids are what survive editing in between.
    StageWallpaper { stage: String },
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

pub const VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Behaviour {
    pub version: u32,
    #[serde(default)]
    pub content: Content,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<Experience>,
}

impl Behaviour {
    pub fn new() -> Self {
        Self {
            version: VERSION,
            content: Content::default(),
            experience: None,
        }
    }

    pub fn from_json_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }

    pub fn to_json_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec_pretty(self)
    }

    pub fn is_from_newer_engine(&self) -> bool {
        self.version > VERSION
    }

    pub fn validate(&self) -> Vec<ValidationIssue> {
        let Some(experience) = &self.experience else {
            return vec![];
        };
        experience.timeline.validate()
    }

    /// Rewrites every reference to a media tag in this behaviour document.
    ///
    /// `replacement = None` removes the tag. Duplicate references are removed while preserving
    /// their original order, which is important when merging one tag into another.
    pub fn rewrite_tag(&mut self, from: &str, replacement: Option<&str>) {
        fn rewrite(tags: &mut Vec<String>, from: &str, replacement: Option<&str>) {
            let mut seen = HashSet::new();
            tags.retain_mut(|tag| {
                if tag == from {
                    let Some(replacement) = replacement else {
                        return false;
                    };
                    replacement.clone_into(tag);
                }
                seen.insert(tag.clone())
            });
        }

        let content = &mut self.content;
        for group in &mut content.content_groups {
            rewrite(&mut group.tags, from, replacement);
        }
        for item in &mut content.captions {
            rewrite(&mut item.tags, from, replacement);
        }
        for item in &mut content.prompts {
            rewrite(&mut item.tags, from, replacement);
        }
        for item in &mut content.notifications {
            rewrite(&mut item.tags, from, replacement);
        }
        for item in &mut content.subliminals {
            rewrite(&mut item.tags, from, replacement);
        }
        for item in &mut content.web_links {
            rewrite(&mut item.tags, from, replacement);
        }
        // `content.wallpaper`/`splash` and a stage's `wallpaper` are media *names*, not tags --
        // renaming a tag can't touch them. Renaming a media file can; that's
        // `rewrite_media_name`.

        if let Some(experience) = &mut self.experience {
            for stage in &mut experience.timeline.stages {
                if let Some(tags) = &mut stage.content.tags {
                    rewrite(tags, from, replacement);
                }
            }
        }
    }

    /// Rewrites every reference to a media file name in this behaviour document -- the
    /// referential-integrity half of the wallpaper/splash slots (see `Content`).
    ///
    /// `replacement = None` clears the reference, which is what deleting the file means; passing
    /// a name is what renaming it means. Returns whether anything changed, so a caller can skip
    /// re-persisting an untouched document.
    pub fn rewrite_media_name(&mut self, from: &str, replacement: Option<&str>) -> bool {
        let mut changed = false;
        for (_, value) in self.media_reference_slots() {
            if value.as_deref() == Some(from) {
                *value = replacement.map(str::to_string);
                changed = true;
            }
        }
        changed
    }

    /// Empties every media slot, returning what each one pointed at.
    ///
    /// For a writer that can't yet check the names it wants to use: the Edgeware importer knows
    /// which file belongs in which slot long before it knows what that file will end up being
    /// called (a name collision suffixes it, and a duplicate or a failed encode means it never
    /// arrives at all). Taking the references out lets the rest of the document be persisted
    /// immediately, with the slots filled back in by [`Behaviour::fill_media_reference`] once
    /// their files really exist -- so a reference is never written on trust.
    pub fn take_media_references(&mut self) -> Vec<(MediaSlot, String)> {
        self.media_reference_slots()
            .filter_map(|(slot, value)| value.take().map(|name| (slot, name)))
            .collect()
    }

    /// Points `slot` at `name`, if that slot exists and is still empty.
    ///
    /// Only-if-empty because a slot is filled part-way through a long-running import, and the
    /// Content tab is live throughout: a slot the author set themselves in the meantime is their
    /// answer, not one to overwrite. Returns whether it was filled.
    pub fn fill_media_reference(&mut self, slot: &MediaSlot, name: String) -> bool {
        for (candidate, value) in self.media_reference_slots() {
            if &candidate == slot && value.is_none() {
                *value = Some(name);
                return true;
            }
        }
        false
    }

    /// Every slot that holds a media file name, with a mutable handle on it. The single place
    /// that knows where references live, so adding a slot later can't leave one of the operations
    /// above quietly not covering it.
    fn media_reference_slots(&mut self) -> impl Iterator<Item = (MediaSlot, &mut Option<String>)> {
        let stages = self
            .experience
            .iter_mut()
            .flat_map(|experience| experience.timeline.stages.iter_mut())
            .map(|stage| {
                (
                    MediaSlot::StageWallpaper {
                        stage: stage.id.clone(),
                    },
                    &mut stage.content.wallpaper,
                )
            });
        [
            (MediaSlot::Wallpaper, &mut self.content.wallpaper),
            (MediaSlot::Splash, &mut self.content.splash),
        ]
        .into_iter()
        .chain(stages)
    }

    /// Every media file name this document references, in no particular order. Used to decide
    /// whether a file a slot is being cleared from is still needed elsewhere (see the pack
    /// editor's slot lifecycle) -- a base wallpaper reused by a timeline stage is Edgeware's
    /// ordinary case, so "clear this stage's wallpaper" must not take the pack's main one with
    /// it.
    pub fn referenced_media_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = [&self.content.wallpaper, &self.content.splash]
            .into_iter()
            .filter_map(|slot| slot.as_deref())
            .collect();
        if let Some(experience) = &self.experience {
            names.extend(
                experience
                    .timeline
                    .stages
                    .iter()
                    .filter_map(|stage| stage.content.wallpaper.as_deref()),
            );
        }
        names
    }
}

impl Default for Behaviour {
    fn default() -> Self {
        Self::new()
    }
}

/// The `experience` section: Experience-mode-only data. Entirely expressed as a timeline of
/// stages connected by transitions -- see `behaviour-design/default-mode.md`, "Transitions v1".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Experience {
    #[serde(default)]
    pub timeline: Timeline,
    /// Optional per-pack display name for the built-in timeline mode. When a pack ships a
    /// timeline it can label how that mode is presented (`config`'s `build_mode_groups` shows
    /// this in place of the mode's own name) -- e.g. the Edgeware converter emits "Corruption",
    /// matching the term those packs' users already know. `None` falls back to the mode's own
    /// name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Timeline {
    #[serde(default)]
    pub stages: Vec<Stage>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
}

impl Timeline {
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = vec![];
        let mut ids = HashSet::new();
        for (index, stage) in self.stages.iter().enumerate() {
            if !ids.insert(stage.id.as_str()) {
                issues.push(ValidationIssue::error(
                    format!("experience.timeline.stages[{index}].id"),
                    "Stage IDs must be unique",
                ));
            }
            let final_stage = index + 1 == self.stages.len();
            if final_stage && stage.end.is_some() {
                issues.push(ValidationIssue::error(
                    format!("experience.timeline.stages[{index}].end"),
                    "The final stage must run until the session ends",
                ));
            } else if !final_stage && stage.end.is_none() {
                issues.push(ValidationIssue::error(
                    format!("experience.timeline.stages[{index}].end"),
                    "Every non-final stage needs an ending condition",
                ));
            }
            if let Some(end) = &stage.end
                && end.duration_seconds.is_none()
                && end.event_count.is_none()
            {
                issues.push(ValidationIssue::error(
                    format!("experience.timeline.stages[{index}].end"),
                    "An ending condition needs a duration or event count",
                ));
            }
        }
        for (index, pair) in self.stages.windows(2).enumerate() {
            let matches = self
                .transitions
                .iter()
                .filter(|transition| {
                    transition.from_stage == pair[0].id && transition.to_stage == pair[1].id
                })
                .count();
            if matches != 1 {
                issues.push(ValidationIssue::error(
                    format!("experience.timeline.transitions[{index}]"),
                    "Each adjacent pair of stages needs exactly one transition",
                ));
            }
        }
        issues
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Stage {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<StageEnd>,
    #[serde(default)]
    pub content: ContentSelection,
    #[serde(default)]
    pub events: Events,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement: Option<Movement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mitosis: Option<Mitosis>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ContentSelection {
    /// `None` uses all content; `Some([])` deliberately selects none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Name of the media file this stage sets as the wallpaper. `None` retains whatever the
    /// previous stage (or `Content::wallpaper`) left in effect -- an absolute write, not a
    /// delta, which is why one name is enough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Events {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup: Option<EventSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<EventSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<EventSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<EventSchedule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subliminal: Option<EventSchedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventSchedule {
    pub interval: Interval,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_delay_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Interval {
    Fixed {
        seconds: f64,
    },
    Random {
        minimum_seconds: f64,
        maximum_seconds: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Movement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_speed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_speed: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mitosis {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StageEnd {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_count: Option<EventCountCondition>,
    #[serde(default)]
    pub strategy: EndStrategy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventCountCondition {
    pub event: EventKind,
    pub count: u32,
    #[serde(default)]
    pub scope: CountScope,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EndStrategy {
    #[default]
    Any,
    All,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CountScope {
    #[default]
    Stage,
    Session,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Popup,
    Web,
    Notification,
    Prompt,
    Subliminal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Transition {
    pub id: String,
    pub from_stage: String,
    pub to_stage: String,
    #[serde(default)]
    pub duration_seconds: f64,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default)]
    pub affected: Vec<TransitionCategory>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransitionCategory {
    // Kept for documents written by early v3 editor builds. New editor versions
    // use the field-level variants below.
    Events,
    Movement,
    Mitosis,
    PopupInterval,
    WebInterval,
    NotificationInterval,
    PromptInterval,
    SubliminalInterval,
    MovementMinimumSpeed,
    MovementMaximumSpeed,
    MitosisChance,
    MitosisCount,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationIssue {
    pub path: String,
    pub severity: ValidationSeverity,
    pub message: String,
}

impl ValidationIssue {
    fn error(path: String, message: impl Into<String>) -> Self {
        Self {
            path,
            severity: ValidationSeverity::Error,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Error,
    Warning,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallpaper_and_splash_are_opt_in_with_no_mechanical_default() {
        // No mechanical fallback: an author who never fills either slot gets `None`, not an
        // assumed "wallpaper.png"/"loading_splash.png" -- see `Content`'s doc comment.
        let content = Content::default();
        assert_eq!(content.wallpaper, None);
        assert_eq!(content.splash, None);
    }

    fn behaviour_with_slots(wallpaper: &str, splash: &str, stage_wallpaper: &str) -> Behaviour {
        let mut behaviour = Behaviour::new();
        behaviour.content.wallpaper = Some(wallpaper.to_string());
        behaviour.content.splash = Some(splash.to_string());
        behaviour.experience = Some(Experience {
            timeline: Timeline {
                stages: vec![Stage {
                    id: "stage-1".to_string(),
                    label: "Stage 1".to_string(),
                    end: None,
                    content: ContentSelection {
                        tags: None,
                        wallpaper: Some(stage_wallpaper.to_string()),
                    },
                    events: Events::default(),
                    movement: None,
                    mitosis: None,
                }],
                transitions: vec![],
            },
            label: None,
        });
        behaviour
    }

    #[test]
    fn taking_media_references_empties_every_slot_and_reports_where_each_pointed() {
        let mut behaviour = behaviour_with_slots("bg.png", "intro.gif", "bg2.png");

        let taken = behaviour.take_media_references();

        assert_eq!(
            taken,
            vec![
                (MediaSlot::Wallpaper, "bg.png".to_string()),
                (MediaSlot::Splash, "intro.gif".to_string()),
                (
                    MediaSlot::StageWallpaper {
                        stage: "stage-1".to_string()
                    },
                    "bg2.png".to_string()
                ),
            ]
        );
        assert_eq!(behaviour.referenced_media_names(), Vec::<&str>::new());
        // An empty slot has nothing to report, and taking again is a no-op.
        assert!(behaviour.take_media_references().is_empty());
    }

    #[test]
    fn filling_a_media_reference_respects_a_slot_that_is_already_set() {
        // The import fills slots at the end of a long-running job, and the Content tab is live
        // throughout: a slot the author set in the meantime is their answer, not one to replace.
        let mut behaviour = behaviour_with_slots("bg.png", "intro.gif", "bg2.png");
        behaviour.take_media_references();
        behaviour.content.splash = Some("author-picked.png".to_string());

        assert!(behaviour.fill_media_reference(&MediaSlot::Wallpaper, "bg (1).png".to_string()));
        assert!(!behaviour.fill_media_reference(&MediaSlot::Splash, "intro.gif".to_string()));
        assert!(behaviour.fill_media_reference(
            &MediaSlot::StageWallpaper {
                        stage: "stage-1".to_string()
                    },
            "bg2.png".to_string()
        ));
        // A stage deleted while the import ran simply has nowhere to fill.
        assert!(!behaviour.fill_media_reference(
            &MediaSlot::StageWallpaper {
                stage: "gone".to_string()
            },
            "bg2.png".to_string()
        ));

        assert_eq!(behaviour.content.wallpaper.as_deref(), Some("bg (1).png"));
        assert_eq!(
            behaviour.content.splash.as_deref(),
            Some("author-picked.png")
        );
        let stage = &behaviour.experience.as_ref().unwrap().timeline.stages[0];
        assert_eq!(stage.content.wallpaper.as_deref(), Some("bg2.png"));
    }

    #[test]
    fn rewriting_a_media_name_covers_every_slot_that_names_it() {
        // Renaming a file the base wallpaper and a stage both use has to move both, and deleting
        // it has to clear both -- the referential integrity the pack editor's lifecycle needs.
        let mut behaviour = behaviour_with_slots("bg.png", "intro.gif", "bg.png");

        assert!(behaviour.rewrite_media_name("bg.png", Some("backdrop.png")));
        assert_eq!(
            behaviour.referenced_media_names(),
            vec!["backdrop.png", "intro.gif", "backdrop.png"]
        );

        assert!(behaviour.rewrite_media_name("backdrop.png", None));
        assert_eq!(behaviour.referenced_media_names(), vec!["intro.gif"]);
        assert!(!behaviour.rewrite_media_name("backdrop.png", None));
    }

    fn sample_behaviour() -> Behaviour {
        Behaviour {
            version: VERSION,
            content: Content {
                content_groups: vec![ContentGroup {
                    id: "kinky".to_string(),
                    label: "Kinky".to_string(),
                    description: Some("Kinky-tagged content".to_string()),
                    tags: vec!["kinky".to_string()],
                    enabled_by_default: true,
                }],
                captions: vec![TextItem {
                    text: "Obey.".to_string(),
                    tags: vec!["kinky".to_string()],
                }],
                web_links: vec![WebLink {
                    url: "https://duckduckgo.com/?q=".to_string(),
                    args: vec!["edgeware packs".to_string()],
                    tags: vec![],
                }],
                wallpaper: Some("bg.png".to_string()),
                ..Content::default()
            },
            experience: Some(Experience {
                timeline: Timeline {
                    stages: vec![
                        Stage {
                            id: "stage-1".to_string(),
                            label: "Stage 1".to_string(),
                            end: Some(StageEnd {
                                duration_seconds: Some(300.0),
                                event_count: None,
                                strategy: EndStrategy::Any,
                            }),
                            content: ContentSelection::default(),
                            events: Events {
                                popup: Some(EventSchedule {
                                    interval: Interval::Fixed { seconds: 5.0 },
                                    initial_delay_seconds: None,
                                    max_concurrent: None,
                                }),
                                ..Events::default()
                            },
                            movement: None,
                            mitosis: None,
                        },
                        Stage {
                            id: "stage-2".to_string(),
                            label: "Stage 2".to_string(),
                            end: None,
                            content: ContentSelection {
                                tags: Some(vec!["kinky".to_string()]),
                                wallpaper: None,
                            },
                            events: Events::default(),
                            movement: Some(Movement {
                                minimum_speed: Some(50.0),
                                maximum_speed: Some(150.0),
                            }),
                            mitosis: Some(Mitosis {
                                chance: Some(0.5),
                                count: Some(2),
                            }),
                        },
                    ],
                    transitions: vec![Transition {
                        id: "transition-1".to_string(),
                        from_stage: "stage-1".to_string(),
                        to_stage: "stage-2".to_string(),
                        duration_seconds: 0.0,
                        easing: Easing::Linear,
                        affected: vec![],
                    }],
                },
                label: None,
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
    fn minimal_document_parses_with_defaults() {
        let decoded = Behaviour::from_json_bytes(br#"{"version":3}"#).unwrap();
        assert_eq!(decoded.version, 3);
        assert_eq!(decoded.content, Content::default());
        assert_eq!(decoded.experience, None);
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
        behaviour.version = VERSION + 1;
        assert!(behaviour.is_from_newer_engine());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let bytes = br#"{
            "version": 3,
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
    fn rewriting_a_tag_updates_every_behaviour_reference_and_deduplicates_merges() {
        let mut behaviour = Behaviour::from_json_bytes(br#"{
          "version": 3,
          "content": {
            "content_groups": [{"id":"group","label":"Group","tags":["old","new"],"enabled_by_default":true}],
            "captions": [{"text":"Caption","tags":["old"]}],
            "prompts": [{"text":"Prompt","tags":["old"]}],
            "notifications": [{"text":"Notification","tags":["old"]}],
            "subliminals": [{"text":"Subliminal","tags":["old"]}],
            "web_links": [{"url":"https://example.com","tags":["old"]}],
            "wallpaper": "old.png",
            "splash": "old.png"
          },
          "experience": {"timeline":{"stages":[{
            "id":"stage","label":"Stage","content":{"tags":["old"],"wallpaper":"old.png"}
          }],"transitions":[]}}
        }"#).unwrap();

        behaviour.rewrite_tag("old", Some("new"));
        assert_eq!(behaviour.content.content_groups[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.captions[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.prompts[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.notifications[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.subliminals[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.web_links[0].tags, vec!["new"]);
        // Media references are names, not tags: renaming a tag leaves them exactly as they were.
        assert_eq!(behaviour.content.wallpaper.as_deref(), Some("old.png"));
        assert_eq!(behaviour.content.splash.as_deref(), Some("old.png"));
        let stage = &behaviour.experience.as_ref().unwrap().timeline.stages[0];
        assert_eq!(
            stage.content.tags.as_deref(),
            Some(["new".into()].as_slice())
        );
        assert_eq!(stage.content.wallpaper.as_deref(), Some("old.png"));

        behaviour.rewrite_tag("new", None);
        assert!(behaviour.content.content_groups[0].tags.is_empty());
        assert!(
            behaviour.experience.as_ref().unwrap().timeline.stages[0]
                .content
                .tags
                .as_ref()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn validation_reports_final_stage_and_transition_structure() {
        let mut behaviour = Behaviour::new();
        behaviour.experience = Some(Experience {
            timeline: Timeline {
                stages: vec![Stage {
                    id: "only".into(),
                    label: "Only".into(),
                    end: Some(StageEnd {
                        duration_seconds: Some(10.0),
                        event_count: None,
                        strategy: EndStrategy::Any,
                    }),
                    content: Default::default(),
                    events: Default::default(),
                    movement: None,
                    mitosis: None,
                }],
                transitions: vec![],
            },
            label: None,
        });
        assert_eq!(
            behaviour.validate()[0].path,
            "experience.timeline.stages[0].end"
        );
    }
}
