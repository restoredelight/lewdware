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

    /// The buttons actually painted. Hit-testing goes through [`Self::button_at`], which refuses
    /// every one of them on an unclosable window.
    ///
    /// A window that cannot be closed either drops the cluster entirely or draws it in the theme's
    /// disabled paint ([`Buttons::unclosable`]), depending on what the platform does. Neither
    /// lies about function: a *coloured* button that did nothing would be the violation, while a
    /// greyed-out one says "this window cannot be closed" — which is otherwise something the user
    /// can only find out by clicking. See `design/window-themes.md`, "Aqua's traffic lights, and
    /// the function-honesty rule".
    fn buttons(&self) -> &'static [Button] {
        if self.closeable || self.chrome.buttons.unclosable.is_some() {
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
        // Disabled beats every pointer state: an unclosable window's cluster is drawn only because
        // the theme supplied a paint saying so, and nothing in it can be hovered or pressed.
        if !self.closeable {
            return self.chrome.buttons.unclosable.unwrap_or(button.idle);
        }

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

    /// The theme's hairline along the bottom edge of the bar, if it has one.
    ///
    /// Drawn last, over the buttons as well as the bar: it marks where the chrome ends, and a
    /// button that interrupted it would make the line look like it belonged to the bar's fill
    /// rather than to the window's structure.
    fn draw_separator(&mut self) {
        let Some(color) = self.chrome.separator else {
            return;
        };

        // One *logical* pixel, at the bottom of the bar — the same visual weight at every scale
        // factor, since the transform scales it.
        let Some(rect) = Rect::from_xywh(
            0.0,
            self.size.height as f32 - 1.0,
            self.size.width as f32,
            1.0,
        ) else {
            return;
        };

        let mut paint = Paint::default();
        paint.set_color(to_tiny_skia(color));
        // Above 1x this line covers a fractional number of physical pixels. Antialiased, it blends
        // back towards the bar and the edge it is there to draw stops reading as an edge — the
        // same reason `fill_area` draws pinstripes hard.
        paint.anti_alias = false;

        self.pixmap.fill_rect(rect, &paint, self.transform(), None);
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

        // Nowhere to draw: a window too narrow to hold its own buttons.
        if safe_right <= safe_left {
            return;
        }

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

        // A pinstriped classic-Mac title bar interrupts its stripes behind the caption. Printing
        // directly over them makes the letters shimmer and looks like a texture laid under text,
        // not Platinum chrome. Use the midpoint of the stripe pair so the plaque belongs to the
        // bar while remaining visibly solid.
        if let Fill::Pinstripe { base, stripe, .. } = self.chrome.header {
            let average = crate::lua::Color {
                r: (base.r + stripe.r) / 2.0,
                g: (base.g + stripe.g) / 2.0,
                b: (base.b + stripe.b) / 2.0,
                a: (base.a + stripe.a) / 2.0,
            };
            let plaque_padding = 4.0 * self.scale_factor as f32;
            let left = (pen_x - plaque_padding).max(safe_left);
            let right = (pen_x + text_width + plaque_padding).min(safe_right);
            if let Some(rect) = Rect::from_xywh(
                left,
                0.0,
                (right - left).max(0.0),
                self.physical_size.height as f32,
            ) {
                self.pixmap.fill_rect(
                    rect,
                    &fill_paint(Fill::Solid(average), rect),
                    Transform::identity(),
                    None,
                );
            }
        }

        let text_pixmap = rasterise_text(
            &text,
            &font,
            scale,
            pen_x,
            pen_y,
            style.color,
            self.physical_size,
            (safe_left, safe_right),
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
                ButtonShape::Square => {
                    let size = (height * button.diameter_ratio).min(width).min(height);
                    if let Some(rect) =
                        Rect::from_xywh(x + (width - size) / 2.0, (height - size) / 2.0, size, size)
                    {
                        self.fill_area(paint.fill, rect);
                    }
                }
                ButtonShape::Circle => {
                    // The layout slot and painted disc are separate measurements. In particular,
                    // Adwaita centres a 24px disc in a 34px image-button slot.
                    let diameter = (height * button.diameter_ratio).min(width).min(height);
                    let radius = diameter / 2.0;
                    let center = (x + width / 2.0, height / 2.0);
                    let Some(bounds) = Rect::from_xywh(
                        center.0 - radius,
                        center.1 - radius,
                        radius * 2.0,
                        radius * 2.0,
                    ) else {
                        continue;
                    };

                    if let Some(rim) = paint.rim {
                        let mut rim_path = PathBuilder::new();
                        rim_path.push_circle(center.0, center.1, radius);
                        if let Some(rim_path) = rim_path.finish() {
                            let mut rim_paint = Paint::default();
                            rim_paint.set_color(to_tiny_skia(rim));
                            self.pixmap.fill_path(
                                &rim_path,
                                &rim_paint,
                                tiny_skia::FillRule::Winding,
                                transform,
                                None,
                            );
                        }
                    }

                    let inner_radius = if paint.rim.is_some() {
                        (radius - 1.0).max(0.0)
                    } else {
                        radius
                    };
                    let mut path = PathBuilder::new();
                    path.push_circle(center.0, center.1, inner_radius);
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

            self.draw_glyph(button, x, width, paint.glyph, transform);
        }
    }

    /// How far a glyph reaches from its button's centre, on each axis.
    ///
    /// Sized from the *button*, not the title bar: scaling a cross to a 28px header inside `aqua`'s
    /// 12px light drew it outside the circle entirely. Shared with the test that checks glyphs stay
    /// inside their buttons, so the two cannot drift apart.
    ///
    /// The proportion itself is the theme's ([`Button::glyph_ratio`]), not this function's. It used
    /// to be decided here, keyed on the [`Glyph`] variant — which meant every theme drawing a cross
    /// shared one number, so correcting macOS's mark silently resized KDE's and GNOME's too.
    fn glyph_reach(&self, button: &Button, width: f32) -> f32 {
        if button.glyph == Glyph::None {
            return 0.0;
        }

        let extent = match button.shape {
            ButtonShape::Rect => width.min(self.size.height as f32),
            ButtonShape::Square | ButtonShape::Circle => (self.size.height as f32
                * button.diameter_ratio)
                .min(width)
                .min(self.size.height as f32),
        };
        extent * button.glyph_ratio
    }

    fn draw_glyph(
        &mut self,
        button: &Button,
        x: f32,
        width: f32,
        color: crate::lua::Color,
        transform: Transform,
    ) {
        let offset = self.glyph_reach(button, width);
        if offset <= 0.0 {
            return;
        }

        let mut paint = Paint::default();
        paint.set_color(to_tiny_skia(color));

        let height = self.size.height as f32;
        let middle_x = x + width / 2.0;
        let middle_y = height / 2.0;

        match button.glyph {
            Glyph::Cross => {
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
            Glyph::Square => {
                let mut outline = PathBuilder::new();
                outline.move_to(middle_x - offset, middle_y - offset);
                outline.line_to(middle_x + offset, middle_y - offset);
                outline.line_to(middle_x + offset, middle_y + offset);
                outline.line_to(middle_x - offset, middle_y + offset);
                outline.close();
                if let Some(path) = outline.finish() {
                    self.pixmap
                        .stroke_path(&path, &paint, &Stroke::default(), transform, None);
                }
            }
            Glyph::None => {}
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
        self.draw_separator();

        self.needs_redraw = false;
    }

    /// The button under `position`, given in header-local **physical** pixels. `None` for inert
    /// buttons, so they can never hover or activate.
    fn button_at(&self, position: PhysicalPosition<f64>) -> Option<usize> {
        // A disabled cluster is painted but never live -- see `buttons()`.
        if !self.closeable {
            return None;
        }

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

    /// Whether a header-local position is part of the draggable title-bar background. Live
    /// buttons are deliberately excluded so pressing close never starts a window drag.
    pub fn is_drag_region(&self, position: PhysicalPosition<f64>) -> bool {
        let logical: LogicalPosition<f64> = position.to_logical(self.scale_factor);

        logical.x >= 0.0
            && logical.x < self.size.width as f64
            && logical.y >= 0.0
            && logical.y < self.size.height as f64
            && self.button_at(position).is_none()
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
/// `clip` is the horizontal span, in physical pixels, the text is allowed to occupy: a title too
/// long for its bar is cut off at the edge of that span rather than running on underneath the
/// buttons. Underneath is not hidden — a themed button is a circle or a rounded box, so glyphs
/// behind it show through the gaps around it, which is what made a long caption appear to spill
/// past the close button.
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
    clip: (f32, f32),
) -> Pixmap {
    let scaled_font = font.as_scaled(scale);
    let color = to_tiny_skia(color).to_color_u8();
    let color_alpha = color.alpha() as f32 / 255.0;

    let mut pixmap =
        Pixmap::new(size.width, size.height).expect("size is clamped to at least 1x1 in `new`");
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let clip_left = (clip.0.round() as i32).max(0);
    let clip_right = (clip.1.round() as i32).min(width);
    let data = pixmap.data_mut();

    for c in text.chars() {
        // Glyphs only ever advance rightwards, so once the pen is past the clip there is nothing
        // left to draw.
        if pen_x >= clip_right as f32 {
            break;
        }

        let glyph_id = scaled_font.glyph_id(c);
        let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(pen_x, pen_y));

        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();

            outlined.draw(|x, y, coverage| {
                let px = bounds.min.x as i32 + x as i32;
                let py = bounds.min.y as i32 + y as i32;

                if px >= clip_left && px < clip_right && py >= 0 && py < height {
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
mod tests;
