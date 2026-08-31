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

-- Whether the pack has a `timeline` section at all, and its optional mode-name override. A row
-- means it has one; no row means it does not.
--
-- `enabled` is the difference between "this pack has no timeline" and "this pack has a timeline the
-- author has switched off". Both read as `Timeline: None` to the engine, the converter and `lw`
-- -- `read_timeline` returns `None` for either -- but only the second keeps its stage rows, so
-- switching the timeline back on restores what was there.
--
-- A column rather than the absence of a row, because absence cannot hold anything. The editor used
-- to keep the suspended timeline in front-end memory (`store.suspendedTimeline`), which lost it
-- for good the moment the pack was closed and orphaned every stage wallpaper with it. See
-- `design/behaviour-storage.md`, "Invariants to preserve".
CREATE TABLE IF NOT EXISTS behaviour_timeline (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    label TEXT,
    enabled INTEGER NOT NULL DEFAULT TRUE CHECK (enabled IN (FALSE, TRUE))
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
    restricts_content INTEGER NOT NULL DEFAULT FALSE CHECK (restricts_content IN (FALSE, TRUE)),
    wallpaper INTEGER REFERENCES media (id) ON DELETE SET NULL,
    audio INTEGER REFERENCES media (id) ON DELETE SET NULL,
    audio_random INTEGER NOT NULL DEFAULT FALSE CHECK (audio_random IN (FALSE, TRUE)),
    -- Entry and prompt settings are always represented in Rust (with defaults), so they belong
    -- on the stage rather than in optional 0..1 extension rows.
    entry_splash_media INTEGER REFERENCES media (id) ON DELETE SET NULL,
    entry_sound_media INTEGER REFERENCES media (id) ON DELETE SET NULL,
    entry_popup_burst INTEGER,
    entry_notification_text TEXT,
    prompt_timeouts_enabled INTEGER NOT NULL DEFAULT TRUE CHECK (prompt_timeouts_enabled IN (FALSE, TRUE)),
    prompt_timeout_multiplier REAL NOT NULL DEFAULT 1,
    prompt_popup_burst INTEGER,
    prompt_sound_media INTEGER REFERENCES media (id) ON DELETE SET NULL
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
-- unrestricted stage has no *included* rows and so owns no inclusion tag, which is right -- there
-- is no selection for a tag to be part of.
--
-- `excluded` is which side of the selection the row is on. A stage shows a file when its inclusion
-- list is absent or matched, *and* no excluded tag is matched -- so exclusion is the only way to
-- take one file out of a stage without editing the author's own vocabulary, which is what the
-- editor's "Appears in" toggle needs. It is a column rather than a second table because everything
-- about the two sides is the same: both are tag associations and both can be owned by the stage.
-- One `UNIQUE` constraint also states the invariant that keeps them coherent: a tag cannot be on
-- both sides of one stage's selection, because that stage would show nothing that carries it while
-- claiming to select by it.
CREATE TABLE IF NOT EXISTS behaviour_stage_tag (
    -- Tags are sets semantically. The generated id only gives their chips a stable insertion order.
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stage_id TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    owned INTEGER NOT NULL DEFAULT FALSE CHECK (owned IN (FALSE, TRUE)),
    excluded INTEGER NOT NULL DEFAULT FALSE CHECK (excluded IN (FALSE, TRUE)),
    UNIQUE (stage_id, tag_id)
) STRICT;

-- Optional sub-structures whose absence is semantically distinct get a table each: 0..1 rows says
-- "absent" without a `has_*` flag beside every column, and `Some(Movement { .. all None })` stays
-- distinct from `None`, which a flat nullable column set could not express. Entry and prompt use
-- plain Rust structs with defaults, so their fields live directly on `behaviour_stage` instead.
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
--
-- No `position` column: a transition sits between one adjacent stage pair, so its place in the
-- timeline *is* `from_stage`'s place in it, and `read_transitions` orders by that stage's
-- position. Storing a copy would be a second answer to a question the endpoints already settle,
-- free to disagree with them -- `normalize_timeline` only ever set it to `from_stage`'s position
-- anyway. Nothing downstream reads the order regardless: `Transition` carries no index across the
-- document, and the default mode finds one by scanning for matching endpoints.
CREATE TABLE IF NOT EXISTS behaviour_transition (
    id TEXT PRIMARY KEY,
    from_stage TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    to_stage TEXT NOT NULL REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    duration_seconds REAL NOT NULL,
    easing TEXT NOT NULL
) STRICT;
CREATE INDEX IF NOT EXISTS behaviour_transition_from_stage
    ON behaviour_transition (from_stage);

-- `Transition::affected` is a set: the engine asks whether a category is in it, and the editor
-- toggles one member at a time. The order the author happened to tick them in was never part of
-- the answer, so the rows carry no `position` and are read back in category order -- which makes
-- one set serialise one way, rather than however the last author clicked it together.
CREATE TABLE IF NOT EXISTS behaviour_transition_category (
    transition_id TEXT NOT NULL REFERENCES behaviour_transition (id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    PRIMARY KEY (transition_id, category)
) STRICT;

-- The three text pools share a table: they are the same shape (some text, some tags) and differ
-- only in which pool a mode draws from, so `kind` is the pool rather than three near-identical
-- tables and three near-identical readers. `summary` belongs to the `Notification` domain type, a
-- title alongside the body; only the 'notification' kind uses it.
-- `AUTOINCREMENT` rather than a plain rowid: SQLite reuses the highest rowid once its row is gone,
-- so removing the last entry and adding another would hand the new one the id the editor is still
-- showing on the old one's Remove button. An id has to outlive the row it named.
--
-- That monotonic id is also the pool's order, which is why there is no `position` column here the
-- way `behaviour_stage` and `behaviour_content_group` have one. A pool is only ever appended to
-- (the three `add_*` editor functions) or rebuilt whole from the document in array order
-- (`write_text_pool`), so
-- ascending id *is* author order, and removing an entry leaves a hole that changes nothing.
-- Reordering or inserting mid-pool would break that and want a real position column back.
CREATE TABLE IF NOT EXISTS behaviour_text_item (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (
        kind IN ('caption', 'prompt', 'notification')
    ),
    text TEXT NOT NULL,
    summary TEXT,
    timeout_seconds REAL
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_text_item_tag (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id INTEGER NOT NULL REFERENCES behaviour_text_item (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    UNIQUE (item_id, tag_id)
) STRICT;

-- Ordered by its `AUTOINCREMENT` id, for the same reason `behaviour_text_item` is: links are
-- appended or rebuilt from the document in order, never reordered.
CREATE TABLE IF NOT EXISTS behaviour_web_link (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_web_link_tag (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    link_id INTEGER NOT NULL REFERENCES behaviour_web_link (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    UNIQUE (link_id, tag_id)
) STRICT;

-- `key` is the author-facing stable name the resolver turns into a mode option
-- (`content_group.<key>`), and `UNIQUE` because two groups answering to one option name is the
-- collision the author has to resolve. It is not the primary key, though: an `AUTOINCREMENT`
-- surrogate is, which buys two things. The list's order is that id, the way the text pools' is --
-- groups are appended and never reordered, so there is no `position` column to keep dense. And
-- the tag table hangs off an integer that never changes, so renaming a group is an UPDATE of one
-- column rather than a foreign key moving under its own rows.
CREATE TABLE IF NOT EXISTS behaviour_content_group (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    description TEXT,
    enabled_by_default INTEGER NOT NULL CHECK (enabled_by_default IN (FALSE, TRUE))
) STRICT;

CREATE TABLE IF NOT EXISTS behaviour_content_group_tag (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER NOT NULL REFERENCES behaviour_content_group (id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    UNIQUE (group_id, tag_id)
) STRICT;

-- Per-item behaviour attributes: what a pack author says about *this file*, rather than about a
-- tag, a stage, or the pack as a whole.
--
-- Every value column is nullable, and a stored row exists only where the author set something.
-- "Unset" has to stay distinguishable from "set to whatever happens to be today's default",
-- because defaults move under the user across engine releases -- see
-- `behaviour-design/default-mode.md`'s Ownership section.
--
-- `weight` is relative frequency, `scale` multiplies the size the mode would otherwise have
-- chosen, the four `region_*` columns are the part of the monitor it may spawn in, `monitor` is
-- which screen it prefers, `caption` is a caption pinned to this file (as opposed to the
-- tag-matched pool), and the two `video_*` columns are the `VideoPopupOpts` fields that are
-- properties of the clip rather than of the user.
--
-- The view has one row for every media item, including files with no stored attributes. That is
-- what makes each inspector edit an UPDATE: its INSTEAD OF trigger creates the sparse backing row
-- on the first non-NULL value and deletes it when the last value is cleared. The backing table's
-- CHECK makes the no-empty-row rule true even for SQL that bypasses the view.
--
-- Four region columns rather than one `anchor TEXT`, which is what this started as. A region
-- subsumes the nine anchors -- the engine centres a window too big for its region and clamps it to
-- the screen, so a zero-size region names one placement exactly -- while also expressing
-- "somewhere in the left half", which an anchor cannot. The four move together: all set, or all
-- NULL.
CREATE TABLE IF NOT EXISTS behaviour_popup_media_fields (
    media_id INTEGER PRIMARY KEY REFERENCES media (id) ON DELETE CASCADE,
    weight REAL,
    scale REAL,
    region_x REAL,
    region_y REAL,
    region_width REAL,
    region_height REAL,
    monitor TEXT CHECK (monitor IN ('any', 'primary')),
    caption TEXT,
    video_loop INTEGER CHECK (video_loop IN (FALSE, TRUE)),
    video_audio INTEGER CHECK (video_audio IN (FALSE, TRUE)),
    video_volume REAL,
    CHECK (weight IS NOT NULL
        OR scale IS NOT NULL
        OR region_x IS NOT NULL
        OR region_y IS NOT NULL
        OR region_width IS NOT NULL
        OR region_height IS NOT NULL
        OR monitor IS NOT NULL
        OR caption IS NOT NULL
        OR video_loop IS NOT NULL
        OR video_audio IS NOT NULL
        OR video_volume IS NOT NULL)
) STRICT;

CREATE VIEW IF NOT EXISTS behaviour_popup_media AS
SELECT media.id AS media_id,
       fields.weight,
       fields.scale,
       fields.region_x,
       fields.region_y,
       fields.region_width,
       fields.region_height,
       fields.monitor,
       fields.caption,
       fields.video_loop,
       fields.video_audio,
       fields.video_volume
FROM media
LEFT JOIN behaviour_popup_media_fields AS fields ON fields.media_id = media.id;

CREATE TRIGGER IF NOT EXISTS behaviour_popup_media_update
INSTEAD OF UPDATE ON behaviour_popup_media
BEGIN
    SELECT RAISE(ABORT, 'behaviour popup media id cannot be changed')
    WHERE NEW.media_id IS NOT OLD.media_id;

    DELETE FROM behaviour_popup_media_fields
    WHERE media_id = OLD.media_id
      AND NEW.weight IS NULL
      AND NEW.scale IS NULL
      AND NEW.region_x IS NULL
      AND NEW.region_y IS NULL
      AND NEW.region_width IS NULL
      AND NEW.region_height IS NULL
      AND NEW.monitor IS NULL
      AND NEW.caption IS NULL
      AND NEW.video_loop IS NULL
      AND NEW.video_audio IS NULL
      AND NEW.video_volume IS NULL;

    UPDATE behaviour_popup_media_fields
    SET weight = NEW.weight,
        scale = NEW.scale,
        region_x = NEW.region_x,
        region_y = NEW.region_y,
        region_width = NEW.region_width,
        region_height = NEW.region_height,
        monitor = NEW.monitor,
        caption = NEW.caption,
        video_loop = NEW.video_loop,
        video_audio = NEW.video_audio,
        video_volume = NEW.video_volume
    WHERE media_id = OLD.media_id;

    INSERT INTO behaviour_popup_media_fields
        (media_id, weight, scale, region_x, region_y, region_width, region_height,
         monitor, caption, video_loop, video_audio, video_volume)
    SELECT NEW.media_id, NEW.weight, NEW.scale,
           NEW.region_x, NEW.region_y, NEW.region_width, NEW.region_height,
           NEW.monitor, NEW.caption, NEW.video_loop, NEW.video_audio, NEW.video_volume
    WHERE (NEW.weight IS NOT NULL
        OR NEW.scale IS NOT NULL
        OR NEW.region_x IS NOT NULL
        OR NEW.region_y IS NOT NULL
        OR NEW.region_width IS NOT NULL
        OR NEW.region_height IS NOT NULL
        OR NEW.monitor IS NOT NULL
        OR NEW.caption IS NOT NULL
        OR NEW.video_loop IS NOT NULL
        OR NEW.video_audio IS NOT NULL
        OR NEW.video_volume IS NOT NULL)
      AND NOT EXISTS (
          SELECT 1 FROM behaviour_popup_media_fields WHERE media_id = NEW.media_id
      );
END;

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
-- Both sides reference media directly. A pairing is itself something the popup entry says, so it
-- must be able to exist without manufacturing an all-NULL row in the scalar-field backing table.
CREATE TABLE IF NOT EXISTS behaviour_popup_audio_pair (
    popup_media_id INTEGER NOT NULL
        REFERENCES media (id) ON DELETE CASCADE,
    audio_media_id INTEGER NOT NULL REFERENCES media (id) ON DELETE CASCADE,
    PRIMARY KEY (popup_media_id, audio_media_id)
) STRICT;
