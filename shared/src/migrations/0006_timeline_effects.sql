-- Phase 5: stage audio, declarative entry effects, and prompt behaviour.
ALTER TABLE behaviour_stage ADD COLUMN audio INTEGER REFERENCES media (id) ON DELETE SET NULL;

CREATE TABLE IF NOT EXISTS behaviour_stage_entry (
    stage_id TEXT PRIMARY KEY REFERENCES behaviour_stage (id) ON DELETE CASCADE,
    splash INTEGER NOT NULL DEFAULT 0 CHECK (splash IN (0, 1)),
    sound INTEGER NOT NULL DEFAULT 0 CHECK (sound IN (0, 1)),
    popup_burst INTEGER,
    notification INTEGER NOT NULL DEFAULT 0 CHECK (notification IN (0, 1))
) STRICT;

ALTER TABLE behaviour_settings ADD COLUMN prompt_timeout_seconds REAL;
ALTER TABLE behaviour_settings ADD COLUMN prompt_wrong_answer_kind TEXT;
ALTER TABLE behaviour_settings ADD COLUMN prompt_wrong_answer_value REAL;
