-- A clip's own soundtrack level (`PopupMedia::video_volume`).
--
-- Its own file, and its own migration in both ledgers, rather than a column appended to
-- `behaviour_schema.sql`: that file is the *first* migration in each ledger, so editing it in
-- place would give the column to freshly created databases only, and every pack already written
-- would keep an eleven-column table that the twelve-column read (and the positional copy the
-- editor imports through) cannot use. Same reasoning as `behaviour_schema.sql`'s own header for
-- why one file feeds both ledgers rather than two hand-kept copies.
ALTER TABLE behaviour_popup_media ADD COLUMN video_volume REAL;
