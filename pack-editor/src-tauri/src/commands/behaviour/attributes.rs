//! What a pack author says about *this file*, rather than about a tag, a stage, or the pack.
//!
//! Every one of these takes a list of ids, because the media inspector edits a selection: "set the
//! scale on these forty files". They are still per-field — each names the one attribute it changes,
//! and says nothing about the others.
//!
//! The rule underneath them all: **absent means "no opinion", never a zero.** A field the author
//! never set stays distinguishable from one they set to today's default, because defaults move
//! under the user across engine releases. An entry left saying nothing is dropped rather than
//! stored full of the values that happened to be current.

use shared::behaviour::editor;
use shared::behaviour::{MonitorPreference, SpawnRegion};

use super::setter;

setter!(
    /// How often these files are drawn relative to their neighbours.
    set_popup_weight(ids: Vec<u64>, weight: Option<f64>) |tx| {
        editor::set_popup_weight(tx, &ids, weight)
    }
);

setter!(
    /// Multiplies the size the mode would otherwise have chosen.
    set_popup_scale(ids: Vec<u64>, scale: Option<f64>) |tx| {
        editor::set_popup_scale(tx, &ids, scale)
    }
);

setter!(
    /// The part of the monitor these files may spawn in — one rectangle, not four edges.
    set_popup_region(ids: Vec<u64>, region: Option<SpawnRegion>) |tx| {
        editor::set_popup_region(tx, &ids, region)
    }
);

setter!(
    set_popup_monitor(ids: Vec<u64>, monitor: Option<MonitorPreference>) |tx| {
        editor::set_popup_monitor(tx, &ids, monitor)
    }
);

setter!(
    /// A caption pinned to these files, as opposed to the tag-matched pool.
    set_popup_caption(ids: Vec<u64>, caption: Option<String>) |tx| {
        editor::set_popup_caption(tx, &ids, caption.as_deref())
    }
);

setter!(
    set_popup_video_loop(ids: Vec<u64>, value: Option<bool>) |tx| {
        editor::set_popup_video_loop(tx, &ids, value)
    }
);

setter!(
    set_popup_video_audio(ids: Vec<u64>, value: Option<bool>) |tx| {
        editor::set_popup_video_audio(tx, &ids, value)
    }
);

setter!(
    /// These tracks' own level, for levelling a pack assembled from mixed sources.
    set_audio_volume(ids: Vec<u64>, volume: Option<f64>) |tx| {
        editor::set_audio_volume(tx, &ids, volume)
    }
);
