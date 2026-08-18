use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

/// Data read by both default modes: captions, prompts, notifications, subliminals, web links,
/// the wallpaper/splash media references, and the content groups a user can toggle. See
/// `behaviour-design/behaviour-tab.md` and `behaviour-design/default-mode.md` (Ownership).
///
/// Wallpaper and splash are single media *references*, not tag queries: one file each, held by
/// id and resolved to a name by the engine before the modes see it. They were tag lists once,
/// which asked an author to know that `wallpaper` was a mechanical tag and to keep it off
/// ordinary media -- while every format they can come from is single-image anyway (Edgeware has one
/// `wallpaper.png`, one `loading_splash.*`, and one wallpaper per corruption level). Both stay
/// opt-in: `None` means the pack doesn't use engine-managed wallpaper/splash at all.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Content {
    /// Per-file popup attributes, keyed by media id. See [`PopupMedia`].
    ///
    /// A map rather than a list because the key is the address: unlike the pools, an entry is
    /// edited, added and removed by media id, which is stable across everything except deleting
    /// the file. That also makes it patchable field by field (`content.popups.42.scale`) instead
    /// of by whole-array replacement.
    ///
    /// Serialized even when empty, like the pools and unlike the optional media slots. A patch
    /// writes a missing key only as its *final* segment — inventing an intermediate one would
    /// resurrect a section a stale editor is writing into (see `patch.rs`) — so a map that
    /// vanished when empty would reject the first entry ever added to a pack.
    #[serde(default)]
    pub popups: BTreeMap<u64, PopupMedia>,
    /// Per-file audio attributes, keyed by media id. See [`AudioMedia`] and `popups` above.
    #[serde(default)]
    pub audio: BTreeMap<u64, AudioMedia>,
    #[serde(default)]
    pub content_groups: Vec<ContentGroup>,
    #[serde(default)]
    pub captions: Vec<TextItem>,
    #[serde(default)]
    pub prompts: Vec<TextItem>,
    #[serde(default)]
    pub notifications: Vec<TextItem>,
    #[serde(default)]
    pub subliminals: Vec<TextItem>,
    #[serde(default)]
    pub web_links: Vec<WebLink>,
    /// Id of the media file used as the desktop wallpaper. `None` means the pack has no
    /// wallpaper feature at all -- see this struct's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<u64>,
    /// Id of the media file shown as the startup splash. See `wallpaper`'s doc comment --
    /// same reasoning, except that a splash may also be a video (an animated GIF probes as one;
    /// see `shared/src/encode.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splash: Option<u64>,
}

impl Content {
    /// Whether this document describes no content at all: no pools, no groups, and neither media
    /// slot filled.
    pub fn is_empty(&self) -> bool {
        self.popups.is_empty()
            && self.audio.is_empty()
            && self.content_groups.is_empty()
            && self.captions.is_empty()
            && self.prompts.is_empty()
            && self.notifications.is_empty()
            && self.subliminals.is_empty()
            && self.web_links.is_empty()
            && self.wallpaper.is_none()
            && self.splash.is_none()
    }
}

/// One place in a behaviour document that points at a media file -- the address of a media slot,
/// independent of what (if anything) currently fills it. Used by the Edgeware importer, which
/// knows which file belongs in which slot long before that file has an id to record, and so fills
/// each slot with [`Behaviour::fill_media_reference`] once its media really exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaSlot {
    /// `Content::wallpaper`.
    Wallpaper,
    /// `Content::splash`.
    Splash,
    /// The wallpaper a timeline stage sets, addressed by stage id rather than by position: a
    /// slot may be filled long after it was read, and ids are what survive editing in between.
    StageWallpaper {
        stage: String,
    },
    /// The background track a timeline stage selects. `None` retains the current track.
    StageAudio {
        stage: String,
    },
    StageEntrySplash {
        stage: String,
    },
    StageEntrySound {
        stage: String,
    },
    StagePromptSound {
        stage: String,
    },
}

