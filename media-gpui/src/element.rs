use gpui_kit::{
    App, Bounds, Corners, DevicePixels, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, ObjectFit, Pixels, RenderImage, Size, Style, Window, relative,
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// Only images that actually reached paint enter this ledger. A discarded render cannot
/// strand retired atlas tiles. The owning view clears it on source changes and release.
#[derive(Default)]
pub(crate) struct Images {
    painted: Option<Arc<RenderImage>>,
    checker: Option<(u32, u32, u32, Arc<RenderImage>)>,
}

impl Images {
    pub fn clear(&mut self, window: &mut Window) {
        if let Some(image) = self.painted.take() {
            let _ = window.drop_image(image);
        }
        if let Some((_, _, _, image)) = self.checker.take() {
            let _ = window.drop_image(image);
        }
    }

    fn checker(&mut self, window: &mut Window, bounds: Bounds<Pixels>) {
        let scale = window.scale_factor();
        let w = (f32::from(bounds.size.width) * scale).round().max(1.) as u32;
        let h = (f32::from(bounds.size.height) * scale).round().max(1.) as u32;
        let cell = (12. * scale).round().max(1.) as u32;
        if self
            .checker
            .as_ref()
            .is_none_or(|(cw, ch, cc, _)| (*cw, *ch, *cc) != (w, h, cell))
        {
            if let Some((_, _, _, old)) = self.checker.take() {
                let _ = window.drop_image(old);
            }
            let bytes = checker_pixels(w, h, cell);
            let image = Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
                image::RgbaImage::from_raw(w, h, bytes).unwrap()
            )]));
            self.checker = Some((w, h, cell, image));
        }
        if let Some((_, _, _, image)) = &self.checker {
            let _ = window.paint_image(bounds, bounds, Corners::default(), image.clone(), 0, false);
        }
    }
}

fn checker_pixels(w: u32, h: u32, cell: u32) -> Vec<u8> {
    let mut bytes = vec![0; w as usize * h as usize * 4];
    for (index, pixel) in bytes.chunks_exact_mut(4).enumerate() {
        let (x, y) = (index as u32 % w, index as u32 / w);
        let value = if (x / cell + y / cell) % 2 == 0 {
            217
        } else {
            166
        };
        pixel.copy_from_slice(&[value, value, value, 255]);
    }
    bytes
}

pub(crate) struct ImageElement {
    pub image: Arc<RenderImage>,
    pub transparent: bool,
    pub images: Rc<RefCell<Images>>,
}

impl IntoElement for ImageElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for ImageElement {
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
        let style = Style {
            size: Size {
                width: relative(1.).into(),
                height: relative(1.).into(),
            },
            ..Default::default()
        };
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
        let size = self.image.size(0);
        let bounds = ObjectFit::Contain.get_bounds(
            bounds,
            Size {
                width: DevicePixels(size.width.0),
                height: DevicePixels(size.height.0),
            },
        );
        let mut images = self.images.borrow_mut();
        if self.transparent {
            images.checker(window, bounds);
        }
        match window.paint_image(
            bounds,
            bounds,
            Corners::default(),
            self.image.clone(),
            0,
            false,
        ) {
            Ok(()) => {
                if images
                    .painted
                    .as_ref()
                    .is_none_or(|image| image.id != self.image.id)
                {
                    if let Some(old) = images.painted.replace(self.image.clone()) {
                        let _ = window.drop_image(old);
                    }
                }
            }
            Err(error) => tracing::warn!("preview paint failed: {error:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checker_is_opaque_and_repeats_in_both_axes() {
        let bytes = checker_pixels(4, 4, 2);
        let pixel = |x: usize, y: usize| &bytes[(y * 4 + x) * 4..(y * 4 + x + 1) * 4];
        assert_eq!(pixel(0, 0), pixel(2, 2));
        assert_eq!(pixel(0, 2), pixel(2, 0));
        assert_ne!(pixel(0, 0), pixel(2, 0));
        assert!(bytes.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }
}
