//! Recorded edits for pack metadata and the modes a pack bundles, as functions over a connection.
//! See `super` for the contract.
//!
//! Metadata is also cached on the [`MediaPack`] (its name is shown everywhere), so a caller that
//! changes it calls [`sync_metadata`] after the write lands. Undo and redo already do
//! (`MediaPack::note_history_moved`).

use std::io::Cursor;

use anyhow::{bail, Context as _, Result};
use rusqlite::{params, Connection, OptionalExtension};
use shared::pack::{Metadata, RecommendedMode};
use uuid::Uuid;

use crate::history;
use crate::pack::{MediaPack, MetadataEdit};

/// One field of the pack's metadata, as the Metadata and Modes views edit it.
#[derive(Clone, Debug, PartialEq)]
pub enum MetadataField {
    /// Required: an empty name is refused.
    Name(String),
    /// `None`, or text that's only whitespace, clears the field.
    Creator(Option<String>),
    Description(Option<String>),
    Version(Option<String>),
    RecommendedMode(Option<RecommendedMode>),
}

impl MetadataField {
    fn label(&self) -> &'static str {
        match self {
            Self::Name(_) => "Rename pack",
            Self::Creator(_) => "Edit pack creator",
            Self::Description(_) => "Edit pack description",
            Self::Version(_) => "Edit pack version",
            Self::RecommendedMode(_) => "Set recommended mode",
        }
    }

    /// Trims text, turns blank optional text into `None`, and refuses a blank name.
    fn into_edit(self) -> Result<MetadataEdit> {
        fn optional(value: Option<String>) -> Option<String> {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        }
        Ok(match self {
            Self::Name(name) => {
                let name = name.trim();
                if name.is_empty() {
                    bail!("The pack needs a name.");
                }
                MetadataEdit::Name(name.to_string())
            }
            Self::Creator(value) => MetadataEdit::Creator(optional(value)),
            Self::Description(value) => MetadataEdit::Description(optional(value)),
            Self::Version(value) => MetadataEdit::Version(optional(value)),
            Self::RecommendedMode(value) => MetadataEdit::RecommendedMode(value),
        })
    }
}

/// Sets one metadata field. `Ok(false)` when it already had that value (no undo entry).
pub fn set_metadata(connection: &Connection, field: MetadataField) -> Result<bool> {
    let label = field.label();
    let edit = field.into_edit()?;
    history::record(connection, label, 0, &[], |tx| edit.write(tx))
}

/// Brings the pack's cached metadata (its name, for the toolbar, recents and the window title) in
/// line with the database after a write that may have changed it.
pub fn sync_metadata(pack: &MediaPack, connection: &Connection) -> Result<()> {
    *pack.metadata.write().unwrap() = Metadata::read(connection)?;
    Ok(())
}

/// A mode bundled with the pack, described from its `.lwmode` header and metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddedMode {
    /// The row in the pack (what `RecommendedMode::Pack` points at).
    pub id: u64,
    /// The mode's own identity, the same across rebuilds of it.
    pub stable_id: Uuid,
    pub name: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub option_count: usize,
    /// Bytes.
    pub size: u64,
}

/// Reads a `.lwmode` file's description. Fails for anything that isn't one.
pub fn describe_mode(id: u64, bytes: &[u8]) -> Result<EmbeddedMode> {
    let (header, metadata) = shared::mode::read_mode_metadata(Cursor::new(bytes))?;
    Ok(EmbeddedMode {
        id,
        stable_id: header.id,
        option_count: metadata.all_options().len(),
        name: metadata.name,
        author: metadata.author,
        version: metadata.version,
        size: bytes.len() as u64,
    })
}

/// Every mode in the pack, in the order they were added.
pub fn modes(connection: &Connection) -> Result<Vec<EmbeddedMode>> {
    let mut statement = connection.prepare("SELECT id, file FROM modes ORDER BY id")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, u64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let mut modes = Vec::new();
    for row in rows {
        let (id, bytes) = row?;
        modes.push(describe_mode(id, &bytes).with_context(|| format!("Mode {id} is unreadable"))?);
    }
    Ok(modes)
}

/// Adds a built `.lwmode` file to the pack. Refuses one that isn't a mode, or a mode the pack
/// already has (by its stable id, so a rebuild of the same mode counts).
pub fn add_mode(connection: &Connection, bytes: Vec<u8>) -> Result<EmbeddedMode> {
    let candidate = describe_mode(0, &bytes).context("That file isn't a Lewdware mode")?;
    if let Some(existing) = modes(connection)?
        .into_iter()
        .find(|mode| mode.stable_id == candidate.stable_id)
    {
        bail!("\"{}\" is already in this pack", existing.name);
    }
    let label = format!("Add mode \"{}\"", candidate.name);
    let id = history::record(connection, &label, candidate.size, &[], |tx| {
        Ok(tx.query_row(
            "INSERT INTO modes (file) VALUES (?) RETURNING id",
            params![bytes],
            |row| row.get(0),
        )?)
    })?;
    Ok(EmbeddedMode { id, ..candidate })
}

/// Removes a mode. If it was the recommended mode, the recommendation goes back to automatic in
/// the same undo entry.
pub fn remove_mode(connection: &Connection, id: u64) -> Result<()> {
    let bytes: Option<Vec<u8>> = connection
        .query_row("SELECT file FROM modes WHERE id = ?", [id], |row| {
            row.get(0)
        })
        .optional()?;
    let Some(bytes) = bytes else {
        bail!("That mode isn't in the pack any more");
    };
    let label = match describe_mode(id, &bytes) {
        Ok(mode) => format!("Remove mode \"{}\"", mode.name),
        Err(_) => "Remove mode".to_string(),
    };
    history::record(connection, &label, bytes.len() as u64, &[], |tx| {
        tx.execute(
            "UPDATE pack_metadata
             SET recommended_mode_kind = NULL, recommended_mode_id = NULL
             WHERE singleton = 1 AND recommended_mode_kind = 'pack' AND recommended_mode_id = ?",
            [id],
        )?;
        tx.execute("DELETE FROM modes WHERE id = ?", [id])?;
        Ok(())
    })
}