/// What a pack author says about one file used as popup content.
///
/// Every field is optional, and an entry with nothing set is not stored at all (see
/// [`PopupMedia::is_empty`]) — "unset" has to stay distinguishable from "set to today's default",
/// because defaults move under the user across engine releases. A mode reading this treats an
/// absent field as "no opinion", never as a zero.
///
/// These describe the *content*, never the user's relationship to the window: there is
/// deliberately no per-file opacity, click-through, decorations or auto-close, since those are
/// user-owned in both modes and a per-file override is exactly the silent surprise the ownership
/// model exists to prevent. See `behaviour-design/default-mode-v2.md`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PopupMedia {
    /// How often this file is drawn relative to its neighbours. Affects *which* file spawns,
    /// never how many windows exist, so it cannot escape `max_popups`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    /// Multiplies the size the mode would otherwise have chosen. The engine's monitor-fraction
    /// cap still binds, so this cannot fill the screen from a pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    /// The part of the monitor this file may spawn in. `None` is the whole of it — the mode's
    /// ordinary random placement — so an author who has said nothing is not pinned to a rectangle
    /// that was the default when they wrote the pack.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<SpawnRegion>,
    /// Which monitor this file prefers. `None` means the mode's choice, which is a random one of
    /// the monitors the user allows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<MonitorPreference>,
    /// A caption belonging to this file, as opposed to the tag-matched pool in `captions`. Still
    /// under the user's `captions_enabled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// `VideoPopupOpts::loop` for this clip — `Some(false)` closes the popup when the clip ends,
    /// which a short clip usually wants and the mode-wide default cannot express.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_loop: Option<bool>,
    /// `VideoPopupOpts::audio` for this clip. Still under the user's popup-sound switch: this can
    /// silence a clip the user allowed, never unsilence one they didn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_audio: Option<bool>,
    /// Sounds paired explicitly with this popup, by media id — the cases tag matching cannot
    /// express. A set: the mode picks one at random, so the order carries no meaning and is
    /// normalised on read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<u64>,
}

impl PopupMedia {
    /// Whether this entry says nothing, and so should not be stored.
    ///
    /// The editor clears an attribute by setting it to null; when the last one goes, the entry
    /// stops existing rather than lingering as a row of NULLs. `read` never produces one, so the
    /// distinction is invisible above this layer.
    pub fn is_empty(&self) -> bool {
        self.weight.is_none()
            && self.scale.is_none()
            && self.region.is_none()
            && self.monitor.is_none()
            && self.caption.is_none()
            && self.video_loop.is_none()
            && self.video_audio.is_none()
            && self.audio.is_empty()
    }
}

/// What a pack author says about one audio file. See [`PopupMedia`] for the conventions; the same
/// "absent means no opinion" rule applies.
///
/// Deliberately no `loop`. A track that should repeat is expressible already -- a pack whose
/// background pool is one file plays that file on a loop, because the rotation re-picks it -- and
/// as an option it did more harm than good: on a popup sound it means nothing (a sting that never
/// ends), and on a background track it *stops the rotation*, so one file marked to loop silently
/// keeps every other track in the pack from ever playing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AudioMedia {
    /// This track's own level, for levelling a pack assembled from mixed sources. Composes with
    /// the user's volume rather than replacing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
}

impl AudioMedia {
    /// See [`PopupMedia::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.volume.is_none()
    }
}

