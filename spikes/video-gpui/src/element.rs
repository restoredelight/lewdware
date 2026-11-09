//! A custom gpui element for one video frame: letterboxed into its bounds, optionally over a
//! checkerboard, and responsible for freeing the frames it no longer shows.
//!
//! gpui's public API offers exactly one way to draw pixels you produced yourself on Linux:
//! `Window::paint_image`, which puts an `Arc<RenderImage>` into the window's sprite atlas keyed by
//! its `ImageId` and draws that tile. There is no way to update a tile in place or to hand gpui a
//! texture (`surface()` is macOS-only and the wgpu renderer ignores surfaces). So "reuse one
//! texture" is not possible from outside gpui; what this element can do is:
//!
//! 1. remove the previous frame's tile as soon as the new one is painted (`Window::drop_image`),
//!    so the atlas never holds more than two frames, and
//! 2. report the size the frame is displayed at, in device pixels, so the decoder can scale to it
//!    (`--scale ffmpeg`) and upload no more pixels than are shown.

use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gpui_kit::{
    App, Bounds, Corners, DevicePixels, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, ObjectFit, Pixels, RenderImage, Size, Style, Window,
    fill, hsla, point, px, relative, size,
};

pub struct VideoElement {
    pub image: Option<Arc<RenderImage>>,
    /// The video's display size, for the aspect ratio (the image itself may be downscaled).
    pub native: (u32, u32),
    pub checkerboard: bool,
    /// Frames shown before `image`: removed from the atlas once `image` is painted.
    pub retire: Vec<Arc<RenderImage>>,
    /// Receives the fitted size in device pixels, `w << 32 | h`.
    pub target_size: Arc<AtomicU64>,
    pub paint_time: Rc<Cell<(Duration, u32)>>,
}

impl IntoElement for VideoElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for VideoElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        style.min_size = size(px(0.).into(), px(0.).into());
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        let started = Instant::now();
        let fitted = fit(bounds, self.native);

        let scale = window.scale_factor();
        let w = (f32::from(fitted.size.width) * scale).round() as u64;
        let h = (f32::from(fitted.size.height) * scale).round() as u64;
        self.target_size.store(w << 32 | h, Ordering::Relaxed);

        if self.checkerboard {
            paint_checkerboard(window, fitted);
        }
        if let Some(image) = self.image.clone() {
            if let Err(err) =
                window.paint_image(fitted, fitted, Corners::default(), image, 0, false)
            {
                tracing::error!("paint_image: {err:#}");
            }
        }
        for old in self.retire.drain(..) {
            let _ = window.drop_image(old);
        }

        let (total, count) = self.paint_time.get();
        self.paint_time.set((total + started.elapsed(), count + 1));
    }
}

/// `ObjectFit::Contain` of a `native`-sized picture in `bounds`.
pub fn fit(bounds: Bounds<Pixels>, native: (u32, u32)) -> Bounds<Pixels> {
    if native.0 == 0 || native.1 == 0 {
        return bounds;
    }
    ObjectFit::Contain.get_bounds(
        bounds,
        Size {
            width: DevicePixels(native.0 as i32),
            height: DevicePixels(native.1 as i32),
        },
    )
}

const CELL: f32 = 12.;

pub fn paint_checkerboard(window: &mut Window, area: Bounds<Pixels>) {
    let light: Hsla = hsla(0., 0., 0.85, 1.);
    let dark: Hsla = hsla(0., 0., 0.65, 1.);
    window.paint_quad(fill(area, light));
    window.with_content_mask(Some(gpui_kit::ContentMask { bounds: area }), |window| {
        let cols = (f32::from(area.size.width) / CELL).ceil() as i32;
        let rows = (f32::from(area.size.height) / CELL).ceil() as i32;
        for row in 0..rows {
            for col in (row % 2..cols).step_by(2) {
                let origin = point(
                    area.origin.x + px(col as f32 * CELL),
                    area.origin.y + px(row as f32 * CELL),
                );
                window.paint_quad(fill(Bounds::new(origin, size(px(CELL), px(CELL))), dark));
            }
        }
    });
}
