CREATE TABLE media_archive_only (
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

INSERT INTO media_archive_only
    (id, file_name, file_type, "offset", length, width, height, transparent, duration,
     audio, hash, thumbnail, source_url)
SELECT id, file_name, file_type, "offset", length, width, height, transparent, duration,
       audio, hash, thumbnail, source_url
FROM media;

DROP TABLE media;
ALTER TABLE media_archive_only RENAME TO media;
CREATE INDEX media_hash_index ON media (hash);
