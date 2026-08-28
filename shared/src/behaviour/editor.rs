//! Fine-grained editing of a stored behaviour document.
//!
//! [`Behaviour`](super::Behaviour) reads and writes complete documents for consumers such as the
//! engine and converter. The editor instead addresses rows by stable primary key and changes only
//! the field or collection item named by an author action. Setters report whether they changed
//! anything so the caller can avoid recording empty undo entries.
//!
//! The implementation is divided by editor domain. Public items are re-exported here to preserve
//! the original `behaviour::editor::*` API.

mod common;
mod group;
mod link;
mod media;
mod membership;
mod pool;
mod stage;
mod stage_tags;
mod timeline;
mod transition;

pub use group::{
    add_content_group, add_content_group_tag, content_groups, remove_content_group,
    remove_content_group_tag, set_content_group_description, set_content_group_enabled_by_default,
    set_content_group_label,
};
pub use link::{
    WebLinkRow, add_web_link, add_web_link_tag, remove_web_link, remove_web_link_tag,
    set_web_link_url, web_links,
};
pub use media::{
    MediaSlots, audio_attributes, clear_media_slots, is_media_referenced, media_slots,
    popup_attributes, set_audio_volume, set_media_slot, set_popup_audio, set_popup_caption,
    set_popup_monitor, set_popup_region, set_popup_scale, set_popup_video_audio,
    set_popup_video_loop, set_popup_video_volume, set_popup_weight, slot_value,
};
pub use membership::{
    MembershipPlan, StageTagCreation, apply_membership_creation, media_tags, plan_stage_membership,
    selects, stage_restricts_content,
};
pub use pool::{
    PoolKind, TextItemRow, add_text_item, add_text_item_tag, remove_text_item,
    remove_text_item_tag, set_text_item_summary, set_text_item_text, set_text_item_timeout,
    text_pool,
};
pub use stage::{
    add_stage_exclude_tag, add_stage_tag, remove_stage_exclude_tag, remove_stage_tag,
    set_stage_audio_random, set_stage_content_tags, set_stage_end_duration,
    set_stage_end_event_count, set_stage_end_strategy, set_stage_entry_notification,
    set_stage_entry_popup_burst, set_stage_event, set_stage_exclude_tags, set_stage_label,
    set_stage_mitosis, set_stage_mitosis_values, set_stage_movement, set_stage_movement_speed,
    set_stage_prompt_popup_burst, set_stage_prompt_timeout_multiplier,
    set_stage_prompt_timeouts_enabled,
};
pub use stage_tags::{
    OwnedStageTag, read_one_stage_label, slugify_stage_label, stage_audio, stage_exclude_tag_name,
    stage_owned_tags, stage_scenery, stage_tag_name, stage_tag_shared,
};
pub use timeline::{
    TimelineView, add_stage, duplicate_stage, move_stage, remove_stage, set_timeline_enabled,
    set_timeline_label, stages, timeline, transitions,
};
pub use transition::{
    set_transition_affected, set_transition_category, set_transition_duration,
    set_transition_easing,
};

#[cfg(test)]
mod tests;
