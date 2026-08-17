-- Per-item behaviour attributes: what a pack author says about *this file*, rather than about a
-- tag, a stage, or the pack as a whole. See `behaviour-design/default-mode-v2.md`.
--
-- Rows rather than a map inside the document, for the reason the timeline moved into tables
-- (`0002`): the media id is a foreign key the database enforces, so deleting a file takes its
-- attributes with it through ON DELETE CASCADE rather than needing a hand-written pass like
-- `Behaviour::clear_media_reference`. And these edits are per-file across potentially thousands of
-- files, so "editing one file produces a changeset the size of one file" matters more here than it
-- did for the content pools.
--
-- Every value column is nullable, and a row exists only where the author set something. "Unset"
-- has to stay distinguishable from "set to whatever happens to be today's default", because
-- defaults move under the user across engine releases -- see `behaviour-design/default-mode.md`'s
-- Ownership section. A row of all-NULLs is not written; `storage::write` drops it.
--
-- Included by both migration ledgers from this one file, for the reason given in
-- `0002_behaviour_timeline.sql`: two hand-kept copies of this schema drifting apart would be a
-- silent corruption rather than a compile error.

-- Attributes of a file used as popup content. `weight` is relative frequency, `scale` multiplies
-- the size the mode would otherwise have chosen, the four `region_*` columns are the part of the
-- monitor it may spawn in, `monitor` is which screen it prefers, `caption` is a caption pinned to
-- this file (as opposed to the tag-matched pool), and the two `video_*` columns are the
-- `VideoPopupOpts` fields that are properties of the clip rather than of the user.
--
-- Four columns rather than one `anchor TEXT`, which is what this started as. A region subsumes the
-- nine anchors -- the engine centres a window too big for its region and clamps it to the screen,
-- so a zero-size region names one placement exactly -- while also expressing "somewhere in the
-- left half", which an anchor cannot. Four nullable columns move together: all set, or all NULL.
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
