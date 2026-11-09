//! `Session` methods for the timeline, stages and transitions. One file per track of the gpui port, so tracks don't edit the
//! same file; see `design/editor-model.md`.
//!
//! Each method is one line over [`Session::write`](super::Session) and a function in
//! `crate::pack::ops::timeline`, which documents what each does.

use super::*;

use shared::behaviour::{
    Easing, EndStrategy, EventKind, EventSchedule, MediaSlot, TransitionCategory,
};

use crate::pack::ops::timeline::{self as op, MembershipOutcome, StageAudioMode};

impl Session {
    // ── The timeline ────────────────────────────────────────────────────────────────────────

    pub fn set_timeline_enabled(&mut self, enabled: bool) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::set_timeline_enabled(c, enabled))
    }

    pub fn set_timeline_label(&mut self, label: Option<&str>) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::set_timeline_label(c, label))
    }

    // ── Stages as a list ────────────────────────────────────────────────────────────────────

    pub fn add_stage(
        &mut self,
        after: Option<&str>,
        source: Option<&str>,
        name: Option<&str>,
    ) -> Result<Applied<String>> {
        self.write(WriteKind::Edit, |c| op::add_stage(c, after, source, name))
    }

    pub fn duplicate_stage(&mut self, id: &str) -> Result<Applied<String>> {
        self.write(WriteKind::Edit, |c| op::duplicate_stage(c, id))
    }

    pub fn move_stage(&mut self, id: &str, to: usize) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::move_stage(c, id, to))
    }

    pub fn remove_stage(&mut self, id: &str, also_remove_tag: bool) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::remove_stage(c, id, also_remove_tag)
        })
    }

    // ── One stage ───────────────────────────────────────────────────────────────────────────

    pub fn set_stage_label(&mut self, id: &str, label: &str) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::set_stage_label(c, id, label))
    }

    pub fn set_stage_restricts_content(&mut self, id: &str, on: bool) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_restricts_content(c, id, on)
        })
    }

    pub fn set_stage_tag(
        &mut self,
        id: &str,
        tag: &str,
        excluded: bool,
        present: bool,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_tag(c, id, tag, excluded, present)
        })
    }

    pub fn set_stage_audio_mode(&mut self, id: &str, mode: StageAudioMode) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::set_stage_audio_mode(c, id, mode))
    }

    pub fn set_stage_event(
        &mut self,
        id: &str,
        kind: EventKind,
        schedule: Option<EventSchedule>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_event(c, id, kind, schedule)
        })
    }

    pub fn set_stage_entry_notification(
        &mut self,
        id: &str,
        text: Option<&str>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_entry_notification(c, id, text)
        })
    }

    pub fn set_stage_entry_popup_burst(
        &mut self,
        id: &str,
        count: Option<u32>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_entry_popup_burst(c, id, count)
        })
    }

    pub fn set_stage_prompt_timeouts_enabled(
        &mut self,
        id: &str,
        enabled: bool,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_prompt_timeouts_enabled(c, id, enabled)
        })
    }

    pub fn set_stage_prompt_timeout_multiplier(
        &mut self,
        id: &str,
        multiplier: f64,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_prompt_timeout_multiplier(c, id, multiplier)
        })
    }

    pub fn set_stage_prompt_popup_burst(
        &mut self,
        id: &str,
        count: Option<u32>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_prompt_popup_burst(c, id, count)
        })
    }

    pub fn set_stage_movement(&mut self, id: &str, on: bool) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::set_stage_movement(c, id, on))
    }

    pub fn set_stage_movement_speed(
        &mut self,
        id: &str,
        minimum: Option<f64>,
        maximum: Option<f64>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_movement_speed(c, id, minimum, maximum)
        })
    }

    pub fn set_stage_mitosis(&mut self, id: &str, on: bool) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::set_stage_mitosis(c, id, on))
    }

    pub fn set_stage_mitosis_values(
        &mut self,
        id: &str,
        chance: Option<f64>,
        count: Option<u32>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_mitosis_values(c, id, chance, count)
        })
    }

    pub fn set_stage_end_duration(
        &mut self,
        id: &str,
        seconds: Option<f64>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_end_duration(c, id, seconds)
        })
    }

    pub fn set_stage_end_event(
        &mut self,
        id: &str,
        event: Option<EventKind>,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::set_stage_end_event(c, id, event))
    }

    pub fn set_stage_end_event_count(&mut self, id: &str, count: u32) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_end_event_count(c, id, count)
        })
    }

    pub fn set_stage_end_strategy(
        &mut self,
        id: &str,
        strategy: EndStrategy,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_end_strategy(c, id, strategy)
        })
    }

    pub fn set_stage_slot(&mut self, slot: &MediaSlot, media: u64) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::set_stage_slot(c, slot, media))
    }

    pub fn clear_stage_slot(&mut self, slot: &MediaSlot) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| op::clear_stage_slot(c, slot))
    }

    pub fn set_stage_membership(
        &mut self,
        media: u64,
        stage: &str,
        member: bool,
        approved: &[String],
    ) -> Result<Applied<MembershipOutcome>> {
        self.write(WriteKind::Edit, |c| {
            op::set_stage_membership(c, media, stage, member, approved)
        })
    }

    // ── Transitions ─────────────────────────────────────────────────────────────────────────

    pub fn set_transition_duration(&mut self, id: &str, seconds: f64) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_transition_duration(c, id, seconds)
        })
    }

    pub fn set_transition_easing(&mut self, id: &str, easing: Easing) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_transition_easing(c, id, easing)
        })
    }

    pub fn set_transition_category(
        &mut self,
        id: &str,
        category: TransitionCategory,
        enabled: bool,
    ) -> Result<Applied<()>> {
        self.write(WriteKind::Edit, |c| {
            op::set_transition_category(c, id, category, enabled)
        })
    }
}
