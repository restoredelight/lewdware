CREATE TABLE IF NOT EXISTS editor_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    archive_generation TEXT,
    current_state_id TEXT NOT NULL,
    saved_state_id TEXT
) STRICT;
CREATE TABLE IF NOT EXISTS pack_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    metadata BLOB NOT NULL
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
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1))
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

CREATE TABLE IF NOT EXISTS history_entries (
    id INTEGER PRIMARY KEY,
    sequence INTEGER NOT NULL UNIQUE,
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
