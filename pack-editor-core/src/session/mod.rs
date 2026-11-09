//! One open pack, edited synchronously. See `design/editor-model.md`.
//!
//! Every write goes through [`Session::write`], which watches the tables history tracks while the
//! write runs. The changeset that comes out is exactly what changed, whichever path made the change
//! (a behaviour edit, a tag edit, undo, redo), so callers never say what to refresh.
//!
//! Writes here are meant for the UI thread: small ones take well under a millisecond on a
//! 10,000-file pack. Actions that touch every file take ~100ms and belong on a background thread
//! (`pack/tests/timing.rs`).

mod content;
pub mod library;
pub mod lifecycle;
mod timeline;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::Result;
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::session::Session as Observer;
use rusqlite::{params, Connection};

use crate::changes::{self, Changes, MediaDelta, Scope};
use crate::events::ImportEvent;
use crate::history;
use crate::pack::{self, ops, BehaviourAction, BehaviourOutcome, MediaFile, MediaPack};

/// The result of one applied action.
#[derive(Debug)]
pub struct Applied<T> {
    pub value: T,
    pub changes: Changes,
    /// Undo/redo availability and whether the pack is back at its saved state, as of this action.
    pub history: history::Status,
}

pub struct Session {
    pack: Arc<MediaPack>,
    connection: PooledConnection<SqliteConnectionManager>,
    media: MediaIndex,
    history: history::Status,
}

/// How a write affects whether the pack is saved.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteKind {
    /// An edit: unsaved if it changed anything.
    Edit,
    /// An undo or redo: saved exactly when history is back at the saved state.
    HistoryMove,
}

impl Session {
    /// Loads the media index (~65ms for 10,000 files), so call it off the UI thread.
    pub fn open(pack: Arc<MediaPack>) -> Result<Self> {
        let connection = pack.connection()?;
        let media = MediaIndex::new(pack::read_files(&connection, None)?);
        let history = history::status(&connection)?;
        Ok(Self {
            pack,
            connection,
            media,
            history,
        })
    }

    pub fn pack(&self) -> &Arc<MediaPack> {
        &self.pack
    }

    pub fn media(&self) -> &MediaIndex {
        &self.media
    }

    pub fn history(&self) -> &history::Status {
        &self.history
    }

    /// Runs a read. For the small queries views make in `render` or on a change in their scope:
    /// `shared::behaviour::editor::timeline`, `captions` and so on.
    pub fn read<T>(&self, query: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        query(&self.connection)
    }

    // ── Behaviour ───────────────────────────────────────────────────────────────────────────

