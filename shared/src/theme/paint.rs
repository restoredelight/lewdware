//! The vocabulary a theme is described in: how an area is filled, how a border is banded, how a
//! caption button and a dialog widget are painted. Pure description -- the per-theme values that
//! fill these in live in [`super::chrome`] and [`super::widgets`].

use serde::Serialize;

use super::{Appearance, Color, Face, TextAlign};

/// A colour from `#rrggbb` components, for building chrome palettes in `const`.
pub(super) const fn rgb8(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

pub(super) const BLACK: Color = rgb8(0, 0, 0);
pub(super) const WHITE: Color = rgb8(255, 255, 255);
/// A glyph colour meaning "draw nothing" — used where a mark only appears on hover.
pub(super) const TRANSPARENT: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// How an area of chrome is filled. Gradients are what `aqua` and `platinum` title bars need;
/// tiny-skia draws them natively, so they cost nothing to carry here.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub enum Fill {
    Solid(Color),
    VerticalGradient {
        from: Color,
        to: Color,
    },
    /// Alternating horizontal lines, `period` logical pixels apart — Mac OS 9's pinstriped title
    /// bar, which is most of what makes `platinum` recognisable.
    Pinstripe {
        base: Color,
        stripe: Color,
        period: u32,
    },
}

/// One one-logical-pixel ring of a window's border, outermost first.
///
/// `Bevel` is what makes a Win95-style 3D edge expressible: the top and left edges take one
/// colour and the bottom and right another, which a single flat colour per ring cannot express.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub enum BorderRing {
    Uniform(Color),
    Bevel {
        top_left: Color,
        bottom_right: Color,
    },
}

/// How a theme edges its *controls* — a dialog's buttons.
///
/// `Bevel` is the one thing `egui::Style` cannot express: `WidgetVisuals::bg_stroke` is a single
/// `Stroke`, uniform on all four sides, so a raised 3D edge — light on the top and left, dark on
/// the bottom and right — is inexpressible however the style is tuned. A theme asking for one is
/// painted by [`crate::window::layer::bevel`] instead of by egui.
///
/// Reuses [`BorderRing`] rather than a parallel type: a control's raised edge and a window's are
/// the same idea at different scales.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub enum WidgetEdge {
    /// egui paints it, from the stroke in the theme's `Style`.
    Flat,
    /// Painted here: rings from the outside in, one logical pixel each. `pressed` is drawn instead
    /// while the control is held, which is how a Win95 button appears to sink.
    Bevel {
        raised: &'static [BorderRing],
        pressed: &'static [BorderRing],
    },
}

/// How a dialog distinguishes the action activated by Enter from its other buttons.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub enum DefaultButtonStyle {
    /// Keep the normal themed face and reinforce its outer edge. Used by themes where a filled
    /// primary action would be anachronistic or needlessly loud.
    Outline(Stroke),
    /// A platform accent fill, with a separate response for hover and press.
    Filled {
        idle: Color,
        hover: Color,
        active: Color,
        text: Color,
        border: Stroke,
    },
}

/// A line: a colour and a width in logical pixels.
///
/// The theme-data stand-in for `egui::Stroke`, so that nothing in a theme's *definition* names an
/// egui type. [`Widgets::to_egui_style`] is the one place the two vocabularies meet.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Stroke {
    pub width: f32,
    pub color: Color,
}

impl Stroke {
    pub const NONE: Self = Self {
        width: 0.0,
        color: TRANSPARENT,
    };

    /// A hairline, which is what every theme that draws an edge at all asks for.
    pub const fn hairline(color: Color) -> Self {
        Self { width: 1.0, color }
    }

    pub(super) const fn new(width: f32, color: Color) -> Self {
        Self { width, color }
    }
}

/// How a control is painted in one interaction state.
///
/// Buttons and text fields share these: egui draws a `TextEdit` with the same widget visuals as a
/// button, which is why a theme that leaves a field unbordered has to say so on both.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct ControlPaint {
    pub fill: Color,
    pub border: Stroke,
}

impl ControlPaint {
    pub(super) const fn new(fill: Color, border: Stroke) -> Self {
        Self { fill, border }
    }
}

/// How a theme's controls are *shaped*, as distinct from coloured. Platforms differ far more in
/// padding and control height than in hue: this is what makes a Win95 button read as a Win95
/// button rather than an egui one in grey.
///
/// Every value is a whole number of logical points, deliberately. egui lays each widget out at the
/// running cursor, so a fractional gap puts every *subsequent* widget on a fractional coordinate —
/// which renders blurry, and which egui flags with an orange "Unaligned" marker in debug builds.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct WidgetMetrics {
    /// Horizontal and vertical padding inside a button.
    pub button_padding: (f32, f32),
    /// The minimum height of an interactive control.
    pub control_height: f32,
    /// Horizontal and vertical gap between adjacent controls.
    pub item_spacing: (f32, f32),
    /// One radius across every state. egui varies it slightly per state by default, which reads as
    /// wobble rather than intent once a theme has committed to a radius.
    pub corner_radius: u8,
}

