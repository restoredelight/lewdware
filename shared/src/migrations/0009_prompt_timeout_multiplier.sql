-- Each mode/stage can scale the prompt's explicit or automatically derived deadline.
ALTER TABLE behaviour_stage_prompt ADD COLUMN timeout_multiplier REAL NOT NULL DEFAULT 1;
