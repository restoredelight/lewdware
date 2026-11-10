//! `Session` methods for what a file does *as a popup* (frequency, size, spawn region, monitor,
//! caption, video looping, sound and level), and the geometry the popup editor draws with. Ported
//! from the Tauri editor's `commands/behaviour/attributes.rs`, `popupSize.ts` and `spawnRegion.ts`.
//!
//! The rule under every setter: **absent means "no opinion", never a default value.** A field the
//! author never set stays distinguishable from one set to today's default, because defaults move
//! under the user across engine releases. So "equal" frequency, a scale of 1, the whole screen,
//! "any" monitor, a blank caption, looping, sound on and full volume are all stored as `None`.

use shared::behaviour::editor as behaviour_editor;
use shared::behaviour::{MonitorPreference, PopupMedia, SpawnRegion};

use super::*;

impl Session {
    /// The popup attributes of `ids`. Files with nothing set are left out.
    pub fn popup_attributes(&self, ids: &[u64]) -> Result<Vec<(u64, PopupMedia)>> {
        let ids = ids.to_vec();
        self.read(move |c| behaviour_editor::popup_attributes(c, &ids))
    }

    /// How often these files are drawn relative to the rest. `None` or a non-positive weight is
    /// "equal".
    pub fn set_popup_weight(
        &mut self,
        ids: &[u64],
        weight: Option<f64>,
        label: &str,
    ) -> Result<Applied<()>> {
        let weight = weight.filter(|weight| *weight > 0.0 && weight.is_finite());
        self.popup_edit(ids, label, move |tx, ids| {
            behaviour_editor::set_popup_weight(tx, ids, weight)
        })
    }

    /// Multiplies the size the mode would otherwise choose. 1 (or anything not positive) is
    /// stored as `None`.
    pub fn set_popup_scale(
        &mut self,
        ids: &[u64],
        scale: Option<f64>,
        label: &str,
    ) -> Result<Applied<()>> {
        let scale = scale.filter(|scale| {
            *scale > 0.0 && scale.is_finite() && (*scale - 1.0).abs() > f64::EPSILON
        });
        self.popup_edit(ids, label, move |tx, ids| {
            behaviour_editor::set_popup_scale(tx, ids, scale)
        })
    }

    /// The part of the screen these files spawn in. The whole screen is stored as `None`.
    pub fn set_popup_region(
        &mut self,
        ids: &[u64],
        region: Option<SpawnRegion>,
        label: &str,
    ) -> Result<Applied<()>> {
        let region = region
            .map(SpawnRegion::sanitized)
            .filter(|region| !region.is_full());
        self.popup_edit(ids, label, move |tx, ids| {
            behaviour_editor::set_popup_region(tx, ids, region)
        })
    }

    /// `Any` is what saying nothing means, so it's stored as `None`.
    pub fn set_popup_monitor(
        &mut self,
        ids: &[u64],
        monitor: Option<MonitorPreference>,
        label: &str,
    ) -> Result<Applied<()>> {
        let monitor = monitor.filter(|monitor| *monitor != MonitorPreference::Any);
        self.popup_edit(ids, label, move |tx, ids| {
            behaviour_editor::set_popup_monitor(tx, ids, monitor)
        })
    }

    /// A caption pinned to these files. Blank clears it, falling back to the caption pool.
    pub fn set_popup_caption(
        &mut self,
        ids: &[u64],
        caption: Option<&str>,
        label: &str,
    ) -> Result<Applied<()>> {
        let caption = caption
            .map(str::trim)
            .filter(|caption| !caption.is_empty())
            .map(str::to_string);
        self.popup_edit(ids, label, move |tx, ids| {
            behaviour_editor::set_popup_caption(tx, ids, caption.as_deref())
        })
    }

    /// `false` closes the popup when the clip ends. Looping is the default, stored as `None`.
    pub fn set_popup_video_loop(
        &mut self,
        ids: &[u64],
        looping: bool,
        label: &str,
    ) -> Result<Applied<()>> {
        let value = (!looping).then_some(false);
        self.popup_edit(ids, label, move |tx, ids| {
            behaviour_editor::set_popup_video_loop(tx, ids, value)
        })
    }

    /// `false` silences the clip. Sound on is the default, stored as `None`.
    pub fn set_popup_video_audio(
        &mut self,
        ids: &[u64],
        audio: bool,
        label: &str,
    ) -> Result<Applied<()>> {
        let value = (!audio).then_some(false);
        self.popup_edit(ids, label, move |tx, ids| {
            behaviour_editor::set_popup_video_audio(tx, ids, value)
        })
    }