/// The widget half of a theme: everything a dialog's buttons, text fields and labels are drawn
/// with, as plain data.
///
/// **Complete, not a diff.** Every value that can appear in a dialog is stated here, including the
/// ones egui would otherwise have supplied — text colour and caret being the two that used to be
/// inherited, differently, by half the catalogue. [`Self::to_egui_style`] still starts from
/// `egui::Visuals::light()`/`dark()`, but only as a substrate for the parts of egui we never draw
/// (window shadows, resize handles, hyperlinks); everything visible in one of our windows is set
/// from these fields. That is what makes the data, rather than an `egui::Style`, the theme.
///
/// Being egui-free, it also serialises — which is how `config/` can show a user what a theme looks
/// like without linking the engine.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Widgets {
    /// Which side of the light/dark divide this palette belongs to. Not redundant with the
    /// requested [`Appearance`]: `platinum` has no dark variant, so its dark request lands here as
    /// `Light`, and a reader of this data should be told what it actually got.
    pub base: Appearance,
    /// The dialog's own background. A Win95 dialog *is* button-face grey; leaving egui's near-white
    /// behind period chrome looked wrong and left some themes' buttons within a few levels of the
    /// surface they sat on. Overridden in turn by a window's `background_color`.
    pub panel: Color,
    /// Every piece of text in the window: labels, button captions, field contents. One colour
    /// rather than egui's per-state grey ramp — a dialog that darkens its own label text when the
    /// pointer crosses a button is egui's default, not any platform's behaviour.
    pub text: Color,
    /// The text-editing caret.
    pub caret: Stroke,
    /// A text field's interior, which is the one surface that is *not* the panel or a control.
    pub field: Color,
    /// The highlight behind selected text, and the colour of the text on top of it — which egui
    /// takes from the same `Selection::stroke` it uses for a focused field's outline. Both have to
    /// be set together: a saturated brand-colour highlight under egui's default text colour is
    /// unreadable, which is what every theme had. The fills are therefore tints rather than the
    /// platforms' full accent colours, since egui has one text colour to spend on both.
    pub selection: Color,
    pub selection_text: Color,
    /// At rest. Also used for the non-interactive and open states, which a dialog never shows in a
    /// way a user could tell apart.
    pub idle: ControlPaint,
    /// Under the pointer, and while held. Both are deliberately louder than the platforms they
    /// imitate: Windows 11 and Adwaita shift a control by a handful of levels on hover, and a
    /// macOS push button has no hover state at all — authentic, and useless here. These popups
    /// arrive in numbers and the user's usual goal is to dismiss them, so a control has to visibly
    /// answer the pointer. `every_theme_answers_the_pointer_visibly` holds the catalogue to a floor.
    pub hover: ControlPaint,
    pub pressed: ControlPaint,
    pub metrics: WidgetMetrics,
    /// The UI face and its size in logical points. Never varies with appearance — a palette swap
    /// is not a typeface change — which `a_font_never_depends_on_the_palette` pins.
    pub font: Face,
    pub font_size: f32,
    pub edge: WidgetEdge,
    pub default_button: DefaultButtonStyle,
}

/// The outline a chrome button is filled within.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonShape {
    /// A full-height rectangle, as on Windows.
    Rect,
    /// A square centred vertically in the title bar, as on classic Mac OS.
    Square,
    /// A circle centred in the header, as on macOS.
    Circle,
}

/// The mark drawn on top of a chrome button's fill. *Which* mark — how big it is drawn is
/// [`Button::glyph_ratio`], a per-theme number rather than a variant here.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Cross,
    /// The inset outline in Mac OS 8/9's close box.
    Square,
    /// Nothing — an inert button, or one whose glyph only appears on hover.
    None,
}

/// What pressing a chrome button does.
///
/// `Inert` exists only so a platform's silhouette can be completed honestly: macOS keeps its
/// minimise and zoom lights in place, uncoloured, on a window that can do neither. Such a button
/// never reacts to the pointer and never activates — see `design/window-themes.md`, "decorations
/// never lie about function". A *coloured* button that did nothing would be the violation.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonAction {
    Close,
    Inert,
}

/// One chrome button's paint in one pointer state.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct ButtonPaint {
    pub fill: Fill,
    pub glyph: Color,
    /// Optional outer rim for circular chrome controls. Rectangular buttons fill their whole slot
    /// and leave their edge to the window theme's border vocabulary.
    pub rim: Option<Color>,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Button {
    pub action: ButtonAction,
    pub shape: ButtonShape,
    pub glyph: Glyph,
    /// Width as a multiple of the header height, so buttons scale with the title bar.
    pub width_ratio: f32,
    /// Diameter of a circular button's painted disc as a multiple of the header height.
    ///
    /// Usually this equals [`Self::width_ratio`]. Adwaita is the important exception: GTK gives
    /// its 24px close disc a wider transparent button slot, so the title clearance and hit target
    /// are larger than the thing a user sees.
    pub diameter_ratio: f32,
    /// How far the glyph reaches from the button's centre on each axis, as a fraction of the
    /// button's own extent — so a cross spans `2 * glyph_ratio` of the button it marks.
    ///
    /// Per theme, because the platforms genuinely differ: macOS's traffic-light cross spans half
    /// its light, GNOME's is nearer two fifths of a much larger button, and Windows' rectangular
    /// caption buttons are a third. Encoding that as a shared constant keyed on [`Glyph`] is what
    /// previously chained the three together — sizing one platform's mark correctly silently
    /// resized the others, twice.
    pub glyph_ratio: f32,
    pub idle: ButtonPaint,
    /// Ignored for [`ButtonAction::Inert`], which never leaves `idle`.
    pub hover: ButtonPaint,
    pub active: ButtonPaint,
}

