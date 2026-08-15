-- The behaviour document's structural half: the media slots and the timeline.
--
-- Held as rows rather than inside the `behaviour` blob in `pack_data` so that a media slot is a
-- foreign key the database enforces rather than one written out longhand, and so that editing one
-- stage produces a changeset the size of that stage rather than of the whole document. The content
-- pools (captions, prompts, notifications, subliminals, web links, content groups) are still in
-- the blob; they move next. See `design/behaviour-storage.md`.
--
-- Included by *both* migration ledgers -- `shared::db` for the runtime pack and
-- `pack_editor::editor_db` for the editor's working copy -- from this one file, because the two
-- schemas diverging here would be a silent corruption rather than a compile error.

-- `Content`'s two media slots. A singleton row rather than 0..1, so writing a slot is one UPDATE
-- and "no wallpaper" is a NULL column rather than an absent row.
CREATE TABLE IF NOT EXISTS behaviour_content (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    wallpaper INTEGER REFERENCES media (id) ON DELETE SET NULL,
    splash INTEGER REFERENCES media (id) ON DELETE SET NULL
) STRICT;
INSERT OR IGNORE INTO behaviour_content (singleton) VALUES (1);

-- Whether the pack has an `experience` section at all, and its optional mode-name override. A row
-- means it has one; no row means it does not, which is also how a suspended timeline reads.
CREATE TABLE IF NOT EXISTS behaviour_experience (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    label TEXT
) STRICT;

-- `position` orders the timeline and is not UNIQUE on purpose: reordering rewrites positions one
-- row at a time, and a uniqueness constraint would make every swap need a temporary value.
CREATE TABLE IF NOT EXISTS behaviour_stage (
    id TEXT PRIMARY KEY,
    position INTEGER NOT NULL,
    label TEXT NOT NULL,
    -- `ContentSelection::tags` has three states: no restriction at all (`None`), or a set that may
    -- be empty (`Some([])`, "deliberately no content"). Absent rows cannot tell those apart, so
    -- the flag says which of the two an empty tag list means.
    restricts_content INTEGER NOT NULL DEFAULT 0 CHECK (restricts_content IN (0, 1)),
    wallpaper INTEGER REFERENCES media (id) ON DELETE SET NULL
) STRICT;
CREATE INDEX IF NOT EXISTS behaviour_stage_position ON behaviour_stage (position);

-- Tags are still stored by name here. They become a join to `tags` when the content pools move,
-- so that all seven tag-bearing lists gain the foreign key in one step rather than this one list
-- gaining it early and the rest waiting.
CREATE TABLE IF NOT EXISTS behaviour_stage_tag (
    stage_id TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (stage_id, tag)
) STRICT;

-- The three optional sub-structures of a stage get a table each: 0..1 rows says "absent" without
-- a `has_*` flag beside every column, and `Some(Movement { .. all None })` stays distinct from
-- `None`, which a flat nullable column set could not express.
CREATE TABLE IF NOT EXISTS behaviour_stage_end (
    stage_id TEXT PRIMARY KEY REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    duration_seconds REAL,
    strategy TEXT NOT NULL,
    -- One `EventCountCondition`: all three together, or all three NULL.
    event_kind TEXT,
    event_count INTEGER,
    event_scope TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_stage_movement (
    stage_id TEXT PRIMARY KEY REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    minimum_speed REAL,
    maximum_speed REAL
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_stage_mitosis (
    stage_id TEXT PRIMARY KEY REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    chance REAL,
    count INTEGER
) STRICT;

-- One row per event kind the stage schedules. `interval_kind` picks which of the interval columns
-- carry the value, mirroring the `Interval` enum's own tag.
CREATE TABLE IF NOT EXISTS behaviour_stage_event (
    stage_id TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    interval_kind TEXT NOT NULL,
    seconds REAL,
    minimum_seconds REAL,
    maximum_seconds REAL,
    initial_delay_seconds REAL,
    max_concurrent INTEGER,
    PRIMARY KEY (stage_id, kind)
) STRICT;

-- Cascading from both ends: a transition only means anything between two stages that exist, so
-- removing either takes it with them. The editor normalizes the timeline itself, but the database
-- should not be able to hold a transition from nowhere.
CREATE TABLE IF NOT EXISTS behaviour_transition (
    id TEXT PRIMARY KEY,
    position INTEGER NOT NULL,
    from_stage TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    to_stage TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    duration_seconds REAL NOT NULL,
    easing TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS behaviour_transition_position ON behaviour_transition (position);

CREATE TABLE IF NOT EXISTS behaviour_transition_category (
    transition_id TEXT NOT NULL REFERENCES behaviour_transition (id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (transition_id, category)
) STRICT;
