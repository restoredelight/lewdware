CREATE TABLE IF NOT EXISTS media (
    id INTEGER PRIMARY KEY,
    file_name TEXT NOT NULL,
    file_type TEXT CHECK (file_type IN ('image', 'video', 'audio')) NOT NULL,
    "offset" INTEGER,
    length INTEGER,
    path TEXT,
    width INTEGER,
    height INTEGER,
    transparent INTEGER,
    duration REAL,
    audio INTEGER,
    hash BLOB NOT NULL,
    thumbnail BLOB
) STRICT;

CREATE INDEX media_hash_index ON media (hash);

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

CREATE TABLE IF NOT EXISTS modes (
    id INTEGER PRIMARY KEY,
    "file" BLOB NOT NULL
) STRICT;
