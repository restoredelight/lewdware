use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use tiny_skia::{
    GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, Rect, SpreadMode, Stroke,
    Transform,
};
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};

use super::redraw::RedrawRequester;
use super::theme::{Button, ButtonAction, ButtonPaint, ButtonShape, Chrome, Fill, Glyph, Side};
use crate::lua::TextAlign;
use crate::text_font;

/// A window's title bar: the theme's [`Chrome`] painted into a pixmap, plus the pointer state and
/// hit-testing for its buttons.
///
/// Everything here works in **header-local** coordinates — the pixmap's own space, with `(0, 0)`
/// at the header's top-left rather than the window's. [`super::decorations::Decorations`] rebases
/// pointer positions before handing them over, so the border offset is not this type's concern.
pub struct Header {
    redraw: RedrawRequester,
    chrome: Chrome,
    /// Which button the pointer is over, and whether it is pressed. Only ever a button whose
    /// action is not [`ButtonAction::Inert`].
    hovered: Option<usize>,
    pressed: Option<usize>,
    needs_redraw: bool,
    pixmap: Pixmap,
    size: LogicalSize<u32>,
    physical_size: PhysicalSize<u32>,
    scale_factor: f64,
    title: Option<String>,
    /// When false no buttons are drawn at all, so the window cannot be closed by pointer.
    closeable: bool,
}

impl Header {
    /// `header_height` is in logical pixels, from the theme's [`Metrics`](super::theme::Metrics).
    pub fn new(
        redraw: RedrawRequester,
        chrome: Chrome,
        window_size: PhysicalSize<u32>,
        scale_factor: f64,
        header_height: u32,
        title: Option<String>,
        closeable: bool,
    ) -> Self {
        let header_size =
            LogicalSize::new(window_size.to_logical(scale_factor).width, header_height);
        let unclamped: PhysicalSize<u32> = header_size.to_physical(scale_factor);

        // `Pixmap` dimensions are non-zero (tiny-skia's `LengthU32` rejects 0), and a window *can*
        // be asked for with zero width — `lewdware.popup.image(img, { width = 0 })` arrives here
        // as 0 — so clamp once, here, rather than unwrapping a `None` into a crash somewhere
        // downstream. A one-pixel header is invisible, which is the right outcome for a window
        // that has no width to draw in.
        let physical_size = PhysicalSize::new(unclamped.width.max(1), unclamped.height.max(1));

        let pixmap = Pixmap::new(physical_size.width, physical_size.height)
            .expect("clamped to at least 1x1 above");

        Self {
            redraw,
            chrome,
            hovered: None,
            pressed: None,
            needs_redraw: true,
            pixmap,
            physical_size,
            size: header_size,
            scale_factor,
            title,
            closeable,
        }
    }

