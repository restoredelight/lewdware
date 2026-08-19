-- The runtime pack's base schema: the files, who made them, what they are tagged with, and the
-- modes and loose blobs that travel alongside. `migrations/behaviour_schema.sql` is concatenated
-- onto this to form the whole of migration 1; the behaviour tables reference `media` and `tags`
-- from here.

CREATE TABLE IF NOT EXISTS media (
    id INTEGER PRIMARY KEY,
    file_name TEXT NOT NULL UNIQUE,
    file_type TEXT CHECK (file_type IN ('image', 'video', 'audio')) NOT NULL,
    "offset" INTEGER NOT NULL,
    length INTEGER NOT NULL,
    width INTEGER,
    height INTEGER,
    transparent INTEGER,
    duration REAL,
    audio INTEGER,
    hash BLOB NOT NULL UNIQUE,
    thumbnail BLOB,
    source_url TEXT
) STRICT;

CREATE INDEX IF NOT EXISTS media_hash_index ON media (hash);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS media_tags (
    media_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    PRIMARY KEY (media_id, tag_id),
    FOREIGN KEY (media_id) REFERENCES media (id) ON DELETE CASCADE,
    FOREIGN KEY (tag_id) REFERENCES tags (id) ON DELETE CASCADE
) STRICT;

CREATE TABLE IF NOT EXISTS artists (
    id INTEGER PRIMARY KEY,
    name TEXT UNIQUE NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS media_artists (
    media_id INTEGER NOT NULL,
    artist_id INTEGER NOT NULL,
    PRIMARY KEY (media_id, artist_id),
    FOREIGN KEY (media_id) REFERENCES media (id) ON DELETE CASCADE,
    FOREIGN KEY (artist_id) REFERENCES artists (id) ON DELETE CASCADE
) STRICT;

CREATE TABLE IF NOT EXISTS modes (
    id INTEGER PRIMARY KEY,
    "file" BLOB NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS pack_data (
    name TEXT PRIMARY KEY,
    blob BLOB NOT NULL
) STRICT;
