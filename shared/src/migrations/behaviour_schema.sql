-- The behaviour document, as tables.
--
-- Held as rows rather than inside a `behaviour` blob in `pack_data` so that a media slot is a
-- foreign key the database enforces rather than one written out longhand, so that a tag is a join
-- rather than a name copied into seven lists, and so that editing one stage produces a changeset
-- the size of that stage rather than of the whole document. See `design/behaviour-storage.md` and
-- `behaviour-design/default-mode-v2.md`.
--
-- Not a migration on its own: it is the second half of the first migration in *both* ledgers --
-- `shared::db` for the runtime pack and `pack_editor::editor_db` for the editor's working copy --
-- concatenated onto whichever base schema that ledger starts from. One file rather than two
-- hand-kept copies, because the two schemas diverging here would be a silent corruption rather
-- than a compile error. It assumes the base schema's `media` and `tags` tables already exist.

-- `Content`'s two media slots. A singleton row rather than 0..1, so writing a slot is one UPDATE
-- and "no wallpaper" is a NULL column rather than an absent row.
CREATE TABLE IF NOT EXISTS behaviour_content (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    wallpaper INTEGER REFERENCES media (id) ON DELETE SET NULL,
    splash INTEGER REFERENCES media (id) ON DELETE SET NULL
) STRICT;
INSERT OR IGNORE INTO behaviour_content (singleton) VALUES (1);

-- Whether the pack has an `experience` section at all, and its optional mode-name override. A row
-- means it has one; no row means it does not.
--
-- `enabled` is the difference between "this pack has no timeline" and "this pack has a timeline the
-- author has switched off". Both read as `Experience: None` to the engine, the converter and `lw`
-- -- `read_experience` returns `None` for either -- but only the second keeps its stage rows, so
-- switching the timeline back on restores what was there.
--
-- A column rather than the absence of a row, because absence cannot hold anything. The editor used
-- to keep the suspended timeline in front-end memory (`store.suspendedExperience`), which lost it
-- for good the moment the pack was closed and orphaned every stage wallpaper with it. See
-- `design/behaviour-storage.md`, "Invariants to preserve".
CREATE TABLE IF NOT EXISTS behaviour_experience (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    label TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
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
    wallpaper INTEGER REFERENCES media (id) ON DELETE SET NULL,
    audio INTEGER REFERENCES media (id) ON DELETE SET NULL,
    audio_random INTEGER NOT NULL DEFAULT 0 CHECK (audio_random IN (0, 1))
) STRICT;
CREATE INDEX IF NOT EXISTS behaviour_stage_position ON behaviour_stage (position);

-- `owned` is which of a stage's tags the editor created for it, and therefore maintains the name
-- of. See `behaviour-design/default-mode-v2.md`, "Stage tags: created, owned, renamed and retired
-- with the stage". A tag the author added by hand -- `imp`, `succubus`, everything the Edgeware
-- converter produces -- means something outside the timeline, and rewriting one to tidy a stage
-- name would be much worse than the tidiness it buys.
--
-- The flag lives here rather than on `behaviour_stage` so that "the stage owns this tag" cannot be
-- recorded for a tag the stage does not actually select by: the row *is* the association. An
-- unrestricted stage has no rows and so owns nothing, which is right -- there is no selection for
-- a tag to be part of.
CREATE TABLE IF NOT EXISTS behaviour_stage_tag (
    stage_id TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    owned INTEGER NOT NULL DEFAULT 0 CHECK (owned IN (0, 1)),
    PRIMARY KEY (stage_id, tag_id)
) STRICT;

-- The optional sub-structures of a stage get a table each: 0..1 rows says "absent" without a
-- `has_*` flag beside every column, and `Some(Movement { .. all None })` stays distinct from
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

-- What the stage does on entry: a splash, a sound, a burst of popups, a notification.
CREATE TABLE IF NOT EXISTS behaviour_stage_entry (
    stage_id TEXT PRIMARY KEY REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    splash_media INTEGER REFERENCES media (id) ON DELETE SET NULL,
    sound_media INTEGER REFERENCES media (id) ON DELETE SET NULL,
    popup_burst INTEGER,
    notification_text TEXT
) STRICT;

-- Prompt behaviour the stage owns, as opposed to the prompt text itself.
CREATE TABLE IF NOT EXISTS behaviour_stage_prompt (
    stage_id TEXT PRIMARY KEY REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    timeouts_enabled INTEGER NOT NULL DEFAULT 1 CHECK (timeouts_enabled IN (0, 1)),
    -- Scales the prompt's explicit or automatically derived deadline.
    timeout_multiplier REAL NOT NULL DEFAULT 1,
    popup_burst INTEGER,
    sound_media INTEGER REFERENCES media (id) ON DELETE SET NULL
) STRICT;

