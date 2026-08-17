-- Explicit stage media, stage-owned prompt behaviour, and per-prompt deadlines.
ALTER TABLE behaviour_stage ADD COLUMN audio_random INTEGER NOT NULL DEFAULT 0
    CHECK (audio_random IN (0, 1));
ALTER TABLE behaviour_text_item ADD COLUMN timeout_seconds REAL;

-- Keep the original boolean columns for backwards-readable history changesets; new readers and
-- writers use the explicit values below.
ALTER TABLE behaviour_stage_entry ADD COLUMN splash_media INTEGER REFERENCES media (id) ON DELETE SET NULL;
ALTER TABLE behaviour_stage_entry ADD COLUMN sound_media INTEGER REFERENCES media (id) ON DELETE SET NULL;
ALTER TABLE behaviour_stage_entry ADD COLUMN notification_text TEXT;

CREATE TABLE IF NOT EXISTS behaviour_stage_prompt (
    stage_id TEXT PRIMARY KEY REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    timeouts_enabled INTEGER NOT NULL DEFAULT 1 CHECK (timeouts_enabled IN (0, 1)),
    popup_burst INTEGER,
    sound_media INTEGER REFERENCES media (id) ON DELETE SET NULL
) STRICT;

-- Preserve the previous pack-wide timeout by applying it to prompts which did not have an
-- individual value yet. The old mutually-exclusive punishment cannot be migrated losslessly to
-- the new composable, stage-owned model and therefore remains only as legacy storage.
UPDATE behaviour_text_item
SET timeout_seconds = (SELECT prompt_timeout_seconds FROM behaviour_settings WHERE singleton = 1)
WHERE kind = 'prompt' AND timeout_seconds IS NULL;