/// The part of a monitor a popup may spawn in, as fractions of its usable area — the
/// `SpawnRegion` class in `api.lua`, which this reaches the engine as.
///
/// This *replaced* a nine-value anchor, and subsumes it: the engine places the window entirely
/// inside the region, centring it on the region and clamping to the screen when it does not fit,
/// so a region of zero size names one placement exactly (`{1, 1, 0, 0}` is the bottom-right
/// corner, `{0.5, 0.5, 0, 0}` is centred). One field instead of two, and "somewhere in the left
/// half" stops being inexpressible.
///
/// Sanitised on the way in rather than trusted: a pack is data, and NaN reaching
/// `rand::random_range` is a panic in the engine.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct SpawnRegion {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl SpawnRegion {
    /// The whole monitor: what `PopupMedia::region` being absent means, spelled out.
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// This region with every edge inside the monitor and no negative or non-finite extent.
    ///
    /// The position is preserved where it can be: an over-large rectangle shrinks against the
    /// far edge rather than sliding, which is what an author dragging one out expects.
    ///
    /// Rounded to a thousandth of the screen — a pixel on a 1920-wide monitor, which is below
    /// anything an author can express or see. That is not cosmetic: clamping produces values like
    /// `1.0 - 0.8`, and the editor decides whether to *store* a region at all by comparing it
    /// against the full screen. Two implementations of this rule disagreeing in the last bit
    /// would make that comparison answer differently on either side.
    pub fn sanitized(self) -> Self {
        fn finite(value: f64) -> f64 {
            if value.is_finite() { value } else { 0.0 }
        }

        fn round(value: f64) -> f64 {
            (value * 1000.0).round() / 1000.0
        }

        let x = finite(self.x).clamp(0.0, 1.0);
        let y = finite(self.y).clamp(0.0, 1.0);

        Self {
            x: round(x),
            y: round(y),
            width: round(finite(self.width).clamp(0.0, 1.0 - x)),
            height: round(finite(self.height).clamp(0.0, 1.0 - y)),
        }
    }

    /// Whether this is the whole monitor, and so says nothing the default does not already say.
    /// The editor stores `None` instead of a full region, for the reason on [`PopupMedia`].
    pub fn is_full(&self) -> bool {
        self.sanitized() == Self::FULL
    }
}

/// Which monitor a popup prefers, when the user has more than one.
///
/// A *preference*, not a guarantee: the user may have switched their primary monitor off in the
/// Monitors tab, and a pack cannot overrule that. The mode falls back to its ordinary random
/// choice rather than refusing to spawn.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MonitorPreference {
    /// Any monitor the user allows, chosen at random. The same as saying nothing; the editor
    /// writes `None` rather than this, and it exists so a mode reading the field can match
    /// exhaustively.
    Any,
    /// The user's primary monitor.
    Primary,
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
    /// Only prompts use this. Absent means the default mode derives a deadline from the prompt's
    /// character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<f64>,
    /// Only notifications use this: the desktop notification's title, with `text` as its body.
    /// Absent (or empty) means the notification is shown with no title, which is what every
    /// converted Edgeware pack gets — Edgeware notifications are bodies only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
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

/// The behaviour document, as every layer above `behaviour::storage` sees it.
///
/// It carries no version of its own. It used to, back when it was a JSON blob that had to
/// describe its own shape; now it is rows, and the pack database's migration ledger says which
/// shape they are in. `shared::db::migrate` already refuses a pack whose schema is newer than the
/// binary understands, which is the check the document's own version was a second, weaker copy of.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Behaviour {
    #[serde(default)]
    pub content: Content,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<Experience>,
}

impl Behaviour {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate(&self) -> Vec<ValidationIssue> {
        let Some(experience) = &self.experience else {
            return vec![];
        };
        experience.timeline.validate()
    }

    /// Empties every slot pointing at `media` -- what deleting that file means for this document.
    ///
    /// Deletion is the only thing that can invalidate a reference now that slots hold ids: a
    /// rename leaves the id alone, which is the whole point of step 1 in
    /// `design/behaviour-storage.md`. Returns whether anything changed, so a caller can skip
    /// re-persisting an untouched document.
    pub fn clear_media_reference(&mut self, media: u64) -> bool {
        let mut changed = false;
        for (_, value) in self.media_reference_slots() {
            if *value == Some(media) {
                *value = None;
                changed = true;
            }
        }
        changed
    }

    /// Points `slot` at `media`, if that slot exists and is still empty.
    ///
    /// Only-if-empty because a slot is filled part-way through a long-running import, and the
    /// Content tab is live throughout: a slot the author set themselves in the meantime is their
    /// answer, not one to overwrite. Returns whether it was filled.
    pub fn fill_media_reference(&mut self, slot: &MediaSlot, media: u64) -> bool {
        for (candidate, value) in self.media_reference_slots() {
            if &candidate == slot && value.is_none() {
                *value = Some(media);
                return true;
            }
        }
        false
    }