-- One row per event kind the stage schedules. `interval_kind` picks which of the interval columns
-- carry the value, mirroring the `Interval` enum's own tag. `kind` is open-ended rather than a
-- CHECK list, so that a new event kind is a reader change rather than a schema change.
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

-- The three text pools share a table: they are the same shape (some text, some tags) and differ
-- only in which pool a mode draws from, so `kind` is the pool rather than three near-identical
-- tables and three near-identical readers. `summary` is `TextItem::summary`, a title alongside the
-- body; only the 'notification' kind uses it.
CREATE TABLE IF NOT EXISTS behaviour_text_item (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (
        kind IN ('caption', 'prompt', 'notification')
    ),
    position INTEGER NOT NULL,
    text TEXT NOT NULL,
    summary TEXT,
    timeout_seconds REAL,
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

-- `ON UPDATE CASCADE` because the group's id is author-facing text, not a surrogate: renaming a
-- group is a rename of the key its rows point at, and without the cascade it silently orphans them.
CREATE TABLE IF NOT EXISTS behaviour_content_group_tag (
    group_id TEXT NOT NULL
        REFERENCES behaviour_content_group (id) ON DELETE CASCADE ON UPDATE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (group_id, tag_id)
) STRICT;

-- Per-item behaviour attributes: what a pack author says about *this file*, rather than about a
-- tag, a stage, or the pack as a whole.
--
-- Every value column is nullable, and a row exists only where the author set something. "Unset"
-- has to stay distinguishable from "set to whatever happens to be today's default", because
-- defaults move under the user across engine releases -- see `behaviour-design/default-mode.md`'s
-- Ownership section. A row of all-NULLs is not written; `storage::write` drops it.
--
-- `weight` is relative frequency, `scale` multiplies the size the mode would otherwise have
-- chosen, the four `region_*` columns are the part of the monitor it may spawn in, `monitor` is
-- which screen it prefers, `caption` is a caption pinned to this file (as opposed to the
-- tag-matched pool), and the two `video_*` columns are the `VideoPopupOpts` fields that are
-- properties of the clip rather than of the user.
--
-- Four region columns rather than one `anchor TEXT`, which is what this started as. A region
-- subsumes the nine anchors -- the engine centres a window too big for its region and clamps it to
-- the screen, so a zero-size region names one placement exactly -- while also expressing
-- "somewhere in the left half", which an anchor cannot. The four move together: all set, or all
-- NULL.
CREATE TABLE IF NOT EXISTS behaviour_popup_media (
    media_id INTEGER PRIMARY KEY REFERENCES media (id) ON DELETE CASCADE,
    weight REAL,
    scale REAL,
    region_x REAL,
    region_y REAL,
    region_width REAL,
    region_height REAL,
    monitor TEXT CHECK (monitor IN ('any', 'primary')),
    caption TEXT,
    video_loop INTEGER CHECK (video_loop IN (0, 1)),
    video_audio INTEGER CHECK (video_audio IN (0, 1))
) STRICT;

-- Volume only. A `loop` column was considered and dropped: see `AudioMedia`'s doc comment -- it
-- is expressible without an option, and as one it silently stops the background rotation.
CREATE TABLE IF NOT EXISTS behaviour_audio_media (
    media_id INTEGER PRIMARY KEY REFERENCES media (id) ON DELETE CASCADE,
    volume REAL
) STRICT;

-- Explicit popup-to-sound pairings, which tag matching cannot express. Many-to-many, so its own
-- table rather than a column.
--
-- A set, not a list: the mode picks one of the eligible sounds at random, so there is no order to
-- preserve and no reason for the `position` column the document's other lists carry.
--
-- The popup side references `behaviour_popup_media` rather than `media` directly, because in the
-- document a pairing is a *field of* the popup's entry (`PopupMedia::audio`) -- an entry that
-- exists precisely because it has something to say. Cascading from there keeps the two in step.
CREATE TABLE IF NOT EXISTS behaviour_popup_audio_pair (
    popup_media_id INTEGER NOT NULL
        REFERENCES behaviour_popup_media (media_id) ON DELETE CASCADE,
    audio_media_id INTEGER NOT NULL REFERENCES media (id) ON DELETE CASCADE,
    PRIMARY KEY (popup_media_id, audio_media_id)
) STRICT;
