use winit::dpi::PhysicalSize;

use super::*;
use crate::window::decorations::Decorations;
use crate::window::redraw::RedrawRequester;
use crate::window::theme::{Appearance, Metrics};

/// `plain`'s metrics plus stand-ins for themes yet to be written, so the agreement below is
/// a property of the seam rather than of one theme's numbers.
const METRICS: &[Metrics] = &[
    Metrics::PLAIN,
    Metrics {
        header_height: 18,
        border_width: 4,
    },
    Metrics {
        header_height: 37,
        border_width: 1,
    },
];

fn content_origin(metrics: Metrics, scale_factor: f64) -> (u32, u32) {
    Decorations::new(
        true,
        metrics,
        Theme::Plain.chrome(Appearance::Light),
        PhysicalSize::new(200, 150),
        scale_factor,
        None,
        true,
        RedrawRequester::detached(),
    )
    .content_origin()
}

/// The regression this seam exists to prevent: egui's coordinate origin has to be the exact
/// pixel the content is drawn at. A mismatch does not look broken — it silently sends clicks
/// to the wrong widget — and the old hardcoded translation drifted from `content_origin()`
/// at every fractional scale factor.
#[test]
fn a_pointer_at_the_content_origin_lands_on_eguis_own_origin() {
    for &metrics in METRICS {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
            let origin = content_origin(metrics, scale);
            let at_origin = PhysicalPosition::new(origin.0 as f64, origin.1 as f64);

            assert_eq!(
                translate_position(at_origin, origin),
                PhysicalPosition::new(0.0, 0.0),
                "{metrics:?} at {scale}x"
            );
        }
    }
}

/// Offsets inside the content area survive the rebase unchanged, so a click 10px into the
/// content is a click 10px into egui.
#[test]
fn offsets_within_the_content_area_are_preserved() {
    for &metrics in METRICS {
        for scale in [1.0, 1.5, 2.0] {
            let origin = content_origin(metrics, scale);
            let position = PhysicalPosition::new(origin.0 as f64 + 10.0, origin.1 as f64 + 20.0);

            assert_eq!(
                translate_position(position, origin),
                PhysicalPosition::new(10.0, 20.0),
                "{metrics:?} at {scale}x"
            );
        }
    }
}

/// A pointer over the decorations themselves rebases to negative coordinates rather than
/// wrapping into the content area — egui must see it as outside, not as a click near (0, 0).
#[test]
fn a_pointer_over_the_header_rebases_outside_the_content() {
    for &metrics in METRICS {
        let origin = content_origin(metrics, 1.0);
        let in_header = PhysicalPosition::new(origin.0 as f64, 0.0);

        let translated = translate_position(in_header, origin);
        assert!(translated.y < 0.0, "{metrics:?} header y={}", translated.y);
    }
}

/// Only pointer events are rebased; everything else passes through untouched.
#[test]
fn non_pointer_events_pass_through() {
    let event = WindowEvent::Focused(true);
    assert!(matches!(
        translate_event_position(event, (1, 25)),
        WindowEvent::Focused(true)
    ));
}