    /// The clip's own level. Full volume is stored as `None`.
    pub fn set_popup_video_volume(
        &mut self,
        ids: &[u64],
        volume: Option<f64>,
        label: &str,
    ) -> Result<Applied<()>> {
        let volume = volume
            .map(|volume| volume.clamp(0.0, 1.0))
            .filter(|volume| (*volume - 1.0).abs() > f64::EPSILON);
        self.popup_edit(ids, label, move |tx, ids| {
            behaviour_editor::set_popup_video_volume(tx, ids, volume)
        })
    }

    fn popup_edit(
        &mut self,
        ids: &[u64],
        label: &str,
        write: impl FnOnce(&rusqlite::Transaction<'_>, &[u64]) -> Result<()> + Send + 'static,
    ) -> Result<Applied<()>> {
        let ids = ids.to_vec();
        self.edit(BehaviourAction::new(label, move |tx| write(tx, &ids)))
            .map(|applied| Applied {
                value: (),
                changes: applied.changes,
                history: applied.history,
            })
    }
}

// ── Popup size ─────────────────────────────────────────────────────────────────────────────

/// The screen the editor reasons against, since the author's isn't the player's. Fixed, so the
/// number an author sees is the one a collaborator sees for the same pack.
pub const REFERENCE_SCREEN: (f64, f64) = (1920.0, 1080.0);

/// See `MIN_POPUP_CAP_WIDTH` in the engine.
const MIN_CAP: (f64, f64) = (300.0, 200.0);

/// The largest an engine-chosen popup may be on a screen of this size. A mirror of the engine's
/// `popup_size_caps` (`lewdware/src/utils.rs`); only ever shown, never authoritative.
pub fn popup_size_caps(screen: (f64, f64)) -> (f64, f64) {
    (
        (screen.0 / 3.0).max(MIN_CAP.0.min(screen.0)),
        (screen.1 / 2.0).max(MIN_CAP.1.min(screen.1)),
    )
}

fn factor(scale: Option<f64>) -> f64 {
    scale.filter(|scale| *scale > 0.0).unwrap_or(1.0)
}

/// The size a popup of `media` will be at `scale`, in logical pixels on `screen`. A mirror of the
/// engine's `calculate_media_popup_size`: the scale applies before the caps, so the caps still
/// get the last word.
pub fn popup_size(media: (f64, f64), scale: Option<f64>, screen: (f64, f64)) -> (f64, f64) {
    let factor = factor(scale);
    let (width, height) = (media.0 * factor, media.1 * factor);
    if width <= 0.0 || height <= 0.0 {
        return (0.0, 0.0);
    }
    let caps = popup_size_caps(screen);
    let shrink = (caps.0 / width).min(caps.1 / height).min(1.0);
    ((width * shrink).round(), (height * shrink).round())
}

/// Whether the caps, rather than the scale, decide this popup's size: past it, turning the scale
/// up changes nothing.
pub fn is_capped(media: (f64, f64), scale: Option<f64>, screen: (f64, f64)) -> bool {
    let factor = factor(scale);
    let caps = popup_size_caps(screen);
    media.0 * factor > caps.0 || media.1 * factor > caps.1
}

/// The scale that makes a popup `target_width` wide, rounded to a thousandth. `None` when that's
/// the media's own width ("no opinion").
pub fn scale_for_width(media_width: f64, target_width: f64) -> Option<f64> {
    if target_width <= 0.0 || media_width <= 0.0 {
        return None;
    }
    let scale = ((target_width / media_width) * 1000.0).round() / 1000.0;
    (scale != 1.0).then_some(scale)
}

// ── Spawn region ───────────────────────────────────────────────────────────────────────────

/// The nine placements, in reading order. Each is a zero-size region at a point.
pub const POINTS: [[&str; 3]; 3] = [
    ["top left", "top centre", "top right"],
    ["centre left", "centre", "centre right"],
    ["bottom left", "bottom centre", "bottom right"],
];

/// Area presets, for the shapes an author actually asks for.
pub const AREAS: [(&str, SpawnRegion); 5] = [
    ("Left half", region(0.0, 0.0, 0.5, 1.0)),
    ("Right half", region(0.5, 0.0, 0.5, 1.0)),
    ("Top half", region(0.0, 0.0, 1.0, 0.5)),
    ("Bottom half", region(0.0, 0.5, 1.0, 0.5)),
    ("Middle", region(0.25, 0.25, 0.5, 0.5)),
];

const fn region(x: f64, y: f64, width: f64, height: f64) -> SpawnRegion {
    SpawnRegion {
        x,
        y,
        width,
        height,
    }
}

/// Edges a dragged edge sticks to within [`SNAP`]: halves, thirds and quarters.
const SNAP_LINES: [f64; 7] = [0.0, 0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75, 1.0];
const SNAP: f64 = 0.015;

pub fn snap(value: f64) -> f64 {
    SNAP_LINES
        .into_iter()
        .find(|line| (line - value).abs() <= SNAP)
        .unwrap_or(value)
}

/// A region from its four edges, tolerating a drag that crossed over itself.
pub fn from_edges(left: f64, top: f64, right: f64, bottom: f64) -> SpawnRegion {
    region(
        left.min(right),
        top.min(bottom),
        (right - left).abs(),
        (bottom - top).abs(),
    )
    .sanitized()
}

/// The zero-size region at the point in row `row`, column `column` of [`POINTS`].
pub fn point_region(row: usize, column: usize) -> SpawnRegion {
    region(column as f64 / 2.0, row as f64 / 2.0, 0.0, 0.0)
}

/// Which of [`POINTS`] a region pins to, as (row, column), or `None` for an area.
pub fn region_point(region: Option<SpawnRegion>) -> Option<(usize, usize)> {
    let region = region?.sanitized();
    if region.width != 0.0 || region.height != 0.0 {
        return None;
    }
    let index = |value: f64| [0.0, 0.5, 1.0].iter().position(|at| *at == value);
    Some((index(region.y)?, index(region.x)?))
}

pub fn is_full(region: Option<SpawnRegion>) -> bool {
    region.is_none_or(|region| region.is_full())
}

/// How a region reads in the editor: "anywhere on the screen", "pinned top left", "left half", or
/// its size and position in percent.
pub fn describe_region(region: Option<SpawnRegion>) -> String {
    if is_full(region) {
        return "anywhere on the screen".into();
    }
    if let Some((row, column)) = region_point(region) {
        return format!("pinned {}", POINTS[row][column]);
    }
    let region = region.expect("not full").sanitized();
    if let Some((label, _)) = AREAS.iter().find(|(_, area)| *area == region) {
        return label.to_lowercase();
    }
    let percent = |value: f64| (value * 100.0).round();
    format!(
        "{}% x {}% area at {}%, {}%",
        percent(region.width),
        percent(region.height),
        percent(region.x),
        percent(region.y)
    )
}

/// Where a popup of `size` lands on one axis, given the region span `[start, start + span]` on a
/// screen of `total`. The engine picks at random when the popup fits; this is the centre draw.
/// Too big for the span, it centres on it, and either way it's pulled back onto the screen.
pub fn place_in_region(start: f64, span: f64, size: f64, total: f64) -> f64 {
    let centred = start + (span - size) / 2.0;
    centred.min(total - size).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_media_keeps_its_size_and_large_media_hits_the_caps() {
        assert_eq!(
            popup_size((400.0, 300.0), None, REFERENCE_SCREEN),
            (400.0, 300.0)
        );
        assert!(!is_capped((400.0, 300.0), None, REFERENCE_SCREEN));
        // Caps on 1920x1080 are 640x540.
        assert_eq!(
            popup_size((1280.0, 720.0), None, REFERENCE_SCREEN),
            (640.0, 360.0)
        );
        assert!(is_capped((1280.0, 720.0), None, REFERENCE_SCREEN));
        assert_eq!(
            popup_size((400.0, 300.0), Some(1.5), REFERENCE_SCREEN),
            (600.0, 450.0)
        );
    }

    #[test]
    fn scale_for_width_is_none_at_the_media_width() {
        assert_eq!(scale_for_width(400.0, 400.0), None);
        assert_eq!(scale_for_width(400.0, 600.0), Some(1.5));
        assert_eq!(scale_for_width(400.0, 0.0), None);
    }

    #[test]
    fn points_round_trip_and_describe_themselves() {
        for row in 0..3 {
            for column in 0..3 {
                let at = point_region(row, column);
                assert_eq!(region_point(Some(at)), Some((row, column)));
            }
        }
        assert_eq!(
            describe_region(Some(point_region(2, 2))),
            "pinned bottom right"
        );
        assert_eq!(describe_region(None), "anywhere on the screen");
        assert_eq!(
            describe_region(Some(SpawnRegion::FULL)),
            "anywhere on the screen"
        );
        assert_eq!(describe_region(Some(AREAS[0].1)), "left half");
        assert_eq!(
            describe_region(Some(region(0.1, 0.2, 0.3, 0.4))),
            "30% x 40% area at 10%, 20%"
        );
    }

    #[test]
    fn edges_snap_and_crossed_drags_normalise() {
        assert_eq!(snap(0.49), 0.5);
        assert_eq!(snap(0.4), 0.4);
        assert_eq!(from_edges(0.5, 0.5, 0.0, 0.0), region(0.0, 0.0, 0.5, 0.5));
    }

    #[test]
    fn a_popup_too_big_for_its_region_stays_on_screen() {
        // Pinned to the right edge: centred on x = 1000 it would overflow, so it's clamped.
        assert_eq!(place_in_region(1000.0, 0.0, 200.0, 1000.0), 800.0);
        assert_eq!(place_in_region(0.0, 0.0, 200.0, 1000.0), 0.0);
        assert_eq!(place_in_region(0.0, 1000.0, 200.0, 1000.0), 400.0);
    }
}
