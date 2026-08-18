-- Notifications carry a title alongside their body (`TextItem::summary`). Only the
-- 'notification' kind uses it; the other pools leave it NULL.
ALTER TABLE behaviour_text_item ADD COLUMN summary TEXT;
