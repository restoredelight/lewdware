use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{Content, schema as legacy};

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
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1);
        if version >= VERSION as u64 {
            serde_json::from_value(value)
        } else {
            let old: legacy::Behaviour = serde_json::from_value(value)?;
            Ok(old.into())
        }
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
        rewrite(&mut content.wallpaper_tags, from, replacement);
        rewrite(&mut content.splash_tags, from, replacement);

        if let Some(experience) = &mut self.experience {
            for stage in &mut experience.timeline.stages {
                if let Some(tags) = &mut stage.content.tags {
                    rewrite(tags, from, replacement);
                }
                if let Some(tags) = &mut stage.content.wallpaper_tags {
                    rewrite(tags, from, replacement);
                }
            }
        }
    }
}

impl Default for Behaviour {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Experience {
    #[serde(default)]
    pub timeline: Timeline,
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
                && end.duration_seconds.is_none() && end.event_count.is_none() {
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
    /// `None` retains the pack-level wallpaper selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper_tags: Option<Vec<String>>,
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

fn schedule(seconds: Option<f64>) -> Option<EventSchedule> {
    seconds.map(|seconds| EventSchedule {
        interval: Interval::Fixed { seconds },
        initial_delay_seconds: None,
        max_concurrent: None,
    })
}

fn movement(design: &legacy::DesignValues) -> Option<Movement> {
    match (design.movement_speed_min, design.movement_speed_max) {
        (None, None) => None,
        (minimum_speed, maximum_speed) => Some(Movement {
            minimum_speed,
            maximum_speed,
        }),
    }
}

fn mitosis(design: &legacy::DesignValues) -> Option<Mitosis> {
    match (design.mitosis_chance, design.mitosis_count) {
        (None, None) => None,
        (chance, count) => Some(Mitosis { chance, count }),
    }
}

impl From<legacy::Behaviour> for Behaviour {
    fn from(old: legacy::Behaviour) -> Self {
        let experience = old.experience.map(|experience| {
            let levels = experience.timeline.levels;
            let stages = levels
                .iter()
                .enumerate()
                .map(|(index, level)| {
                    let next = levels.get(index + 1);
                    let end = next.map(|next| StageEnd {
                        duration_seconds: Some((next.at_seconds - level.at_seconds).max(0.0)),
                        event_count: next.at_popups.map(|count| EventCountCondition {
                            event: EventKind::Popup,
                            count,
                            scope: CountScope::Session,
                        }),
                        strategy: EndStrategy::Any,
                    });
                    Stage {
                        id: format!("stage-{}", index + 1),
                        label: format!("Stage {}", index + 1),
                        end,
                        content: ContentSelection {
                            tags: level.tags.clone(),
                            wallpaper_tags: level.wallpaper_tags.clone(),
                        },
                        events: Events {
                            popup: schedule(level.anchors.popup),
                            web: schedule(level.anchors.web),
                            notification: schedule(level.anchors.notification),
                            prompt: schedule(level.anchors.prompt),
                            subliminal: schedule(level.anchors.subliminal),
                        },
                        movement: movement(&level.design),
                        mitosis: mitosis(&level.design),
                    }
                })
                .collect::<Vec<_>>();
            let transitions = stages
                .windows(2)
                .enumerate()
                .map(|(index, pair)| Transition {
                    id: format!("transition-{}", index + 1),
                    from_stage: pair[0].id.clone(),
                    to_stage: pair[1].id.clone(),
                    duration_seconds: 0.0,
                    easing: Easing::Linear,
                    affected: vec![],
                })
                .collect();
            Experience {
                timeline: Timeline {
                    stages,
                    transitions,
                },
            }
        });
        Self {
            version: VERSION,
            content: old.content,
            experience,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_v2_levels_without_losing_complete_stage_values() {
        let bytes = br#"{
          "version": 2,
          "content": {"captions": [{"text": "Hello", "tags": []}]},
          "experience": {"timeline": {"levels": [
            {"at_seconds":0,"anchors":{"popup":30},"design":{"movement_speed_min":50,"movement_speed_max":100}},
            {"at_seconds":300,"at_popups":20,"anchors":{"popup":10},"design":{"mitosis_chance":0.5,"mitosis_count":2},"tags":["intense"]}
          ]}}
        }"#;
        let migrated = Behaviour::from_json_bytes(bytes).unwrap();
        assert_eq!(migrated.version, VERSION);
        assert_eq!(migrated.content.captions[0].text, "Hello");
        let timeline = &migrated.experience.as_ref().unwrap().timeline;
        assert_eq!(timeline.stages.len(), 2);
        assert_eq!(timeline.transitions.len(), 1);
        assert_eq!(
            timeline.stages[0].end.as_ref().unwrap().duration_seconds,
            Some(300.0)
        );
        assert_eq!(
            timeline.stages[0]
                .end
                .as_ref()
                .unwrap()
                .event_count
                .as_ref()
                .unwrap()
                .scope,
            CountScope::Session
        );
        assert!(timeline.stages[1].end.is_none());
        assert_eq!(
            timeline.stages[1].content.tags,
            Some(vec!["intense".into()])
        );
        assert_eq!(
            timeline.stages[1].mitosis,
            Some(Mitosis {
                chance: Some(0.5),
                count: Some(2)
            })
        );
        assert!(migrated.validate().is_empty());
    }

    #[test]
    fn v3_roundtrips_without_running_the_migration_again() {
        let migrated = Behaviour::from_json_bytes(
            br#"{"version":1,"experience":{"timeline":{"levels":[{}]}}}"#,
        )
        .unwrap();
        let decoded = Behaviour::from_json_bytes(&migrated.to_json_bytes().unwrap()).unwrap();
        assert_eq!(decoded, migrated);
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
            "wallpaper_tags": ["old"],
            "splash_tags": ["old"]
          },
          "experience": {"timeline":{"stages":[{
            "id":"stage","label":"Stage","content":{"tags":["old"],"wallpaper_tags":["old"]}
          }],"transitions":[]}}
        }"#).unwrap();

        behaviour.rewrite_tag("old", Some("new"));
        assert_eq!(behaviour.content.content_groups[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.captions[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.prompts[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.notifications[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.subliminals[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.web_links[0].tags, vec!["new"]);
        assert_eq!(behaviour.content.wallpaper_tags, vec!["new"]);
        assert_eq!(behaviour.content.splash_tags, vec!["new"]);
        let stage = &behaviour.experience.as_ref().unwrap().timeline.stages[0];
        assert_eq!(
            stage.content.tags.as_deref(),
            Some(["new".into()].as_slice())
        );
        assert_eq!(
            stage.content.wallpaper_tags.as_deref(),
            Some(["new".into()].as_slice())
        );

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
        });
        assert_eq!(
            behaviour.validate()[0].path,
            "experience.timeline.stages[0].end"
        );
    }
}
