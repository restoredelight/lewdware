ALTER TABLE media ADD COLUMN source_url TEXT;

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
