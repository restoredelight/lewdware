//! Reading a pack or a mode file off disk.
//!
//! Ported from `config/src-tauri/src/modes.rs` and `state.rs`. The Tauri copy goes away when this
//! crate replaces it, so the two are deliberately not shared in the meantime.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::PathBuf;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use shared::{
    behaviour::Behaviour,
    db::migrate,
    mode::{self, Metadata},
    pack::{Header, Metadata as PackMetadata, RecommendedMode, HEADER_SIZE},
};
use tempfile::NamedTempFile;
use uuid::Uuid;

pub struct PackModeEntry {
    pub id: u64,
    pub metadata: Metadata,
}

pub struct UploadedModeEntry {
    pub path: PathBuf,
    pub metadata: Metadata,
}

pub struct LoadedPack {
    /// The pack's index, copied out so SQLite can open it. Held for as long as the pack is.
    pub _db_file: NamedTempFile,
    pub id: Uuid,
    pub metadata: PackMetadata,
    pub modes: Vec<PackModeEntry>,
    /// The pack's behaviour.json, or empty. Only the built-in default modes consult it.
    pub behaviour: Behaviour,
    pub recommended_mode: Option<RecommendedMode>,
}

impl LoadedPack {
    /// A label the pack's timeline gives the Sequence mode (e.g. "Corruption"), ignored when the
    /// timeline is empty because there is no progression to name.
    pub fn timeline_label(&self) -> Option<&str> {
        self.behaviour
            .timeline
            .as_ref()
            .filter(|timeline| !timeline.stages.is_empty())
            .and_then(|timeline| timeline.label.as_deref())
    }
}

pub fn load_pack(path: PathBuf) -> anyhow::Result<LoadedPack> {
    let mut file = std::fs::File::open(&path)?;
    let mut header_bytes = [0; HEADER_SIZE];
    file.read_exact(&mut header_bytes)?;
    let header = Header::from_buf(header_bytes)?;

    let mut db_file = NamedTempFile::new()?;
    file.seek(SeekFrom::Start(header.index_offset))?;
    let mut db_data = (&mut file).take(header.index_length);
    std::io::copy(&mut db_data, db_file.as_file_mut())?;

    let manager = SqliteConnectionManager::file(db_file.path());
    let pool = Pool::builder().build(manager)?;
    let mut conn = pool.get()?;
    migrate(&mut conn)?;
    let metadata = PackMetadata::read(&conn)?;

    let mut stmt = conn.prepare("SELECT id, file FROM modes")?;
    let modes = stmt
        .query_and_then([], |row| {
            let id = row.get("id")?;
            let data: Vec<u8> = row.get("file")?;
            let (_, metadata) = mode::read_mode_metadata(&mut Cursor::new(data))?;
            Ok(PackModeEntry { id, metadata })
        })?
        .collect::<anyhow::Result<Vec<_>>>()?;
    drop(stmt);

    // A pack whose behaviour cannot be read still opens; it just offers no behaviour-derived
    // options, which is what an empty document means anyway.
    let behaviour = Behaviour::read(&conn)
        .inspect_err(|error| tracing::warn!("Could not read the pack's behaviour: {error}"))
        .unwrap_or_default();

    Ok(LoadedPack {
        _db_file: db_file,
        id: header.id,
        recommended_mode: metadata.recommended_mode.clone(),
        metadata,
        modes,
        behaviour,
    })
}

pub fn load_mode_file(path: PathBuf) -> anyhow::Result<UploadedModeEntry> {
    let mut file = std::fs::File::open(&path)?;
    let (_, metadata) = mode::read_mode_metadata(&mut file)?;
    Ok(UploadedModeEntry { path, metadata })
}