    /// The buttons actually painted and hit-tested: none at all when the window is not closeable,
    /// since a themed cluster of dead controls would be decoration pretending to be function.
    fn buttons(&self) -> &'static [Button] {
        if self.closeable {
            self.chrome.buttons.buttons
        } else {
            &[]
        }
    }

    /// Button `index`'s horizontal span as `(x, width)` in header-local logical pixels.
    ///
    /// The one place button placement is decided. Drawing and hit-testing both go through it, so
    /// they cannot disagree about where a button is — which is how a left-side cluster (`aqua`,
    /// `platinum`) works without a second copy of this arithmetic.
    ///
    /// Indexes the theme's full button list rather than [`Self::buttons`]: this answers *where* a
    /// button would go, while `buttons()` decides *whether* any are shown. Both callers iterate
    /// `buttons()`, so the indices line up.
    fn button_span(&self, index: usize) -> (f32, f32) {
        let layout = &self.chrome.buttons;
        let height = self.size.height as f32;
        let width = |button: &Button| button.width_ratio * height;

        // Distance from the buttons' own side to this button's outer edge.
        let mut offset = layout.inset;
        for button in &layout.buttons[..index] {
            offset += width(button) + layout.gap;
        }

        let this = width(&layout.buttons[index]);

        let x = match layout.side {
            Side::Left => offset,
            Side::Right => self.size.width as f32 - offset - this,
        };

        (x, this)
    }

    /// The total logical width the button cluster occupies, including its inset and gaps.
    fn buttons_extent(&self) -> f32 {
        let layout = &self.chrome.buttons;
        let buttons = self.buttons();
        if buttons.is_empty() {
            return 0.0;
        }

        let height = self.size.height as f32;
        let widths: f32 = buttons.iter().map(|b| b.width_ratio * height).sum();
        layout.inset + widths + layout.gap * (buttons.len() - 1) as f32
    }

    fn paint_for(&self, index: usize, button: &Button) -> ButtonPaint {
        if button.action == ButtonAction::Inert {
            return button.idle;
        }

        match (self.pressed == Some(index), self.hovered == Some(index)) {
            (true, _) => button.active,
            (false, true) => button.hover,
            (false, false) => button.idle,
        }
    }

    fn transform(&self) -> Transform {
        Transform::from_scale(self.scale_factor as f32, self.scale_factor as f32)
    }

    fn draw_background(&mut self) {
        // Logical, and so possibly zero on a zero-width window — unlike the pixmap, which is
        // clamped. `from_xywh` rejects a zero-sized rect, and there is nothing to paint anyway.
        let Some(rect) = Rect::from_xywh(0.0, 0.0, self.size.width as f32, self.size.height as f32)
        else {
            return;
        };

        self.fill_area(self.chrome.header, rect);
    }

    /// Fill `rect` with `fill`. Anything tiny-skia can express as a `Paint` goes through
    /// [`fill_paint`]; pinstripes are drawn as their own run of lines instead, since a repeating
    /// pattern is not a shader tiny-skia offers.
    fn fill_area(&mut self, fill: Fill, rect: Rect) {
        let transform = self.transform();

        let Fill::Pinstripe {
            base,
            stripe,
            period,
        } = fill
        else {
            self.pixmap
                .fill_rect(rect, &fill_paint(fill, rect), transform, None);
            return;
        };

        self.pixmap
            .fill_rect(rect, &fill_paint(Fill::Solid(base), rect), transform, None);

        let period = period.max(1) as f32;
        let mut paint = Paint::default();
        paint.set_color(to_tiny_skia(stripe));
        // Each stripe is one *logical* pixel, which above 1x covers a fractional number of physical
        // ones. Antialiased, that blends every stripe back towards the base colour and the whole
        // bar reads as flat; hard edges keep it striped at any scale factor.
        paint.anti_alias = false;

        let mut y = rect.top();
        while y < rect.bottom() {
            if let Some(line) = Rect::from_xywh(rect.left(), y, rect.width(), 1.0) {
                self.pixmap.fill_rect(line, &paint, transform, None);
            }
            y += period;
        }
    }

    fn draw_title(&mut self) {
        let Some(text) = self.title.clone() else {
            return;
        };
        let Some(font) = text_font::chrome_font(self.chrome.title.font) else {
            return;
        };

        let style = self.chrome.title;
        let font_size = style.size * self.scale_factor as f32;
        let scale = PxScale::from(font_size);
        let scaled_font = font.as_scaled(scale);

        let text_width = text
            .chars()
            .map(|c| scaled_font.glyph_id(c))
            .fold(0.0, |acc, id| acc + scaled_font.h_advance(id));

        let padding = style.padding * self.scale_factor as f32;
        let buttons = self.buttons_extent() * self.scale_factor as f32;
        let full_width = self.physical_size.width as f32;

        // The span the title may occupy: the header minus its padding, and minus the buttons at
        // whichever end they sit.
        let (safe_left, safe_right) = match self.chrome.buttons.side {
            Side::Left => (padding + buttons, full_width - padding),
            Side::Right => (padding, full_width - buttons.max(padding)),
        };

        // `Center` gives way to the near edge when the text would not fit between the padding and
        // the buttons; `Left`/`Right` sit against their end of the safe span regardless.
        let pen_x = match style.align {
            TextAlign::Left => safe_left,
            TextAlign::Right => (safe_right - text_width).max(safe_left),
            TextAlign::Center => {
                let centered = (full_width - text_width) / 2.0;
                if centered >= safe_left && (centered + text_width) <= safe_right {
                    centered
                } else {
                    safe_left
                }
            }
        };

        // pen_y is the baseline. Center the cap-height/ascent in the header.
        let pen_y = (self.physical_size.height as f32 / 2.0) + (scaled_font.ascent() / 2.0)
            - (1.0 * self.scale_factor as f32);

        let text_pixmap = rasterise_text(
            &text,
            &font,
            scale,
            pen_x,
            pen_y,
            style.color,
            self.physical_size,
        );

        self.pixmap.draw_pixmap(
            0,
            0,
            text_pixmap.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }

    fn draw_buttons(&mut self) {
        let transform = self.transform();
        let height = self.size.height as f32;

        for (index, button) in self.buttons().iter().enumerate() {
            let (x, width) = self.button_span(index);
            let paint = self.paint_for(index, button);

            match button.shape {
                ButtonShape::Rect => {
                    if let Some(rect) = Rect::from_xywh(x, 0.0, width, height) {
                        self.fill_area(paint.fill, rect);
                    }
                }
                ButtonShape::Circle => {
                    let radius = width.min(height) / 2.0;
                    let center = (x + width / 2.0, height / 2.0);
                    let Some(bounds) = Rect::from_xywh(
                        center.0 - radius,
                        center.1 - radius,
                        radius * 2.0,
                        radius * 2.0,
                    ) else {
                        continue;
                    };

                    let mut path = PathBuilder::new();
                    path.push_circle(center.0, center.1, radius);
                    if let Some(path) = path.finish() {
                        self.pixmap.fill_path(
                            &path,
                            &fill_paint(paint.fill, bounds),
                            tiny_skia::FillRule::Winding,
                            transform,
                            None,
                        );
                    }
                }
            }

            self.draw_glyph(button.glyph, button.shape, x, width, paint.glyph, transform);
        }
    }

    /// How far a glyph reaches from its button's centre, on each axis.
    ///
    /// Sized from the *button*, not the title bar: scaling a cross to a 28px header inside `aqua`'s
    /// 12px light drew it outside the circle entirely. Shared with the test that checks glyphs stay
    /// inside their buttons, so the two cannot drift apart.
    fn glyph_reach(&self, shape: ButtonShape, width: f32) -> f32 {
        let extent = width.min(self.size.height as f32);
        match shape {
            ButtonShape::Rect => extent / 6.0,
            // A cross inscribed in a circle reaches `offset * sqrt(2)` from the centre, so it has
            // to be pulled in further than in a rectangle of the same size.
            ButtonShape::Circle => extent / 6.0 * std::f32::consts::FRAC_1_SQRT_2,
        }
    }

    fn draw_glyph(
        &mut self,
        glyph: Glyph,
        shape: ButtonShape,
        x: f32,
        width: f32,
        color: crate::lua::Color,
        transform: Transform,
    ) {
        let Glyph::Cross = glyph else {
            return;
        };

        let mut paint = Paint::default();
        paint.set_color(to_tiny_skia(color));

        let height = self.size.height as f32;
        let middle_x = x + width / 2.0;
        let middle_y = height / 2.0;

        let offset = self.glyph_reach(shape, width);

        for (from, to) in [
            ((-offset, -offset), (offset, offset)),
            ((-offset, offset), (offset, -offset)),
        ] {
            let mut line = PathBuilder::new();
            line.move_to(middle_x + from.0, middle_y + from.1);
            line.line_to(middle_x + to.0, middle_y + to.1);

            if let Some(path) = line.finish() {
                self.pixmap
                    .stroke_path(&path, &paint, &Stroke::default(), transform, None);
            }
        }
    }

    pub fn draw(&mut self) -> Option<&Pixmap> {
        if !self.needs_redraw {
            return None;
        }

        self.render();

        Some(&self.pixmap)
    }

    /// Re-render the pixmap if dirty, then always return it. Use this on the softbuffer path
    /// where the buffer is not guaranteed to retain previous frame content (e.g. macOS).
    pub fn get_pixmap(&mut self) -> &Pixmap {
        if self.needs_redraw {
            self.render();
        }
        &self.pixmap
    }

    /// Repaint the whole header.
    ///
    /// Unconditional rather than tracking which parts changed: a themed header's title and buttons
    /// can be drawn over any fill (a gradient, a bevel), so there is no longer a "background is
    /// already there" state that a partial repaint could safely build on.
    fn render(&mut self) {
        self.draw_background();
        self.draw_title();
        self.draw_buttons();

        self.needs_redraw = false;
    }

    /// The button under `position`, given in header-local **physical** pixels. `None` for inert
    /// buttons, so they can never hover or activate.
    fn button_at(&self, position: PhysicalPosition<f64>) -> Option<usize> {
        // Signed logical, so a pointer over the border above the header stays negative rather
        // than wrapping into the header's own range.
        let position: LogicalPosition<f64> = position.to_logical(self.scale_factor);

        if position.y < 0.0 || position.y > self.size.height as f64 {
            return None;
        }

        self.buttons()
            .iter()
            .enumerate()
            .filter(|(_, button)| button.action != ButtonAction::Inert)
            .find(|(index, _)| {
                let (x, width) = self.button_span(*index);
                position.x >= x as f64 && position.x < (x + width) as f64
            })
            .map(|(index, _)| index)
    }

    fn request_redraw(&mut self) {
        self.needs_redraw = true;
        self.redraw.request_redraw();
    }

    pub fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        let hovered = self.button_at(position);

        if self.hovered != hovered {
            self.hovered = hovered;
            self.request_redraw();
        }
    }

    pub fn handle_cursor_left(&mut self) {
        if self.hovered.is_some() || self.pressed.is_some() {
            self.hovered = None;
            self.pressed = None;
            self.request_redraw();
        }
    }

    pub fn handle_mouse_down(&mut self) {
        if let Some(hovered) = self.hovered
            && self.pressed != Some(hovered)
        {
            self.pressed = Some(hovered);
            self.request_redraw();
        }
    }

    /// Returns whether a close button was activated: pressed and released over the same button.
    pub fn handle_mouse_up(&mut self) -> bool {
        let Some(pressed) = self.pressed.take() else {
            return false;
        };

        if self.hovered == Some(pressed) {
            return self.buttons()[pressed].action == ButtonAction::Close;
        }

        self.request_redraw();
        false
    }

    pub fn set_title(&mut self, text: Option<String>) {
        self.title = text;
        self.request_redraw();
    }
}

