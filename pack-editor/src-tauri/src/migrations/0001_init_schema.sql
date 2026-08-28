-- The editor working copy's base schema: the media it knows about (soft-deleted, and either
-- inside the archive or loose on disk), the undo history, and the save bookkeeping.
-- `shared/src/migrations/behaviour_schema.sql` is concatenated onto this to form the whole of
-- migration 1; the behaviour tables reference `media` and `tags` from here.

CREATE TABLE IF NOT EXISTS editor_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    archive_generation TEXT,
    current_state_id TEXT NOT NULL,
    saved_state_id TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS pack_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    name TEXT NOT NULL,
    creator TEXT,
    description TEXT,
    version TEXT,
    recommended_mode_kind TEXT
        CHECK (recommended_mode_kind IS NULL OR recommended_mode_kind IN ('sandbox', 'experience', 'pack')),
    recommended_mode_id INTEGER,
    CHECK (
        CASE recommended_mode_kind
            WHEN 'sandbox' THEN recommended_mode_id IS NULL
            WHEN 'experience' THEN recommended_mode_id IS NULL
            WHEN 'pack' THEN recommended_mode_id IS NOT NULL
            ELSE recommended_mode_kind IS NULL AND recommended_mode_id IS NULL
        END
    )
) STRICT;

CREATE TABLE IF NOT EXISTS media (
    id INTEGER PRIMARY KEY,
    file_name TEXT NOT NULL UNIQUE,
    file_type TEXT CHECK (file_type IN ('image', 'video', 'audio')) NOT NULL,
    width INTEGER,
    height INTEGER,
    transparent INTEGER,
    duration REAL,
    audio INTEGER,
    hash BLOB NOT NULL UNIQUE,
    thumbnail BLOB,
    source_url TEXT,
    deleted INTEGER NOT NULL DEFAULT FALSE CHECK (deleted IN (FALSE, TRUE))
) STRICT;
CREATE INDEX IF NOT EXISTS media_hash_index ON media (hash);

CREATE TABLE IF NOT EXISTS pack_media (
    media_id INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    generation_id TEXT NOT NULL,
    "offset" INTEGER NOT NULL,
    length INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS loose_media (
    media_id INTEGER PRIMARY KEY REFERENCES media(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    length INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS media_tags (
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (media_id, tag_id)
) STRICT;

CREATE TABLE IF NOT EXISTS artists (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS media_artists (
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
    PRIMARY KEY (media_id, artist_id)
) STRICT;

CREATE TABLE IF NOT EXISTS modes (
    id INTEGER PRIMARY KEY,
    file BLOB NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS pack_data (
    name TEXT PRIMARY KEY,
    blob BLOB NOT NULL
) STRICT;

-- `id` orders history as well as identifying it, which is why it is `AUTOINCREMENT`: entries are
-- appended, and the id SQLite mints is therefore always past every id already there. There is no
-- separate `sequence` column, and nothing renumbers.
--
-- `history_state.cursor` is what made a dense sequence look necessary. It is an *id boundary*, not
-- a count: entries below it are done and undoable, entries at or above it are undone and redoable,
-- and past the newest entry it holds a sentinel one greater than the last id minted. Trimming the
-- oldest entry, or dropping a pending one from the middle, then removes a row and moves nothing --
-- where a dense sequence had to shift every surviving row (through negative intermediates, to dodge
-- its own UNIQUE constraint) and drag the cursor along behind them.
CREATE TABLE IF NOT EXISTS history_entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    forward_changeset BLOB,
    inverse_changeset BLOB,
    payload TEXT,
    storage_bytes INTEGER NOT NULL DEFAULT 0,
    before_state_id TEXT NOT NULL,
    after_state_id TEXT,
    status TEXT NOT NULL DEFAULT 'ready' CHECK (status IN ('pending', 'ready'))
) STRICT;
CREATE TABLE IF NOT EXISTS history_media_refs (
    history_id INTEGER NOT NULL REFERENCES history_entries(id) ON DELETE CASCADE,
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    PRIMARY KEY (history_id, media_id)
) STRICT;
CREATE INDEX IF NOT EXISTS history_media_refs_media_index ON history_media_refs(media_id);
CREATE TABLE IF NOT EXISTS history_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    cursor INTEGER NOT NULL DEFAULT 0
) STRICT;
INSERT OR IGNORE INTO history_state(singleton, cursor) VALUES (1, 0);

CREATE TABLE IF NOT EXISTS save_sessions (
    id TEXT PRIMARY KEY,
    generation_id TEXT NOT NULL,
    editor_state_id TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS snapshot_media (
    save_id TEXT NOT NULL REFERENCES save_sessions(id) ON DELETE CASCADE,
    media_id INTEGER NOT NULL REFERENCES media(id) ON DELETE CASCADE,
    PRIMARY KEY (save_id, media_id)
) STRICT;
CREATE INDEX IF NOT EXISTS snapshot_media_media_index ON snapshot_media(media_id);

CREATE TABLE IF NOT EXISTS pending_file_deletions (
    path TEXT PRIMARY KEY
) STRICT;