    /// Every slot that holds a media id, with a mutable handle on it. The single place that knows
    /// where references live, so adding a slot later can't leave one of the operations above
    /// quietly not covering it.
    fn media_reference_slots(&mut self) -> impl Iterator<Item = (MediaSlot, &mut Option<u64>)> {
        let stages = self
            .experience
            .iter_mut()
            .flat_map(|experience| experience.timeline.stages.iter_mut())
            .flat_map(|stage| {
                let entry = &mut stage.on_enter;
                let StageEntry { splash, sound, .. } = entry;
                [
                    (
                        MediaSlot::StageWallpaper {
                            stage: stage.id.clone(),
                        },
                        &mut stage.content.wallpaper,
                    ),
                    (
                        MediaSlot::StageAudio {
                            stage: stage.id.clone(),
                        },
                        &mut stage.content.audio,
                    ),
                    (
                        MediaSlot::StageEntrySplash {
                            stage: stage.id.clone(),
                        },
                        splash,
                    ),
                    (
                        MediaSlot::StageEntrySound {
                            stage: stage.id.clone(),
                        },
                        sound,
                    ),
                    (
                        MediaSlot::StagePromptSound {
                            stage: stage.id.clone(),
                        },
                        &mut stage.prompt.sound,
                    ),
                ]
            });
        [
            (MediaSlot::Wallpaper, &mut self.content.wallpaper),
            (MediaSlot::Splash, &mut self.content.splash),
        ]
        .into_iter()
        .chain(stages)
    }

    /// Every media id this document references, in no particular order. Used to decide whether a
    /// file a slot is being cleared from is still needed elsewhere (see the pack editor's slot
    /// lifecycle) -- a base wallpaper reused by a timeline stage is Edgeware's ordinary case, so
    /// "clear this stage's wallpaper" must not take the pack's main one with it.
    pub fn referenced_media_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = [self.content.wallpaper, self.content.splash]
            .into_iter()
            .flatten()
            .collect();
        if let Some(experience) = &self.experience {
            ids.extend(
                experience
                    .timeline
                    .stages
                    .iter()
                    .filter_map(|stage| stage.content.wallpaper),
            );
            ids.extend(
                experience
                    .timeline
                    .stages
                    .iter()
                    .filter_map(|stage| stage.content.audio),
            );
            ids.extend(
                experience
                    .timeline
                    .stages
                    .iter()
                    .filter_map(|stage| stage.on_enter.splash),
            );
            ids.extend(
                experience
                    .timeline
                    .stages
                    .iter()
                    .filter_map(|stage| stage.on_enter.sound),
            );
            ids.extend(
                experience
                    .timeline
                    .stages
                    .iter()
                    .filter_map(|stage| stage.prompt.sound),
            );
        }
        ids
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
    #[serde(default, skip_serializing_if = "StageEntry::is_default")]
    pub on_enter: StageEntry,
    #[serde(default, skip_serializing_if = "StagePrompt::is_default")]
    pub prompt: StagePrompt,
}

/// Declarative punctuation fired once after a stage transition has completed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StageEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splash: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup_burst: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<String>,
}

