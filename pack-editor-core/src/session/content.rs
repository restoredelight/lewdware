//! `Session` methods for captions, prompts, notifications, web links, content groups and the
//! pack's media slots. One file per track of the gpui port, so tracks don't edit the same file;
//! see `design/editor-model.md`.
//!
//! Each method is one line over [`Session::write`](super::Session) and a function in
//! `crate::pack::ops::content`.

use super::*;

use shared::behaviour::MediaSlot;

use crate::pack::ops::content::{self as content_ops, EntryKind};

impl Session {
    // ── Entries ─────────────────────────────────────────────────────────────────────────────

    /// Adds an empty entry at the end of its pool; the value is its id.
    pub fn add_entry(&mut self, kind: EntryKind) -> Result<Applied<i64>> {
        self.write(WriteKind::Edit, |c| content_ops::add_entry(c, kind))
    }

    pub fn remove_entry(&mut self, kind: EntryKind, id: i64) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| content_ops::remove_entry(c, kind, id))
    }

    pub fn set_entry_text(&mut self, kind: EntryKind, id: i64, text: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::set_entry_text(c, kind, id, text)
        })
    }

    pub fn set_notification_summary(
        &mut self,
        id: i64,
        summary: Option<&str>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::set_notification_summary(c, id, summary)
        })
    }

    pub fn set_prompt_timeout(&mut self, id: i64, seconds: Option<f64>) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::set_prompt_timeout(c, id, seconds)
        })
    }

    pub fn add_entry_tag(&mut self, kind: EntryKind, id: i64, tag: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::add_entry_tag(c, kind, id, tag)
        })
    }

    pub fn remove_entry_tag(&mut self, kind: EntryKind, id: i64, tag: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::remove_entry_tag(c, kind, id, tag)
        })
    }

    // ── Content groups ──────────────────────────────────────────────────────────────────────

    /// Adds an unnamed group at the end; the value is its key.
    pub fn add_content_group(&mut self) -> Result<Applied<String>> {
        self.write(WriteKind::Edit, content_ops::add_content_group)
    }

    /// Makes `tag` a group of its own; the value is the new group's key.
    pub fn add_content_group_from_tag(&mut self, tag: &str) -> Result<Applied<String>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::add_content_group_from_tag(c, tag)
        })
    }

    pub fn remove_content_group(&mut self, key: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::remove_content_group(c, key)
        })
    }

    pub fn set_content_group_label(&mut self, key: &str, label: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::set_content_group_label(c, key, label)
        })
    }

    pub fn set_content_group_description(
        &mut self,
        key: &str,
        description: Option<&str>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::set_content_group_description(c, key, description)
        })
    }

    pub fn set_content_group_enabled_by_default(
        &mut self,
        key: &str,
        enabled: bool,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::set_content_group_enabled_by_default(c, key, enabled)
        })
    }

    pub fn add_content_group_tag(&mut self, key: &str, tag: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::add_content_group_tag(c, key, tag)
        })
    }

    pub fn remove_content_group_tag(&mut self, key: &str, tag: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::remove_content_group_tag(c, key, tag)
        })
    }

    // ── Media slots ─────────────────────────────────────────────────────────────────────────

    /// Points `slot` at a file already in the pack; the value is the file that left with the
    /// file it replaced, if it did.
    pub fn set_media_slot(
        &mut self,
        slot: &MediaSlot,
        media_id: u64,
    ) -> Result<Applied<Option<u64>>> {
        self.write(WriteKind::Edit, |c| {
            content_ops::set_media_slot(c, slot, media_id)
        })
    }

    /// Empties `slot`; the value is the file that left with it, if it did.
    pub fn clear_media_slot(&mut self, slot: &MediaSlot) -> Result<Applied<Option<u64>>> {
        self.write(WriteKind::Edit, |c| content_ops::clear_media_slot(c, slot))
    }

    /// Catches the session up after `encode::process_file_into_slot` imported a file into a slot
    /// on its own pooled connection: re-reads the files it touched (`files`: the one it added or
    /// reused, and the one it displaced, if any) and reports the slot, the files and history as
    /// changed. The pipeline's own writes never pass through [`Self::write`], so this is how they
    /// reach the index and the views.
    pub fn slot_filled_by_import(&mut self, files: &[u64]) -> Result<Applied<()>> {
        let fresh = pack::read_files(&self.connection, Some(files))?;
        let media = self.media.apply(files, fresh);
        self.history = history::status(&self.connection)?;
        // Stage slots are the timeline's.
        let mut scopes = Scope::Slots | Scope::Timeline | Scope::Tags;
        if !media.is_empty() {
            scopes |= Scope::Media;
        }
        Ok(Applied {
            value: (),
            changes: Changes { scopes, media },
            history: self.history.clone(),
        })
    }
}
