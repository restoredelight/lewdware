//! Addressing one place in a behaviour document, so an edit can be described instead of sent.
//!
//! The pack editor used to save behaviour by handing the backend a whole `Behaviour` it had
//! mutated in memory. That made the front end a second writer of a document the backend also
//! edits (media slots, tag renames, file removal), and every defect in the reconciliation between
//! the two came from the same place: a whole-document write says nothing about *what* changed, so
//! it can only be applied by overwriting -- including over a change the backend made in the
//! meantime. See `design/behaviour-storage.md`.
//!
//! A [`Patch`] names a path and a value. The backend applies it to its own current document, so an
//! edit to `content.captions` cannot disturb a wallpaper slot filled a moment earlier, and the
//! author's action arrives with enough context to label its own undo entry.
//!
//! Paths are dot-separated: `content.prompt_settings.submit_label`,
//! `experience.timeline.stages.2.label`. A segment that parses as a number indexes an array;
//! anything else is an object key. Structural changes (adding a caption, removing a stage,
//! reordering) are expressed as a patch replacing the whole array -- coarse, but an array is a
//! fraction of the document, and those actions are single clicks that never need coalescing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::schema::Behaviour;

/// One edit: the value at `path` becomes `value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub path: String,
    pub value: Value,
}

impl Patch {
    pub fn new(path: impl Into<String>, value: Value) -> Self {
        Self {
            path: path.into(),
            value,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum PatchError {
    EmptyPath,
    /// The document has nothing at an intermediate segment. Reported rather than created: a path
    /// into a section that isn't there (`experience.…` while the timeline is suspended) is a stale
    /// editor acting on a document that has moved, and inventing the section to hold the value
    /// would resurrect it.
    NoSuchPath {
        path: String,
        segment: String,
    },
    IndexOutOfRange {
        path: String,
        index: usize,
        length: usize,
    },
    /// A path descending through a string, number or null -- only objects and arrays have
    /// anything under them.
    NotAContainer {
        path: String,
        segment: String,
    },
    /// The patched document is no longer a `Behaviour` -- a wrongly typed value, or one missing a
    /// field its variant needs. Caught here so an unusable document is never written.
    Invalid(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => write!(f, "a behaviour patch needs a path"),
            Self::NoSuchPath { path, segment } => {
                write!(f, "no “{segment}” in the behaviour document at “{path}”")
            }
            Self::IndexOutOfRange {
                path,
                index,
                length,
            } => write!(
                f,
                "“{path}” is index {index} of an array with {length} entries"
            ),
            Self::NotAContainer { path, segment } => {
                write!(f, "“{path}” has no “{segment}” to address")
            }
            Self::Invalid(error) => {
                write!(f, "that edit does not fit the behaviour document: {error}")
            }
        }
    }
}

impl std::error::Error for PatchError {}

impl Behaviour {
    /// This document with every patch applied in order, or an error leaving it untouched.
    ///
    /// All-or-nothing on purpose: the patches in one call are one author action (the fields they
    /// touched before pausing), so half of it is not a state they ever asked for.
    pub fn patched(&self, patches: &[Patch]) -> Result<Behaviour, PatchError> {
        let mut document = serde_json::to_value(self)
            .expect("a Behaviour always serializes: every field is a plain data type");
        for patch in patches {
            set_at_path(&mut document, &patch.path, patch.value.clone())?;
        }
        serde_json::from_value(document).map_err(|error| PatchError::Invalid(error.to_string()))
    }
}

/// Walks `document` to the parent of `path` and writes `value` into the final segment.
///
/// A missing final key on an object is *written*, not rejected: the fields that serialize away
/// when empty (`content.wallpaper`, a content group's `description`) are absent from the document
/// exactly when they are the ones being set for the first time.
fn set_at_path(document: &mut Value, path: &str, value: Value) -> Result<(), PatchError> {
    let mut segments = path.split('.').peekable();
    let mut cursor = document;
    let mut walked = String::new();

    loop {
        let segment = segments.next().ok_or(PatchError::EmptyPath)?;
        if segment.is_empty() {
            return Err(PatchError::EmptyPath);
        }
        let last = segments.peek().is_none();

        cursor = match cursor {
            Value::Object(map) => {
                if last {
                    map.insert(segment.to_string(), value);
                    return Ok(());
                }
                map.get_mut(segment).ok_or_else(|| PatchError::NoSuchPath {
                    path: path.to_string(),
                    segment: segment.to_string(),
                })?
            }
            Value::Array(items) => {
                let length = items.len();
                let index = segment
                    .parse::<usize>()
                    .map_err(|_| PatchError::NoSuchPath {
                        path: path.to_string(),
                        segment: segment.to_string(),
                    })?;
                // Arrays are replaced whole rather than appended to, so an index past the end is a
                // stale editor addressing an entry that has since been removed.
                let item = items.get_mut(index).ok_or(PatchError::IndexOutOfRange {
                    path: path.to_string(),
                    index,
                    length,
                })?;
                if last {
                    *item = value;
                    return Ok(());
                }
                item
            }
            _ => {
                return Err(PatchError::NotAContainer {
                    path: walked,
                    segment: segment.to_string(),
                });
            }
        };

        if !walked.is_empty() {
            walked.push('.');
        }
        walked.push_str(segment);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behaviour::{ContentGroup, Stage, TextItem};

    fn behaviour() -> Behaviour {
        let mut behaviour = Behaviour::new();
        behaviour.content.captions = vec![
            TextItem {
                text: "first".to_string(),
                tags: vec![],
                timeout_seconds: None,
            },
            TextItem {
                text: "second".to_string(),
                tags: vec!["a".to_string()],
                timeout_seconds: None,
            },
        ];
        behaviour.content.content_groups = vec![ContentGroup {
            id: "g".to_string(),
            label: "Group".to_string(),
            description: None,
            tags: vec![],
            enabled_by_default: true,
        }];
        behaviour
    }

    fn patch(path: &str, value: Value) -> Result<Behaviour, PatchError> {
        behaviour().patched(&[Patch::new(path, value)])
    }

    #[test]
    fn sets_a_nested_field() {
        let patched = patch("content.captions.1.text", "edited".into()).unwrap();
        assert_eq!(patched.content.captions[1].text, "edited");
        // Untouched siblings survive, which is the whole point of patching over overwriting.
        assert_eq!(patched.content.captions[0].text, "first");
    }

    #[test]
    fn replaces_a_whole_array() {
        let patched = patch(
            "content.captions",
            serde_json::json!([{ "text": "only", "tags": [] }]),
        )
        .unwrap();
        assert_eq!(patched.content.captions.len(), 1);
        assert_eq!(patched.content.captions[0].text, "only");
    }

    #[test]
    fn writes_a_field_that_serializes_away_when_empty() {
        // `description` and the media slots are `skip_serializing_if = "Option::is_none"`, so the
        // document has no such key at the moment one is first filled in.
        let patched = patch("content.content_groups.0.description", "why".into()).unwrap();
        assert_eq!(
            patched.content.content_groups[0].description.as_deref(),
            Some("why")
        );
        let patched = patch("content.wallpaper", 12.into()).unwrap();
        assert_eq!(patched.content.wallpaper, Some(12));
    }

    #[test]
    fn writes_a_whole_stage_prompt_when_its_default_serializes_away() {
        let mut behaviour = behaviour();
        behaviour.experience = Some(Default::default());
        behaviour
            .experience
            .as_mut()
            .unwrap()
            .timeline
            .stages
            .push(Stage {
                id: "first".to_string(),
                label: "First".to_string(),
                end: None,
                content: Default::default(),
                events: Default::default(),
                movement: None,
                mitosis: None,
                on_enter: Default::default(),
                prompt: Default::default(),
            });

        let patched = behaviour
            .patched(&[Patch::new(
                "experience.timeline.stages.0.prompt",
                serde_json::json!({ "timeouts_enabled": false }),
            )])
            .unwrap();

        assert!(
            !patched.experience.unwrap().timeline.stages[0]
                .prompt
                .timeouts_enabled
        );
    }

    #[test]
    fn clears_an_optional_field_with_null() {
        let patched = patch("content.prompt_settings.submit_label", Value::Null).unwrap();
        assert_eq!(patched.content.prompt_settings.submit_label, None);
    }

    #[test]
    fn applies_patches_in_order() {
        let patched = behaviour()
            .patched(&[
                Patch::new("content.captions.0.text", "one".into()),
                Patch::new("content.captions.0.text", "two".into()),
            ])
            .unwrap();
        assert_eq!(patched.content.captions[0].text, "two");
    }

    /// The per-item maps are keyed by media id, so their path segments *look* like array indices.
    /// They are not: `set_at_path` only parses a segment as an index when the cursor is an array,
    /// so an object keyed by "42" is addressed by key, and the entry an edit lands in is the one
    /// the author pointed at rather than the one at that position.
    #[test]
    fn addresses_a_per_item_entry_by_media_id_not_by_position() {
        use crate::behaviour::PopupMedia;

        let mut behaviour = behaviour();
        behaviour.content.popups.insert(
            7,
            PopupMedia {
                scale: Some(1.0),
                ..PopupMedia::default()
            },
        );
        behaviour.content.popups.insert(
            42,
            PopupMedia {
                scale: Some(2.0),
                ..PopupMedia::default()
            },
        );

        let patched = behaviour
            .patched(&[Patch::new("content.popups.42.scale", 3.0.into())])
            .unwrap();
        assert_eq!(patched.content.popups[&42].scale, Some(3.0));
        assert_eq!(
            patched.content.popups[&7].scale,
            Some(1.0),
            "the entry that happens to be first must be untouched",
        );
    }

    /// Unlike the pools, adding and removing an entry needs no whole-collection replacement: the
    /// key is stable, so a new entry is written by naming it. (A missing *final* segment on an
    /// object is written rather than rejected -- see `set_at_path`.) Individual fields of an entry
    /// that does not exist yet are still out of reach, so the editor writes the whole entry.
    #[test]
    fn adds_and_clears_a_per_item_entry_by_key() {
        let behaviour = behaviour();
        assert!(behaviour.content.popups.is_empty());

        let added = behaviour
            .patched(&[Patch::new(
                "content.popups.42",
                serde_json::json!({ "scale": 2.0 }),
            )])
            .unwrap();
        assert_eq!(added.content.popups[&42].scale, Some(2.0));
        assert_eq!(added.content.popups[&42].weight, None);

        // Clearing every field leaves an entry that says nothing; `storage::write` drops it, so
        // the document does not need a removal patch of its own.
        let cleared = added
            .patched(&[Patch::new("content.popups.42.scale", Value::Null)])
            .unwrap();
        assert!(cleared.content.popups[&42].is_empty());

        // A field of an entry that was never created has nothing to write into.
        assert_eq!(
            behaviour.patched(&[Patch::new("content.popups.42.scale", 2.0.into())]),
            Err(PatchError::NoSuchPath {
                path: "content.popups.42.scale".to_string(),
                segment: "42".to_string(),
            })
        );
    }

    #[test]
    fn rejects_a_path_into_a_missing_section() {
        // The timeline is suspended, so `experience` is absent -- a stale editor's write into it
        // must not bring it back.
        assert_eq!(
            patch("experience.timeline.stages", serde_json::json!([])),
            Err(PatchError::NoSuchPath {
                path: "experience.timeline.stages".to_string(),
                segment: "experience".to_string(),
            })
        );
    }

    #[test]
    fn rejects_an_index_past_the_end() {
        assert_eq!(
            patch("content.captions.5.text", "gone".into()),
            Err(PatchError::IndexOutOfRange {
                path: "content.captions.5.text".to_string(),
                index: 5,
                length: 2,
            })
        );
    }

    #[test]
    fn rejects_descending_through_a_leaf() {
        assert_eq!(
            patch("content.captions.0.text.length", 3.into()),
            Err(PatchError::NotAContainer {
                path: "content.captions.0.text".to_string(),
                segment: "length".to_string(),
            })
        );
    }

    #[test]
    fn rejects_an_empty_path() {
        assert_eq!(patch("", Value::Null), Err(PatchError::EmptyPath));
        assert_eq!(
            patch("content..captions", Value::Null),
            Err(PatchError::EmptyPath)
        );
    }

    #[test]
    fn rejects_a_value_of_the_wrong_shape() {
        // A document that no longer parses is never written; the caller keeps the one it had.
        let error = patch("content.captions.0.text", 7.into()).unwrap_err();
        assert!(matches!(error, PatchError::Invalid(_)), "{error:?}");
    }

    #[test]
    fn a_patch_that_empties_the_experience_section_is_allowed() {
        let mut behaviour = behaviour();
        behaviour.experience = Some(Default::default());
        let patched = behaviour
            .patched(&[Patch::new("experience", Value::Null)])
            .unwrap();
        assert!(patched.experience.is_none());
    }
}
