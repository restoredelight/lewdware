-- Indexes for lookups every edit makes, so their cost doesn't grow with the pack.
--
-- `history::collect_garbage` runs twice per recorded edit and looks for soft-deleted media.
-- Without an index that was a scan of the whole media table each time: about 4ms per edit on a
-- 10,000-file pack, against 0.5ms with it.
CREATE INDEX IF NOT EXISTS media_deleted_index ON media(id) WHERE deleted = TRUE;

-- The join tables' primary keys lead with media_id, so "which files have this tag" (renaming,
-- merging and deleting tags, stage content) scanned every link.
CREATE INDEX IF NOT EXISTS media_tags_tag_index ON media_tags(tag_id);
CREATE INDEX IF NOT EXISTS media_artists_artist_index ON media_artists(artist_id);