impl StageEntry {
    pub(crate) fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StagePrompt {
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub timeouts_enabled: bool,
    #[serde(default = "default_one", skip_serializing_if = "is_one")]
    pub timeout_multiplier: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup_burst: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<u64>,
}

impl Default for StagePrompt {
    fn default() -> Self {
        Self {
            timeouts_enabled: true,
            timeout_multiplier: 1.0,
            popup_burst: None,
            sound: None,
        }
    }
}

impl StagePrompt {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

fn is_true(value: &bool) -> bool {
    *value
}

fn default_one() -> f64 {
    1.0
}

fn is_one(value: &f64) -> bool {
    *value == 1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ContentSelection {
    /// `None` uses all content; `Some([])` deliberately selects none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// The one of [`Self::tags`] the editor created for this stage, and therefore maintains the
    /// name of: renaming the stage renames it, and deleting the stage retires it when nothing else
    /// claims it. A tag the author added by hand is never owned, and never touched.
    ///
    /// Necessarily one of `tags` — ownership is recorded on the association row, so a stage that
    /// stops selecting by a tag stops owning it. `None` for every stage whose tags the author chose
    /// themselves, and for every unrestricted stage, which has no selection to own a tag in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_tag: Option<String>,
    /// Id of the media file this stage sets as the wallpaper. `None` retains whatever the
    /// previous stage (or `Content::wallpaper`) left in effect -- an absolute write, not a
    /// delta, which is why one id is enough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallpaper: Option<u64>,
    /// Background track selected on entry. `None` retains whatever is already playing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<u64>,
    /// Select a fresh background track from the stage's active tags on entry. Mutually exclusive
    /// with `audio`; absence of both retains what is currently playing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub audio_random: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<EventSchedule>,
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
    Sound,
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
    SoundInterval,
    Crossfade,
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

    fn behaviour_with_slots(wallpaper: u64, splash: u64, stage_wallpaper: u64) -> Behaviour {
        let mut behaviour = Behaviour::new();
        behaviour.content.wallpaper = Some(wallpaper);
        behaviour.content.splash = Some(splash);
        behaviour.experience = Some(Experience {
            timeline: Timeline {
                stages: vec![Stage {
                    id: "stage-1".to_string(),
                    label: "Stage 1".to_string(),
                    end: None,
                    content: ContentSelection {
                        tags: None,
                        owned_tag: None,
                        wallpaper: Some(stage_wallpaper),
                        audio: None,
                        audio_random: false,
                    },
                    events: Events::default(),
                    movement: None,
                    mitosis: None,
                    on_enter: Default::default(),
                    prompt: Default::default(),
                }],
                transitions: vec![],
            },
            label: None,
        });
        behaviour
    }

    #[test]
    fn filling_a_media_reference_respects_a_slot_that_is_already_set() {
        // The import fills slots at the end of a long-running job, and the Content tab is live
        // throughout: a slot the author set in the meantime is their answer, not one to replace.
        // Mid-import: the wallpaper and the stage's are still empty (their files haven't landed),
        // while the author has already picked a splash by hand.
        let mut behaviour = Behaviour::new();
        behaviour.experience = behaviour_with_slots(1, 2, 3).experience;
        behaviour.experience.as_mut().unwrap().timeline.stages[0]
            .content
            .wallpaper = None;
        behaviour.content.splash = Some(99);

        assert!(behaviour.fill_media_reference(&MediaSlot::Wallpaper, 1));
        assert!(!behaviour.fill_media_reference(&MediaSlot::Splash, 2));
        assert!(behaviour.fill_media_reference(
            &MediaSlot::StageWallpaper {
                stage: "stage-1".to_string()
            },
            3
        ));
        assert!(behaviour.fill_media_reference(
            &MediaSlot::StageAudio {
                stage: "stage-1".to_string()
            },
            4
        ));
        // A stage deleted while the import ran simply has nowhere to fill.
        assert!(!behaviour.fill_media_reference(
            &MediaSlot::StageWallpaper {
                stage: "gone".to_string()
            },
            3
        ));

        assert_eq!(behaviour.content.wallpaper, Some(1));
        assert_eq!(behaviour.content.splash, Some(99));
        let stage = &behaviour.experience.as_ref().unwrap().timeline.stages[0];
        assert_eq!(stage.content.wallpaper, Some(3));
        assert_eq!(stage.content.audio, Some(4));
    }

    #[test]
    fn clearing_a_media_reference_covers_every_slot_pointing_at_it() {
        // Deleting a file the base wallpaper and a stage both use has to clear both -- the
        // referential integrity the pack editor's lifecycle needs.
        let mut behaviour = behaviour_with_slots(1, 2, 1);

        assert!(behaviour.clear_media_reference(1));
        assert_eq!(behaviour.referenced_media_ids(), vec![2]);
        // Nothing points at it any more, so there is nothing to re-persist.
        assert!(!behaviour.clear_media_reference(1));
    }

    /// The point of holding ids: a file can be renamed freely without the document hearing about
    /// it, which is what removes the rename round-trip between the editor and the backend.
    #[test]
    fn a_slot_survives_its_file_being_renamed() {
        let behaviour = behaviour_with_slots(1, 2, 3);
        let before = behaviour.referenced_media_ids();
        // A rename touches `media.file_name` and nothing else -- there is no behaviour operation
        // for it at all, which is why this test can only assert the absence of one.
        assert_eq!(before, vec![1, 2, 3]);
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
                    on_enter: Default::default(),
                    prompt: Default::default(),
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