/// Rasterise `text` into its own transparent pixmap, ready to be composited over the header.
///
/// Separate from [`Header::draw_title`] so the pixels can be inspected directly: this is the one
/// place in the engine that writes into a tiny-skia `Pixmap` by hand rather than going through a
/// `Paint`, and `Pixmap` is **premultiplied**, so it is the one place that can produce pixels
/// tiny-skia cannot composite. Compositing them does not fail loudly — `draw_pixmap` clamps, and
/// the visible result is a fat solid block where an antialiased glyph edge should be — so the
/// invariant has to be checked here, before the blend hides it.
#[allow(clippy::too_many_arguments)]
fn rasterise_text(
    text: &str,
    font: &FontArc,
    scale: PxScale,
    mut pen_x: f32,
    pen_y: f32,
    color: crate::lua::Color,
    size: PhysicalSize<u32>,
) -> Pixmap {
    let scaled_font = font.as_scaled(scale);
    let color = to_tiny_skia(color).to_color_u8();
    let color_alpha = color.alpha() as f32 / 255.0;

    let mut pixmap =
        Pixmap::new(size.width, size.height).expect("size is clamped to at least 1x1 in `new`");
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let data = pixmap.data_mut();

    for c in text.chars() {
        let glyph_id = scaled_font.glyph_id(c);
        let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(pen_x, pen_y));

        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();

            outlined.draw(|x, y, coverage| {
                let px = bounds.min.x as i32 + x as i32;
                let py = bounds.min.y as i32 + y as i32;

                if px >= 0 && px < width && py >= 0 && py < height {
                    let index = ((py * width + px) * 4) as usize;

                    // Premultiplied: every channel scaled by the alpha it is stored with. Writing
                    // the straight colour leaves `channel > alpha`, which is not a representable
                    // premultiplied pixel.
                    //
                    // This only ever wrote black before the title colour was themeable, and
                    // `(0, 0, 0, a)` is coincidentally valid at any alpha — which is why the bug
                    // stayed hidden until a theme asked for a white title.
                    let alpha = coverage * color_alpha;
                    let channel = |value: u8| (value as f32 * alpha).round() as u8;

                    // Overlapping glyphs take whichever covers the pixel more, rather than
                    // whichever came last.
                    if (alpha * 255.0).round() as u8 <= data[index + 3] {
                        return;
                    }

                    data[index] = channel(color.red());
                    data[index + 1] = channel(color.green());
                    data[index + 2] = channel(color.blue());
                    data[index + 3] = (alpha * 255.0).round() as u8;
                }
            });
        }

        pen_x += scaled_font.h_advance(glyph_id);
    }

    pixmap
}

