-- Phase 6 adds no columns: sound schedules use behaviour_stage_event's open-ended kind column,
-- and crossfade uses behaviour_transition_category in the same way as every other category.
-- Keep a ledger entry so older engines reject packs whose behaviour vocabulary they cannot read.
SELECT 1;
