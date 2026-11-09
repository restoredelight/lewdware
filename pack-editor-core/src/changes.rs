//! What an edit changed, read from the SQLite changeset it produced rather than declared by the
//! code that made it. See `design/editor-model.md`, section 2.

use std::collections::BTreeSet;
use std::ops::{BitOr, BitOrAssign};

use anyhow::Result;
use rusqlite::fallible_streaming_iterator::FallibleStreamingIterator;
use rusqlite::hooks::Action;
use rusqlite::session::Changeset;
use rusqlite::types::ValueRef;

/// One area of the pack a view can depend on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Scope {
    /// Name, creator, description, version, recommended mode.
    Metadata,
    /// Modes uploaded into the pack.
    Modes,
    /// Opaque per-pack blobs (`pack_data`).
    PackData,
    /// The pack-wide media slots: wallpaper and splash.
    Slots,
    /// Captions, prompts, notifications and web links.
    Entries,
    ContentGroups,
    /// The timeline, its stages (including their own media slots) and transitions.
    Timeline,
    /// Per-file popup and audio settings.
    Attributes,
    /// Tag names and which files carry them.
    Tags,
    /// Artist names and which files carry them.
    Artists,
    /// Files themselves. The row-level detail is in [`MediaDelta`].
    Media,
}

/// A set of [`Scope`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Scopes(u16);

impl Scopes {
    pub const NONE: Scopes = Scopes(0);

    pub fn contains(self, scope: Scope) -> bool {
        self.0 & (1 << scope as u16) != 0
    }

    /// Whether any scope is in both sets: "does this change concern me?"
    pub fn intersects(self, other: Scopes) -> bool {
        self.0 & other.0 != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Every scope: for a view that needs refreshing whatever happened (a pack just opened).
    pub fn all() -> Scopes {
        use Scope::*;
        Metadata
            | Modes
            | PackData
            | Slots
            | Entries
            | ContentGroups
            | Timeline
            | Attributes
            | Tags
            | Artists
            | Media
    }
}

impl From<Scope> for Scopes {
    fn from(scope: Scope) -> Self {
        Scopes(1 << scope as u16)
    }
}

impl<T: Into<Scopes>> BitOr<T> for Scopes {
    type Output = Scopes;
    fn bitor(self, other: T) -> Scopes {
        Scopes(self.0 | other.into().0)
    }
}

impl<T: Into<Scopes>> BitOr<T> for Scope {
    type Output = Scopes;
    fn bitor(self, other: T) -> Scopes {
        Scopes::from(self) | other
    }
}

impl<T: Into<Scopes>> BitOrAssign<T> for Scopes {
    fn bitor_assign(&mut self, other: T) {
        self.0 |= other.into().0;
    }
}

/// What one applied action changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub scopes: Scopes,
    pub media: MediaDelta,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty() && self.media.is_empty()
    }
}

/// Which files appeared, disappeared or changed (title, source, tags, artists). The files
/// themselves are in the session's `MediaIndex`, already updated.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaDelta {
    pub added: Vec<u64>,
    pub removed: Vec<u64>,
    pub updated: Vec<u64>,
}

impl MediaDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.updated.is_empty()
    }
}

/// The scopes a write to `table` touches, or `None` for a table no view depends on.
///
/// Every table history tracks must have an answer here; a test holds that, so adding a table
/// without mapping it fails rather than leaving views stale.
pub fn table_scopes(table: &str) -> Option<Scopes> {
    use Scope::*;
    Some(match table {
        "pack_metadata" => Metadata.into(),
        "modes" => Modes.into(),
        "pack_data" => PackData.into(),
        "media" => Media.into(),
        // A file's tags are part of the file (the grid shows them) and of the tag (its count).
        "tags" => Tags.into(),
        "media_tags" => Tags | Media,
        "artists" => Artists.into(),
        "media_artists" => Artists | Media,
        "behaviour_content" => Slots.into(),
        // Stages hold their own wallpaper, audio and entry media.
        "behaviour_stage" => Timeline | Slots,
        "behaviour_timeline"
        | "behaviour_stage_tag"
        | "behaviour_stage_end"
        | "behaviour_stage_movement"
        | "behaviour_stage_mitosis"
        | "behaviour_stage_event"
        | "behaviour_transition"
        | "behaviour_transition_category" => Timeline.into(),
        "behaviour_content_entry" | "behaviour_content_entry_tag" => Entries.into(),
        "behaviour_content_group" | "behaviour_content_group_tag" => ContentGroups.into(),
        "behaviour_popup_media_fields" | "behaviour_popup_audio_pair" | "behaviour_audio_media" => {
            Attributes.into()
        }
        _ => return None,
    })
}

/// A changeset, read down to what the session needs to resolve it.
#[derive(Debug, Default)]
pub(crate) struct Observed {
    pub scopes: Scopes,
    /// Files whose own row, or tag or artist links, changed.
    pub media: BTreeSet<u64>,
    /// Tags and artists whose *name* changed. Every file carrying one shows the new name, though
    /// no row of theirs changed.
    pub renamed_tags: Vec<i64>,
    pub renamed_artists: Vec<i64>,
}

impl Observed {
    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }
}

pub(crate) fn observe(changeset: &Changeset) -> Result<Observed> {
    let mut observed = Observed::default();
    let mut items = changeset.iter()?;
    while let Some(item) = items.next()? {
        let op = item.op()?;
        let table = op.table_name();
        if let Some(scopes) = table_scopes(table) {
            observed.scopes |= scopes;
        }
        // Column 0 is the primary key, or its leading column, in each of these tables. A changeset
        // carries it as the new value for an insert and the old value otherwise.
        let key = || -> Result<Option<i64>> {
            let value = match op.code() {
                Action::SQLITE_INSERT => item.new_value(0)?,
                _ => item.old_value(0)?,
            };
            Ok(match value {
                ValueRef::Integer(id) => Some(id),
                _ => None,
            })
        };
        match table {
            "media" | "media_tags" | "media_artists" => {
                if let Some(id) = key()? {
                    observed.media.insert(id as u64);
                }
            }
            "tags" | "artists" if op.code() == Action::SQLITE_UPDATE => {
                if let Some(id) = key()? {
                    if table == "tags" {
                        observed.renamed_tags.push(id);
                    } else {
                        observed.renamed_artists.push(id);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(observed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tracked_table_has_a_scope() {
        for table in crate::history::tracked_tables() {
            assert!(
                table_scopes(table).is_some(),
                "`{table}` is recorded by history but maps to no scope: add it to `table_scopes`"
            );
        }
    }

    #[test]
    fn scope_sets() {
        let set = Scope::Tags | Scope::Media;
        assert!(set.contains(Scope::Tags) && set.contains(Scope::Media));
        assert!(!set.contains(Scope::Timeline));
        assert!(set.intersects(Scope::Media.into()));
        assert!(!set.intersects(Scope::Timeline | Scope::Slots));
        assert!(Scopes::NONE.is_empty());
    }
}
