//! Internal mapping between behaviour domain types and their database rows.
//!
//! [`super::schema::Behaviour`] assembles and writes whole documents, while [`super::editor`]
//! reads and changes the rows involved in individual author actions.
//! Keeping the mapping here prevents either facade from becoming the other's implementation.

mod common;
mod content;
mod experience;
mod media;
mod stage;
mod tags;
mod transition;

pub(crate) use content::{
    read_content_groups, read_text_pool, read_web_links, write_content_groups, write_text_pool,
    write_web_links,
};
pub(crate) use experience::{read_experience, read_timeline, write_experience};
pub(crate) use media::{
    read_audio_media, read_one_popup_media, read_popup_media, write_audio_media,
    write_one_popup_media, write_popup_media,
};
#[allow(unused_imports)]
pub(crate) use stage::{
    read_one_stage, read_owned_stage_tag, read_stage_end, read_stage_entry,
    read_stage_events, read_stage_mitosis, read_stage_movement, read_stage_prompt, read_stage_tags,
    read_stages, write_one_stage, write_one_stage_event, write_owned_stage_tag, write_stage_end,
    write_stage_entry, write_stage_events, write_stage_mitosis, write_stage_movement,
    write_stage_prompt, write_stage_tags, write_stages,
};
pub(crate) use tags::{read_tags, tag_id, write_tags};
pub(crate) use transition::read_transitions;