    /// One author action on the behaviour document: one transaction, one undo entry.
    pub fn edit<P: Send + 'static>(
        &mut self,
        action: BehaviourAction<P>,
    ) -> Result<Applied<BehaviourOutcome>> {
        self.write(WriteKind::Edit, |connection| {
            Ok(pack::behaviour_edit_on(connection, action)?.0)
        })
    }

    // ── History ─────────────────────────────────────────────────────────────────────────────

    pub fn undo(&mut self) -> Result<Applied<()>> {
        self.write(WriteKind::HistoryMove, |connection| {
            history::undo(connection).map(drop)
        })
    }

    pub fn redo(&mut self) -> Result<Applied<()>> {
        self.write(WriteKind::HistoryMove, |connection| {
            history::redo(connection).map(drop)
        })
    }

    // ── Tags and artists ────────────────────────────────────────────────────────────────────

    pub fn add_tag_to_files(&mut self, ids: &[u64], tag: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::add_tag_to_files(c, ids, tag))
    }

    pub fn remove_tag_from_files(&mut self, ids: &[u64], tag: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::remove_tag_from_files(c, ids, tag))
    }

    pub fn rename_tag(&mut self, from: &str, to: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::rename_tag(c, from, to))
    }

    pub fn merge_tag(&mut self, from: &str, to: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::merge_tag(c, from, to))
    }

    pub fn delete_tag(&mut self, tag: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::delete_tag(c, tag))
    }

    pub fn add_artist_to_files(&mut self, ids: &[u64], artist: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            ops::add_artist_to_files(c, ids, artist)
        })
    }

    pub fn remove_artist_from_files(&mut self, ids: &[u64], artist: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            ops::remove_artist_from_files(c, ids, artist)
        })
    }

    pub fn rename_artist(&mut self, from: &str, to: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::rename_artist(c, from, to))
    }

    pub fn merge_artist(&mut self, from: &str, to: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::merge_artist(c, from, to))
    }

    pub fn delete_artist(&mut self, artist: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::delete_artist(c, artist))
    }

    // ── Files ───────────────────────────────────────────────────────────────────────────────

    pub fn set_source_url(&mut self, id: u64, url: Option<&str>) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| ops::set_source_url(c, id, url))
    }

    // ── Imports ─────────────────────────────────────────────────────────────────────────────

    /// Folds an import pipeline's progress into the session. Imports write on their own pooled
    /// connections, so their changes don't pass through [`Self::write`]; this is how they reach the
    /// index and the views instead. `None` for events that change nothing in the pack.
    pub fn import_event(&mut self, event: &ImportEvent) -> Result<Option<Applied<()>>> {
        let changes = match event {
            ImportEvent::Added(file) => {
                let fresh = !self.media.files.contains_key(&file.id);
                self.media.files.insert(file.id, file.clone());
                Changes {
                    // An import can bring the file's tags and artists with it.
                    scopes: Scope::Media | Scope::Tags | Scope::Artists,
                    media: MediaDelta {
                        added: if fresh { vec![file.id] } else { Vec::new() },
                        updated: if fresh { Vec::new() } else { vec![file.id] },
                        removed: Vec::new(),
                    },
                }
            }
            ImportEvent::SlotsFilled(_) => Changes {
                scopes: Scope::Slots | Scope::Timeline,
                media: MediaDelta::default(),
            },
            // The import's history entry is finalised: undo now covers it.
            ImportEvent::Done => Changes::default(),
            ImportEvent::Start { .. }
            | ImportEvent::Skipped(_)
            | ImportEvent::Error(_)
            | ImportEvent::FileDone => return Ok(None),
        };
        self.history = history::status(&self.connection)?;
        Ok(Some(Applied {
            value: (),
            changes,
            history: self.history.clone(),
        }))
    }

    /// [`Self::write`] for an arbitrary recorded operation, so tests can reach paths the public
    /// methods don't cover yet.
    #[cfg(test)]
    pub(crate) fn write_for_tests<T>(
        &mut self,
        op: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<Applied<T>> {
        self.write(WriteKind::Edit, op)
    }

    // ── The one write path ──────────────────────────────────────────────────────────────────

    fn write<T>(
        &mut self,
        kind: WriteKind,
        op: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<Applied<T>> {
        let _guard = self.pack.try_edit_guard()?;

        let (value, observed) = {
            let mut observer = Observer::new(&self.connection)?;
            for table in history::tracked_tables() {
                observer.attach(Some(table))?;
            }
            // On an error the operation's transaction rolled back and nothing changed, so there's
            // nothing to report: the observer is simply dropped.
            let value = op(&self.connection)?;
            let observed = changes::observe(&observer.changeset()?)?;
            (value, observed)
        };

        let history = history::status(&self.connection)?;
        match kind {
            WriteKind::HistoryMove => self.pack.note_history_moved(&self.connection, &history)?,
            WriteKind::Edit if !observed.is_empty() => self.pack.note_edited()?,
            WriteKind::Edit => {}
        }
        self.history = history.clone();

        let media = self.resolve_media(&observed)?;
        let mut scopes = observed.scopes;
        if !media.is_empty() {
            scopes |= Scope::Media;
        }
        Ok(Applied {
            value,
            changes: Changes { scopes, media },
            history,
        })
    }

    /// Re-reads the files a write touched and brings the index in line with them.
    fn resolve_media(&mut self, observed: &changes::Observed) -> Result<MediaDelta> {
        let mut touched: BTreeSet<u64> = observed.media.clone();
        for (ids, table) in [
            (&observed.renamed_tags, "media_tags"),
            (&observed.renamed_artists, "media_artists"),
        ] {
            if ids.is_empty() {
                continue;
            }
            let column = if table == "media_tags" {
                "tag_id"
            } else {
                "artist_id"
            };
            let selected =
                pack::media_id_array(&ids.iter().map(|&id| id as u64).collect::<Vec<_>>())?;
            let mut statement = self.connection.prepare(&format!(
                "SELECT media_id FROM {table} WHERE {column} IN (SELECT value FROM rarray(?))"
            ))?;
            let rows = statement.query_map(params![selected], |row| row.get::<_, u64>(0))?;
            for id in rows {
                touched.insert(id?);
            }
        }
        if touched.is_empty() {
            return Ok(MediaDelta::default());
        }

        let ids: Vec<u64> = touched.into_iter().collect();
        // Looking files up one id at a time costs more than one pass over the table once an edit
        // touches a good share of the pack (a bulk tag over 10,000 files: ~280ms against ~50ms).
        let fresh = if ids.len() > self.media.len() / 4 {
            let wanted: BTreeSet<u64> = ids.iter().copied().collect();
            pack::read_files(&self.connection, None)?
                .into_iter()
                .filter(|file| wanted.contains(&file.id))
                .collect()
        } else {
            pack::read_files(&self.connection, Some(&ids))?
        };
        Ok(self.media.apply(&ids, fresh))
    }
}

/// Every visible file, with its tags and artists, kept in step with the pack by the session.
/// Views filter and sort it in memory.
pub struct MediaIndex {
    files: BTreeMap<u64, MediaFile>,
}

impl MediaIndex {
    fn new(files: Vec<MediaFile>) -> Self {
        Self {
            files: files.into_iter().map(|file| (file.id, file)).collect(),
        }
    }

    pub fn get(&self, id: u64) -> Option<&MediaFile> {
        self.files.get(&id)
    }

    /// In id order, which is import order.
    pub fn iter(&self) -> impl Iterator<Item = &MediaFile> {
        self.files.values()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// `ids` were re-read and `fresh` is what's visible of them now: any id missing from it was
    /// deleted or never existed.
    fn apply(&mut self, ids: &[u64], fresh: Vec<MediaFile>) -> MediaDelta {
        let mut fresh: BTreeMap<u64, MediaFile> =
            fresh.into_iter().map(|file| (file.id, file)).collect();
        let mut delta = MediaDelta::default();
        for &id in ids {
            match (self.files.contains_key(&id), fresh.remove(&id)) {
                (false, Some(file)) => {
                    self.files.insert(id, file);
                    delta.added.push(id);
                }
                (true, Some(file)) => {
                    if self.files.get(&id) != Some(&file) {
                        self.files.insert(id, file);
                        delta.updated.push(id);
                    }
                }
                (true, None) => {
                    self.files.remove(&id);
                    delta.removed.push(id);
                }
                (false, None) => {}
            }
        }
        delta
    }
}