/// A tiny-skia paint for `fill`. `bounds` is the area being filled, which a gradient needs in
/// order to know where its stops land.
fn fill_paint(fill: Fill, bounds: Rect) -> Paint<'static> {
    let mut paint = Paint::default();

    match fill {
        Fill::Solid(color) => paint.set_color(to_tiny_skia(color)),
        // Handled by `Header::fill_area`, which needs the pixmap; falling back to the base
        // colour keeps this function total rather than panicking if it is ever reached directly.
        Fill::Pinstripe { base, .. } => paint.set_color(to_tiny_skia(base)),
        Fill::VerticalGradient { from, to } => {
            let shader = LinearGradient::new(
                Point::from_xy(bounds.left(), bounds.top()),
                Point::from_xy(bounds.left(), bounds.bottom()),
                vec![
                    GradientStop::new(0.0, to_tiny_skia(from)),
                    GradientStop::new(1.0, to_tiny_skia(to)),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            );

            // A degenerate gradient (zero height) has no shader; fall back to its first stop
            // rather than leaving the area unpainted.
            match shader {
                Some(shader) => paint.shader = shader,
                None => paint.set_color(to_tiny_skia(from)),
            }
        }
    }

    paint
}

pub fn to_tiny_skia(c: crate::lua::Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(c.r, c.g, c.b, c.a).unwrap_or(tiny_skia::Color::BLACK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::Color;
    use crate::window::theme::{Appearance, Buttons, Theme};

    const OPAQUE: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    const PAINT: ButtonPaint = ButtonPaint {
        fill: Fill::Solid(OPAQUE),
        glyph: OPAQUE,
    };

    const fn button(action: ButtonAction, width_ratio: f32) -> Button {
        Button {
            action,
            shape: ButtonShape::Circle,
            glyph: Glyph::None,
            width_ratio,
            idle: PAINT,
            hover: PAINT,
            active: PAINT,
        }
    }

    /// A macOS-shaped cluster: close, then two lights that only complete the silhouette.
    static TRAFFIC_LIGHTS: [Button; 3] = [
        button(ButtonAction::Close, 1.0),
        button(ButtonAction::Inert, 1.0),
        button(ButtonAction::Inert, 1.0),
    ];

    const HEADER_H: u32 = 20;
    const WINDOW_W: u32 = 200;

    fn header(buttons: Buttons, closeable: bool) -> Header {
        let chrome = Chrome {
            buttons,
            ..Theme::Plain.chrome(Appearance::Light)
        };

        Header::new(
            RedrawRequester::detached(),
            chrome,
            PhysicalSize::new(WINDOW_W, 100),
            1.0,
            HEADER_H,
            None,
            closeable,
        )
    }

    fn plain_header() -> Header {
        header(Theme::Plain.chrome(Appearance::Light).buttons, true)
    }

    /// Header-local physical position. At 1x this is also the logical position.
    fn at(x: f64, y: f64) -> PhysicalPosition<f64> {
        PhysicalPosition::new(x, y)
    }

    #[test]
    fn a_right_side_button_sits_against_the_far_edge() {
        let header = plain_header();
        let (x, width) = header.button_span(0);
        let ratio = Theme::Plain.chrome(Appearance::Light).buttons.buttons[0].width_ratio;

        assert_eq!(width, ratio * HEADER_H as f32);
        assert_eq!(x, WINDOW_W as f32 - width);
    }

    /// A left-side cluster runs inwards from the near edge, in the order the theme lists it —
    /// the placement `aqua` and `platinum` need, from the same arithmetic the right side uses.
    #[test]
    fn a_left_side_cluster_runs_inwards_from_the_near_edge() {
        let header = header(
            Buttons {
                side: Side::Left,
                inset: 8.0,
                gap: 4.0,
                buttons: &TRAFFIC_LIGHTS,
            },
            true,
        );

        let size = HEADER_H as f32;
        assert_eq!(header.button_span(0), (8.0, size));
        assert_eq!(header.button_span(1), (8.0 + size + 4.0, size));
        assert_eq!(header.button_span(2), (8.0 + 2.0 * (size + 4.0), size));
    }

    #[test]
    fn the_pointer_finds_the_button_it_is_over() {
        let header = plain_header();
        let (x, width) = header.button_span(0);

        assert_eq!(header.button_at(at(x as f64 + 1.0, 5.0)), Some(0));
        assert_eq!(
            header.button_at(at(x as f64 + width as f64 - 0.5, 5.0)),
            Some(0)
        );
        // Just outside each edge.
        assert_eq!(header.button_at(at(x as f64 - 1.0, 5.0)), None);
        assert_eq!(header.button_at(at((x + width) as f64, 5.0)), None);
    }

    /// The border sits above the header, so a pointer there arrives with a negative local y. It
    /// must read as outside rather than wrapping into the header's own range.
    #[test]
    fn a_pointer_above_the_header_is_outside_it() {
        let header = plain_header();
        let (x, _) = header.button_span(0);

        assert_eq!(header.button_at(at(x as f64 + 1.0, -1.0)), None);
        assert_eq!(
            header.button_at(at(x as f64 + 1.0, HEADER_H as f64 + 1.0)),
            None
        );
    }

    /// Inert buttons are never hit-tested, so they cannot hover or activate — the honesty rule
    /// in `design/window-themes.md`. Only the close light responds.
    #[test]
    fn inert_buttons_are_never_hit() {
        let header = header(
            Buttons {
                side: Side::Left,
                inset: 0.0,
                gap: 0.0,
                buttons: &TRAFFIC_LIGHTS,
            },
            true,
        );

        for (index, light) in TRAFFIC_LIGHTS.iter().enumerate() {
            let (x, _) = header.button_span(index);
            let hit = header.button_at(at(x as f64 + 1.0, 5.0));
            match light.action {
                ButtonAction::Close => assert_eq!(hit, Some(index), "close light {index}"),
                ButtonAction::Inert => assert_eq!(hit, None, "inert light {index}"),
            }
        }
    }

    /// An inert button stays on its idle paint whatever the pointer is doing.
    #[test]
    fn inert_buttons_do_not_change_paint() {
        let mut inert = button(ButtonAction::Inert, 1.0);
        inert.hover = ButtonPaint {
            fill: Fill::Solid(Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            }),
            glyph: OPAQUE,
        };

        let mut header = header(
            Buttons {
                side: Side::Left,
                inset: 0.0,
                gap: 0.0,
                buttons: &TRAFFIC_LIGHTS,
            },
            true,
        );
        // Pretend the pointer somehow settled on it, which `button_at` will not do.
        header.hovered = Some(1);

        assert_eq!(header.paint_for(1, &inert), inert.idle);
    }

    #[test]
    fn an_unclosable_window_has_no_buttons_at_all() {
        let mut header = header(Theme::Plain.chrome(Appearance::Light).buttons, false);
        let (x, _) = plain_header().button_span(0);

        assert!(header.buttons().is_empty());
        assert_eq!(header.button_at(at(x as f64 + 1.0, 5.0)), None);

        header.handle_cursor_moved(at(x as f64 + 1.0, 5.0));
        header.handle_mouse_down();
        assert!(!header.handle_mouse_up());
    }

    #[test]
    fn pressing_and_releasing_the_close_button_activates_it() {
        let mut header = plain_header();
        let (x, _) = header.button_span(0);
        let over = at(x as f64 + 1.0, 5.0);

        header.handle_cursor_moved(over);
        header.handle_mouse_down();
        assert!(header.handle_mouse_up());
    }

    /// Releasing away from the button that was pressed cancels, as every platform's buttons do.
    #[test]
    fn releasing_off_the_button_does_not_activate_it() {
        let mut header = plain_header();
        let (x, _) = header.button_span(0);

        header.handle_cursor_moved(at(x as f64 + 1.0, 5.0));
        header.handle_mouse_down();
        header.handle_cursor_moved(at(1.0, 5.0));
        assert!(!header.handle_mouse_up());
    }

    #[test]
    fn a_release_without_a_press_does_nothing() {
        let mut header = plain_header();
        let (x, _) = header.button_span(0);

        header.handle_cursor_moved(at(x as f64 + 1.0, 5.0));
        assert!(!header.handle_mouse_up());
    }

    #[test]
    fn leaving_the_window_clears_the_pointer_state() {
        let mut header = plain_header();
        let (x, _) = header.button_span(0);

        header.handle_cursor_moved(at(x as f64 + 1.0, 5.0));
        header.handle_mouse_down();
        header.handle_cursor_left();

        assert_eq!(header.hovered, None);
        assert_eq!(header.pressed, None);
        assert!(!header.handle_mouse_up());
    }

    /// Hit-testing scales with the window, so a HiDPI pointer position lands on the same button
    /// it visually points at.
    #[test]
    fn hit_testing_follows_the_scale_factor() {
        let chrome = Theme::Plain.chrome(Appearance::Light);
        let scale = 2.0;
        let header = Header::new(
            RedrawRequester::detached(),
            chrome,
            PhysicalSize::new(WINDOW_W * 2, 200),
            scale,
            HEADER_H,
            None,
            true,
        );

        let (x, width) = header.button_span(0);
        // A physical position in the middle of the button's physical span.
        let physical_x = (x + width / 2.0) as f64 * scale;

        assert_eq!(header.button_at(at(physical_x, 5.0)), Some(0));
        assert_eq!(header.button_at(at((x as f64 - 2.0) * scale, 5.0)), None);
    }

    fn pixel(header: &mut Header, x: u32, y: u32) -> [u8; 4] {
        let pixmap = header.get_pixmap();
        let index = ((y * pixmap.width() + x) * 4) as usize;
        let data = pixmap.data();
        [
            data[index],
            data[index + 1],
            data[index + 2],
            data[index + 3],
        ]
    }

    /// The painted result for `plain`, not just the data behind it: a flat grey bar, a close button
    /// invisible until hovered, and red when it is. Read from the pixels rather than the palette, so
    /// it covers the drawing as well as the data.
    #[test]
    fn plain_paints_a_flat_grey_bar_with_a_red_close_on_hover() {
        const GREY: [u8; 4] = [232, 232, 232, 255];
        const RED: [u8; 4] = [255, 0, 0, 255];

        let mut header = plain_header();
        let (button_x, _) = header.button_span(0);
        // Inside the button, but off-centre so the cross glyph is not in the way.
        let in_button = (button_x as u32 + 2, 2);
        let in_bar = (2, HEADER_H / 2);

        assert_eq!(pixel(&mut header, in_bar.0, in_bar.1), GREY, "title bar");
        assert_eq!(
            pixel(&mut header, in_button.0, in_button.1),
            GREY,
            "idle close button is the bar's own grey"
        );

        header.handle_cursor_moved(at(button_x as f64 + 2.0, 2.0));
        assert_eq!(
            pixel(&mut header, in_button.0, in_button.1),
            RED,
            "hovered close button"
        );
        // The rest of the bar is untouched by the button's state.
        assert_eq!(pixel(&mut header, in_bar.0, in_bar.1), GREY, "title bar");
    }

    /// Every theme renders at every scale factor and window size, without panicking and without
    /// leaving the bar unpainted.
    ///
    /// The catalogue is data, so a bad number in it — a button wider than its bar, a zero-size
    /// pixmap, a degenerate gradient — would only show up when something tried to draw it.
    #[test]
    fn every_theme_renders_at_every_size() {
        for &theme in crate::window::theme::ALL_THEMES {
            let metrics = theme.metrics();

            for appearance in [Appearance::Light, Appearance::Dark] {
                for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
                    // Including widths narrower than some themes' button clusters, and zero.
                    for width in [0u32, 1, 10, 60, 400] {
                        let mut header = Header::new(
                            RedrawRequester::detached(),
                            theme.chrome(appearance),
                            LogicalSize::new(width, 100).to_physical(scale),
                            scale,
                            metrics.header_height,
                            Some("A window title long enough to overflow a narrow bar".to_owned()),
                            true,
                        );

                        let pixmap = header.get_pixmap();
                        assert!(
                            pixmap.width() > 0 && pixmap.height() > 0,
                            "{theme:?} {appearance:?} at {scale}x, {width}px: empty pixmap"
                        );

                        // Anything with room to draw in paints its bar; a zero-width window has none,
                        // and correctly paints nothing rather than crashing.
                        if width > 0 {
                            assert!(
                                pixmap.data().chunks_exact(4).any(|px| px[3] > 0),
                                "{theme:?} {appearance:?} at {scale}x, {width}px: nothing painted"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Hover and press repaint every theme, including the ones whose buttons are circles or sit on
    /// the left — the states a static render never reaches.
    #[test]
    fn every_theme_repaints_through_its_pointer_states() {
        for &theme in crate::window::theme::ALL_THEMES {
            let metrics = theme.metrics();
            let mut header = Header::new(
                RedrawRequester::detached(),
                theme.chrome(Appearance::Light),
                PhysicalSize::new(300, 100),
                1.0,
                metrics.header_height,
                Some("title".to_owned()),
                true,
            );

            let (x, width) = header.button_span(0);
            let over = at((x + width / 2.0) as f64, (metrics.header_height / 2) as f64);

            header.handle_cursor_moved(over);
            assert_eq!(header.hovered, Some(0), "{theme:?} did not hover its close");
            let _ = header.get_pixmap();

            header.handle_mouse_down();
            let _ = header.get_pixmap();
            assert!(header.handle_mouse_up(), "{theme:?} did not close");
        }
    }

    /// The invariant that was broken: every pixel the title rasteriser writes must be valid
    /// **premultiplied** RGBA, with no channel above its own alpha.
    ///
    /// Asserted on the rasteriser's own output rather than the finished header, because
    /// `draw_pixmap` clamps invalid source pixels while compositing — the header ends up
    /// valid-but-wrong, so a check there passes even when this is broken.
    #[test]
    fn the_title_rasteriser_writes_premultiplied_pixels() {
        let font = text_font::chrome_font(crate::lua::TextFont::Default).expect("default font");
        let size = PhysicalSize::new(200, 30);

        // White is the case that broke: the brightest possible colour against any coverage.
        for color in [
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 0.5,
            },
            Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        ] {
            let pixmap =
                rasterise_text("Title", &font, PxScale::from(20.0), 4.0, 20.0, color, size);

            for (index, px) in pixmap.data().chunks_exact(4).enumerate() {
                let (r, g, b, a) = (px[0], px[1], px[2], px[3]);
                assert!(
                    r <= a && g <= a && b <= a,
                    "{color:?}: pixel {index} is not premultiplied (rgba {r},{g},{b},{a})"
                );
            }
        }
    }

    /// Antialiasing has to survive into the finished header.
    ///
    /// The premultiplication bug did not make glyphs vanish — it made every covered pixel clamp to
    /// full brightness, turning antialiased edges into fat solid blocks. So "is the title visible"
    /// passes either way; what distinguishes correct output is the presence of *intermediate*
    /// tones between the bar and the title colour.
    #[test]
    fn a_light_title_keeps_its_antialiased_edges() {
        let theme = crate::window::theme::Theme::Fluent;
        let chrome = theme.chrome(Appearance::Dark);
        // The palette this is asserting about: near-white text on a near-black bar.
        assert!(chrome.title.color.r > 0.9);
        let Fill::Solid(bar) = chrome.header else {
            panic!("fluent's bar is a solid fill");
        };
        let bar = (bar.r * 255.0).round() as u8;

        let mut header = Header::new(
            RedrawRequester::detached(),
            chrome,
            PhysicalSize::new(400, 100),
            1.0,
            theme.metrics().header_height,
            Some("Title text".to_owned()),
            true,
        );

        let reds: Vec<u8> = header
            .get_pixmap()
            .data()
            .chunks_exact(4)
            .map(|px| px[0])
            .collect();

        // The glyphs are there at all.
        let brightest = *reds.iter().max().unwrap();
        assert!(
            brightest > bar + 40,
            "no title drawn: {brightest} vs bar {bar}"
        );

        // And they have soft edges: pixels partway between the bar and the text colour.
        let intermediate = reds
            .iter()
            .filter(|&&red| red > bar + 20 && red < brightest - 20)
            .count();
        assert!(
            intermediate > 20,
            "title has no antialiased edges — only {intermediate} intermediate pixels, \
             which is what writing straight alpha into a premultiplied pixmap produces"
        );
    }

    /// A close button's glyph has to stay inside the button it marks.
    ///
    /// It used to be scaled from the *title bar* height, so `aqua`'s 12px traffic light got a cross
    /// sized for a 28px header and spilled out of the circle.
    #[test]
    fn every_glyph_stays_inside_its_button() {
        for &theme in crate::window::theme::ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let chrome = theme.chrome(appearance);
                let header_height = theme.metrics().header_height as f32;

                let header = Header::new(
                    RedrawRequester::detached(),
                    chrome,
                    PhysicalSize::new(400, 100),
                    1.0,
                    theme.metrics().header_height,
                    None,
                    true,
                );

                for (index, button) in chrome.buttons.buttons.iter().enumerate() {
                    if button.glyph == Glyph::None {
                        continue;
                    }

                    let (x, width) = header.button_span(index);
                    let reach = header.glyph_reach(button.shape, width);

                    // The cross's corners, which are its furthest points from the centre.
                    let (centre_x, centre_y) = (x + width / 2.0, header_height / 2.0);
                    let corner_x = reach;
                    let corner_y = reach;

                    match button.shape {
                        ButtonShape::Rect => {
                            assert!(
                                centre_x - corner_x >= x && centre_x + corner_x <= x + width,
                                "{theme:?} {appearance:?} button {index}: cross is wider than its \
                                 {width}px button"
                            );
                            assert!(
                                centre_y - corner_y >= 0.0 && centre_y + corner_y <= header_height,
                                "{theme:?} {appearance:?} button {index}: cross is taller than the \
                                 {header_height}px bar"
                            );
                        }
                        ButtonShape::Circle => {
                            let radius = width.min(header_height) / 2.0;
                            let corner = (corner_x * corner_x + corner_y * corner_y).sqrt();
                            assert!(
                                corner <= radius,
                                "{theme:?} {appearance:?} button {index}: the cross reaches \
                                 {corner} from the centre of a circle only {radius} in radius"
                            );
                        }
                    }
                }
            }
        }
    }

    /// `platinum`'s pinstripes have to survive rendering, not just exist in the palette.
    ///
    /// A stripe is one *logical* pixel, so above 1x it covers a fractional number of physical ones.
    /// Antialiased, each stripe blends towards the base and the bar collapses into a smooth ramp —
    /// which is what made it read as flat grey. So the property checked here is **bimodality**: the
    /// bar's rows should cluster near the two palette colours rather than smear between them.
    #[test]
    fn platinum_pinstripes_survive_a_fractional_scale_factor() {
        let theme = crate::window::theme::Theme::Platinum;
        let Fill::Pinstripe { base, stripe, .. } = theme.chrome(Appearance::Light).header else {
            panic!("platinum's bar is pinstriped");
        };

        let level = |color: crate::lua::Color| (color.r * 255.0).round() as i32;
        let (base, stripe) = (level(base), level(stripe));

        // Calibrated, not arbitrary: the pair this replaced was 17 levels apart and read as flat.
        assert!(
            (stripe - base).abs() >= 20,
            "the pinstripe pair is only {} levels apart",
            (stripe - base).abs()
        );

        for scale in [1.0, 1.5, 2.0, 2.09375, 3.0] {
            let mut header = Header::new(
                RedrawRequester::detached(),
                theme.chrome(Appearance::Light),
                LogicalSize::new(200u32, 100).to_physical(scale),
                scale,
                theme.metrics().header_height,
                None,
                true,
            );

            // Well right of the left-hand close box, so the stripes are unobstructed.
            let rows: Vec<i32> = (0..header.get_pixmap().height())
                .map(|y| pixel(&mut header, 150, y)[0] as i32)
                .collect();

            // A quarter of the palette difference is generous: it takes only a little blending to
            // pull a row out of its cluster.
            let tolerance = (stripe - base).abs() / 4;
            let near = |value: i32, target: i32| (value - target).abs() <= tolerance;

            let at_base = rows.iter().filter(|&&row| near(row, base)).count();
            let at_stripe = rows.iter().filter(|&&row| near(row, stripe)).count();
            let smeared = rows.len() - at_base - at_stripe;

            assert!(
                at_base > 0 && at_stripe > 0,
                "at {scale}x the bar has {at_base} base rows and {at_stripe} stripe rows, \
                 so it is not striped"
            );
            assert!(
                smeared * 3 < rows.len(),
                "at {scale}x {smeared} of {} rows sit between the two colours — the stripes are \
                 blending into a flat ramp",
                rows.len()
            );
        }
    }

    /// Hovering a close button must always change how it looks.
    ///
    /// Deliberately *not* a contrast threshold between the button and its bar: `plain` and `fluent`
    /// use the bar's own colour so the button is invisible until hovered (which is what Windows
    /// does), while `platinum`'s close box is faint by design. How loud a button is at rest is a
    /// per-theme judgement; that it responds at all is not.
    #[test]
    fn hovering_a_close_button_always_changes_it() {
        for &theme in crate::window::theme::ALL_THEMES {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let Some(close) = theme
                    .chrome(appearance)
                    .buttons
                    .buttons
                    .iter()
                    .find(|button| button.action == ButtonAction::Close)
                else {
                    continue;
                };

                assert_ne!(
                    close.idle, close.hover,
                    "{theme:?} {appearance:?}: hovering the close button changes nothing"
                );
                assert_ne!(
                    close.hover, close.active,
                    "{theme:?} {appearance:?}: pressing the close button changes nothing"
                );
            }
        }
    }

    /// A gradient fill actually varies down the header rather than painting flat — the mechanism
    /// `aqua`/`platinum` need, with no theme using it yet.
    #[test]
    fn a_gradient_header_varies_from_top_to_bottom() {
        let chrome = Chrome {
            header: Fill::VerticalGradient {
                from: Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
                to: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
            },
            buttons: Buttons {
                side: Side::Right,
                inset: 0.0,
                gap: 0.0,
                buttons: &[],
            },
            ..Theme::Plain.chrome(Appearance::Light)
        };

        let mut header = Header::new(
            RedrawRequester::detached(),
            chrome,
            PhysicalSize::new(WINDOW_W, 100),
            1.0,
            HEADER_H,
            None,
            true,
        );

        let top = pixel(&mut header, 2, 0)[0];
        let bottom = pixel(&mut header, 2, HEADER_H - 1)[0];
        assert!(
            top < bottom,
            "gradient did not darken upwards: {top} → {bottom}"
        );
    }

    /// Every state paints something: a themed header is repainted whole each time, so a missing
    /// arm would leave a stale button rather than a wrong colour.
    #[test]
    fn each_pointer_state_selects_its_own_paint() {
        let mut header = plain_header();
        let close = &Theme::Plain.chrome(Appearance::Light).buttons.buttons[0];

        assert_eq!(header.paint_for(0, close), close.idle);

        header.hovered = Some(0);
        assert_eq!(header.paint_for(0, close), close.hover);

        header.pressed = Some(0);
        assert_eq!(header.paint_for(0, close), close.active);
    }
}