/// Which end of the title bar the buttons sit at, and how they are spaced.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Buttons {
    pub side: Side,
    /// Logical pixels between the outer edge and the first button.
    pub inset: f32,
    /// Logical pixels between adjacent buttons.
    pub gap: f32,
    /// In visual order from `side` inwards.
    pub buttons: &'static [Button],
    /// How the cluster is drawn on a window that cannot be closed (`closeable = false`).
    ///
    /// `None` — the default for most themes — draws no cluster at all. `Some(paint)` draws every
    /// button in the cluster in that one paint, hit-testing none of them: the platform's own
    /// *disabled* control, which says "this window cannot be closed" rather than leaving the user
    /// to discover it by clicking. See `design/window-themes.md`, "Aqua's traffic lights, and the
    /// function-honesty rule" — a greyed-out control is the opposite of a decoration that lies,
    /// so this is opt-in per theme, and only for the platforms that really do grey their caption
    /// buttons (Windows and macOS) rather than dropping them (GNOME, KDE, Mac OS 9).
    pub unclosable: Option<ButtonPaint>,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct TitleStyle {
    pub font: Face,
    /// Logical points.
    pub size: f32,
    pub color: Color,
    /// Logical pixels kept clear at the ends of the title bar.
    pub padding: f32,
    /// Where the title sits along the bar. A real per-platform difference: Windows and GNOME
    /// dialogs align left, macOS centres. `Center` falls back to the near edge when the text is
    /// too wide to centre without colliding with the buttons.
    pub align: TextAlign,
}

/// Everything a theme paints outside the content area.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
pub struct Chrome {
    pub header: Fill,
    /// Outermost ring first. Its length is the border width in logical pixels, and must equal
    /// [`Metrics::border_width`] — a test holds every theme to that.
    pub border: &'static [BorderRing],
    /// A hairline drawn along the bottom edge of the title bar, in logical pixels.
    ///
    /// Only for the themes whose platform draws one. A flat theme whose bar is the same colour as
    /// the panel below it (`breeze`, `fluent`) has nothing else to mark where the chrome ends —
    /// on a real desktop the window's drop shadow and its non-uniform content do that work, and
    /// this engine has neither. `None` for every theme whose bar is already a different colour,
    /// or whose platform genuinely runs the two together.
    pub separator: Option<Color>,
    pub title: TitleStyle,
    pub buttons: Buttons,
}

/// The physical band `[start, end)` that ring `index` of `count` occupies within a border
/// `thickness` physical pixels thick.
///
/// Rings are one *logical* pixel each, so at a fractional scale factor they cannot each be a whole
/// number of physical pixels. Dividing the measured total keeps the rings adjacent and exactly
/// filling it, which is what matters: any gap would show the window's background between the
/// border and the content.
pub fn ring_band(index: usize, count: usize, thickness: u32) -> (u32, u32) {
    debug_assert!(index < count, "ring {index} out of range for {count} rings");

    let at = |i: usize| ((i as u64 * thickness as u64) / count as u64) as u32;
    (at(index), at(index + 1))
}

impl BorderRing {
    /// The colour of this ring's top and left edges.
    pub fn top_left(self) -> Color {
        match self {
            Self::Uniform(color) => color,
            Self::Bevel { top_left, .. } => top_left,
        }
    }

    /// The colour of this ring's bottom and right edges.
    pub fn bottom_right(self) -> Color {
        match self {
            Self::Uniform(color) => color,
            Self::Bevel { bottom_right, .. } => bottom_right,
        }
    }
}

/// A theme's geometry, in **logical** pixels.
///
/// Appearance (light/dark) must never change these — see `design/window-themes.md`'s invariance
/// rule. That is what lets appearance be resolved after a window already exists, while metrics
/// have to be known at creation time because they set the outer size.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    pub header_height: u32,
    pub border_width: u32,
}

impl Metrics {
    pub const PLAIN: Self = Self {
        header_height: 20,
        border_width: 1,
    };

    /// The extra logical pixels a decorated window needs beyond its content, as
    /// `(width, height)`. Borders on both sides; the header only above.
    pub fn outer_padding(&self) -> (u32, u32) {
        (
            self.border_width * 2,
            self.border_width * 2 + self.header_height,
        )
    }
}
