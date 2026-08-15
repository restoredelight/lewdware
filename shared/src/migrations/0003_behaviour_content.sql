-- The rest of the behaviour document: the content pools, the web links, the content groups, and
-- the one settings field. With these, nothing is left in the `pack_data` blob.
--
-- The reason to finish the job rather than stop at the timeline is the tags. Behaviour named them
-- by string, so renaming or deleting one meant rewriting the document by hand
-- (`Behaviour::rewrite_tag`, and the three tag commands that called it). As joins they are foreign
-- keys: a rename is `UPDATE tags SET name`, a merge is a re-point, and a delete is a cascade.
--
-- Included by both migration ledgers from this one file, for the reason given in
-- `0002_behaviour_timeline.sql`.

-- Stage tags were stored by name in 0002, deliberately, so that all seven tag-bearing lists could
-- gain the foreign key together rather than this one gaining it early. Recreated rather than
-- altered: nothing is released, and `storage::read` refuses a document from before this migration
-- anyway (see below).
DROP TABLE IF EXISTS behaviour_stage_tag;
CREATE TABLE behaviour_stage_tag (
    stage_id TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (stage_id, tag_id)
) STRICT;

-- The four text pools share a table: they are the same shape (some text, some tags) and differ
-- only in which pool a mode draws from, so `kind` is the pool rather than four near-identical
-- tables and four near-identical readers.
CREATE TABLE IF NOT EXISTS behaviour_text_item (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (
        kind IN ('caption', 'prompt', 'notification', 'subliminal')
    ),
    position INTEGER NOT NULL,
    text TEXT NOT NULL,
    -- Pool entries have no identity outside the document, so a write addresses them by position
    -- and upserts in place. That needs the position to be something to conflict on.
    UNIQUE (kind, position)
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_text_item_tag (
    item_id INTEGER NOT NULL REFERENCES behaviour_text_item (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (item_id, tag_id)
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_web_link (
    id INTEGER PRIMARY KEY,
    position INTEGER NOT NULL UNIQUE,
    url TEXT NOT NULL
) STRICT;

-- Suffixes appended at random when the link is opened. Ordered but not a set: the same suffix
-- twice is a legitimate way to weight it, so `position` is part of the key rather than the value.
CREATE TABLE IF NOT EXISTS behaviour_web_link_arg (
    link_id INTEGER NOT NULL REFERENCES behaviour_web_link (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (link_id, position)
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_web_link_tag (
    link_id INTEGER NOT NULL REFERENCES behaviour_web_link (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (link_id, tag_id)
) STRICT;

-- `id` is the author-facing stable key the resolver turns into a mode option
-- (`content_group.<id>`), so it is the primary key rather than a surrogate.
CREATE TABLE IF NOT EXISTS behaviour_content_group (
    id TEXT PRIMARY KEY,
    position INTEGER NOT NULL,
    label TEXT NOT NULL,
    description TEXT,
    enabled_by_default INTEGER NOT NULL CHECK (enabled_by_default IN (0, 1))
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_content_group_tag (
    group_id TEXT NOT NULL REFERENCES behaviour_content_group (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (group_id, tag_id)
) STRICT;

-- Everything left over that is one value per pack rather than a list.
CREATE TABLE IF NOT EXISTS behaviour_settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    prompt_submit_label TEXT
) STRICT;
INSERT OR IGNORE INTO behaviour_settings (singleton) VALUES (1);
