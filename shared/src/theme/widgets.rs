//! One [`Widgets`] per theme per palette: the dialog controls each named look is made of.
//!
//! Values carried over from when this half was built by mutating an `egui::Style`, with two
//! exceptions that were previously inherited from egui and are now each theme's own: `text`
//! (egui's #505050/#8c8c8c body grey, and the ramp that darkened labels under the pointer) and
//! `caret` (egui's #00537d/#c0dbf5 accent, which belonged to no theme here).

use super::chrome::*;
use super::paint::*;
use super::{Appearance, Face};

/// Every retro face wants a hairline caret; egui's default 2pt bar is a modern affectation.
pub(super) const PERIOD_CARET: f32 = 1.0;
pub(super) const MODERN_CARET: f32 = 2.0;

pub(super) const PLAIN_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(245, 245, 245),
    text: rgb8(26, 26, 26),
    caret: Stroke::new(MODERN_CARET, rgb8(26, 26, 26)),
    field: WHITE,
    selection: rgb8(200, 200, 200),
    selection_text: BLACK,
    idle: ControlPaint::new(WHITE, Stroke::hairline(rgb8(48, 48, 48))),
    hover: ControlPaint::new(rgb8(220, 220, 220), Stroke::hairline(rgb8(48, 48, 48))),
    pressed: ControlPaint::new(rgb8(180, 180, 180), Stroke::hairline(rgb8(48, 48, 48))),
    metrics: WidgetMetrics {
        button_padding: (8.0, 3.0),
        control_height: 22.0,
        item_spacing: (7.0, 5.0),
        corner_radius: 0,
    },
    font: Face::Default,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    // A monochrome inversion rather than an accent: `plain` imitates no platform, so it has no
    // accent colour to borrow.
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(45, 45, 45),
        hover: rgb8(70, 70, 70),
        active: rgb8(105, 105, 105),
        text: WHITE,
        border: Stroke::NONE,
    },
};

pub(super) const PLAIN_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(24, 24, 24),
    text: rgb8(240, 240, 240),
    caret: Stroke::new(MODERN_CARET, rgb8(240, 240, 240)),
    field: rgb8(13, 13, 13),
    selection: rgb8(89, 89, 89),
    selection_text: WHITE,
    idle: ControlPaint::new(rgb8(51, 51, 51), Stroke::hairline(rgb8(160, 160, 160))),
    hover: ControlPaint::new(rgb8(77, 77, 77), Stroke::hairline(rgb8(160, 160, 160))),
    pressed: ControlPaint::new(rgb8(102, 102, 102), Stroke::hairline(rgb8(160, 160, 160))),
    metrics: PLAIN_WIDGETS.metrics,
    font: Face::Default,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(235, 235, 235),
        hover: rgb8(210, 210, 210),
        active: rgb8(175, 175, 175),
        text: BLACK,
        border: Stroke::NONE,
    },
};

pub(super) const FLUENT_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(243, 243, 243),
    // Windows 11's own body text, which is near-black rather than the pure black of Win95.
    text: rgb8(27, 27, 27),
    caret: Stroke::new(MODERN_CARET, rgb8(27, 27, 27)),
    field: WHITE,
    selection: rgb8(204, 228, 247),
    selection_text: rgb8(0, 62, 107),
    // Windows 11 outlines every control, including at rest — that hairline is a lot of why its
    // buttons look like buttons rather than flat tinted rectangles.
    idle: ControlPaint::new(rgb8(251, 251, 251), Stroke::hairline(rgb8(209, 209, 209))),
    hover: ControlPaint::new(rgb8(229, 229, 229), Stroke::hairline(rgb8(209, 209, 209))),
    pressed: ControlPaint::new(rgb8(213, 213, 213), Stroke::hairline(rgb8(209, 209, 209))),
    metrics: WidgetMetrics {
        button_padding: (11.0, 6.0),
        control_height: 32.0,
        item_spacing: (8.0, 6.0),
        corner_radius: 4,
    },
    font: Face::Selawik,
    font_size: 14.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(0, 103, 192),
        hover: rgb8(25, 117, 197),
        active: rgb8(0, 90, 158),
        text: WHITE,
        border: Stroke::hairline(rgb8(0, 90, 158)),
    },
};

pub(super) const FLUENT_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(32, 32, 32),
    text: WHITE,
    caret: Stroke::new(MODERN_CARET, WHITE),
    field: rgb8(31, 31, 31),
    selection: rgb8(31, 58, 92),
    selection_text: rgb8(207, 230, 255),
    idle: ControlPaint::new(rgb8(45, 45, 45), Stroke::hairline(rgb8(66, 66, 66))),
    hover: ControlPaint::new(rgb8(68, 68, 68), Stroke::hairline(rgb8(66, 66, 66))),
    pressed: ControlPaint::new(rgb8(82, 82, 82), Stroke::hairline(rgb8(66, 66, 66))),
    metrics: FLUENT_WIDGETS.metrics,
    font: Face::Selawik,
    font_size: 14.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(0, 95, 158),
        hover: rgb8(18, 112, 178),
        active: rgb8(0, 78, 130),
        text: WHITE,
        border: Stroke::hairline(rgb8(0, 70, 117)),
    },
};

pub(super) const REDMOND_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    // Win95 dialogs are the same grey as their buttons; the buttons are told apart by their
    // border, not their fill.
    panel: REDMOND_FACE,
    text: BLACK,
    caret: Stroke::new(PERIOD_CARET, BLACK),
    field: WHITE,
    selection: rgb8(195, 207, 230),
    selection_text: rgb8(0, 0, 128),
    // Square, flat, and a hard edge on every state: Win95 controls have no radius and no hover
    // fill of their own, only a bevel — which `edge` paints.
    idle: ControlPaint::new(REDMOND_FACE, Stroke::hairline(rgb8(128, 128, 128))),
    hover: ControlPaint::new(rgb8(214, 210, 202), Stroke::hairline(BLACK)),
    pressed: ControlPaint::new(rgb8(160, 160, 160), Stroke::hairline(BLACK)),
    metrics: WidgetMetrics {
        button_padding: (8.0, 3.0),
        control_height: 21.0,
        item_spacing: (6.0, 4.0),
        corner_radius: 0,
    },
    font: Face::Pixel,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: REDMOND_RAISED,
        pressed: REDMOND_PRESSED,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(BLACK)),
};

pub(super) const REDMOND_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: REDMOND_PANEL_DARK,
    text: rgb8(230, 225, 245),
    caret: Stroke::new(PERIOD_CARET, rgb8(230, 225, 245)),
    field: rgb8(34, 31, 39),
    selection: rgb8(74, 66, 96),
    selection_text: rgb8(230, 225, 245),
    idle: ControlPaint::new(REDMOND_FACE_DARK, Stroke::hairline(REDMOND_SHADOW_DARK)),
    hover: ControlPaint::new(rgb8(137, 132, 148), Stroke::hairline(BLACK)),
    pressed: ControlPaint::new(rgb8(82, 77, 92), Stroke::hairline(BLACK)),
    metrics: REDMOND_WIDGETS.metrics,
    font: Face::Pixel,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: REDMOND_RAISED_DARK,
        pressed: REDMOND_PRESSED_DARK,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(REDMOND_GLYPH_DARK)),
};

pub(super) const AQUA_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(236, 236, 236),
    text: rgb8(29, 29, 31),
    caret: Stroke::new(MODERN_CARET, rgb8(29, 29, 31)),
    // Aqua has no borders to find a control by, so the text field has to be told apart from the
    // dialog by its fill alone — clearly lighter here, and clearly darker in dark mode, as macOS
    // does.
    field: WHITE,
    selection: rgb8(180, 213, 254),
    selection_text: rgb8(11, 61, 145),
    idle: ControlPaint::new(WHITE, Stroke::NONE),
    hover: ControlPaint::new(rgb8(232, 232, 232), Stroke::NONE),
    // Accent belongs to the primary action. An ordinary push button darkens neutrally while held
    // so it responds without briefly claiming primary status.
    pressed: ControlPaint::new(rgb8(205, 205, 207), Stroke::NONE),
    metrics: WidgetMetrics {
        button_padding: (13.0, 4.0),
        control_height: 24.0,
        item_spacing: (10.0, 6.0),
        // Modern macOS rounds controls generously without turning ordinary dialog fields and push
        // buttons into the full capsules used by iOS.
        corner_radius: 6,
    },
    font: Face::Inter,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(0, 122, 255),
        hover: rgb8(20, 132, 255),
        active: rgb8(0, 96, 205),
        text: WHITE,
        border: Stroke::NONE,
    },
};

pub(super) const AQUA_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(50, 50, 52),
    text: rgb8(245, 245, 247),
    caret: Stroke::new(MODERN_CARET, rgb8(245, 245, 247)),
    field: rgb8(28, 28, 30),
    selection: rgb8(44, 74, 110),
    selection_text: rgb8(207, 226, 255),
    idle: ControlPaint::new(rgb8(86, 86, 88), Stroke::NONE),
    hover: ControlPaint::new(rgb8(110, 110, 112), Stroke::NONE),
    pressed: ControlPaint::new(rgb8(75, 75, 78), Stroke::NONE),
    metrics: AQUA_WIDGETS.metrics,
    font: Face::Inter,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(10, 132, 255),
        hover: rgb8(40, 146, 255),
        active: rgb8(0, 105, 220),
        text: WHITE,
        border: Stroke::NONE,
    },
};

pub(super) const ADWAITA_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(250, 250, 251),
    // Cantarell in egui's generic mid-grey made ordinary GNOME content look insensitive. Adwaita's
    // own foreground tones instead; weak text (including placeholders) stays muted independently.
    text: ADWAITA_TEXT,
    caret: Stroke::new(MODERN_CARET, ADWAITA_TEXT),
    field: WHITE,
    selection: rgb8(197, 221, 246),
    selection_text: rgb8(20, 84, 140),
    // Darker than the headerbar-grey panel behind it: Adwaita's own dialog buttons are roughly 10%
    // black over the window background rather than the near-white tried first, which left them a
    // couple of levels from the surface and invisible. The hairline matters for the same reason —
    // egui draws text fields with the same visuals as buttons, and an unbordered white field on a
    // near-white panel cannot be found at all.
    idle: ControlPaint::new(rgb8(235, 235, 236), Stroke::hairline(rgb8(211, 211, 213))),
    hover: ControlPaint::new(rgb8(211, 211, 213), Stroke::hairline(rgb8(194, 194, 196))),
    pressed: ControlPaint::new(rgb8(190, 190, 192), Stroke::hairline(rgb8(178, 178, 181))),
    metrics: WidgetMetrics {
        button_padding: (14.0, 7.0),
        control_height: 34.0,
        item_spacing: (8.0, 6.0),
        // Libadwaita rounds everything to the same generous radius.
        corner_radius: 8,
    },
    font: Face::Cantarell,
    font_size: 14.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(53, 132, 228),
        hover: rgb8(70, 145, 232),
        active: rgb8(38, 112, 204),
        text: WHITE,
        border: Stroke::NONE,
    },
};

pub(super) const ADWAITA_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(34, 34, 38),
    text: WHITE,
    caret: Stroke::new(MODERN_CARET, WHITE),
    field: rgb8(29, 29, 32),
    selection: rgb8(38, 69, 107),
    selection_text: rgb8(214, 230, 250),
    idle: ControlPaint::new(rgb8(56, 56, 56), Stroke::hairline(rgb8(74, 74, 74))),
    hover: ControlPaint::new(rgb8(79, 79, 79), Stroke::hairline(rgb8(74, 74, 74))),
    pressed: ControlPaint::new(rgb8(96, 96, 96), Stroke::hairline(rgb8(74, 74, 74))),
    metrics: ADWAITA_WIDGETS.metrics,
    font: Face::Cantarell,
    font_size: 14.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(28, 113, 216),
        hover: rgb8(48, 128, 222),
        active: rgb8(20, 92, 180),
        text: WHITE,
        border: Stroke::NONE,
    },
};

pub(super) const BREEZE_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: rgb8(239, 240, 241),
    text: rgb8(35, 38, 41),
    caret: Stroke::new(MODERN_CARET, rgb8(35, 38, 41)),
    field: WHITE,
    selection: rgb8(183, 225, 247),
    selection_text: rgb8(0, 82, 120),
    idle: ControlPaint::new(rgb8(252, 252, 252), Stroke::hairline(rgb8(189, 195, 199))),
    hover: ControlPaint::new(rgb8(225, 228, 230), Stroke::hairline(rgb8(189, 195, 199))),
    pressed: ControlPaint::new(rgb8(207, 212, 215), Stroke::hairline(rgb8(189, 195, 199))),
    metrics: WidgetMetrics {
        button_padding: (10.0, 5.0),
        control_height: 30.0,
        item_spacing: (8.0, 6.0),
        corner_radius: 3,
    },
    font: Face::NotoSans,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    default_button: DefaultButtonStyle::Filled {
        idle: rgb8(61, 174, 233),
        hover: rgb8(83, 184, 236),
        active: rgb8(41, 143, 197),
        text: WHITE,
        border: Stroke::NONE,
    },
};

pub(super) const BREEZE_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: rgb8(32, 35, 38),
    text: rgb8(252, 252, 252),
    caret: Stroke::new(MODERN_CARET, rgb8(252, 252, 252)),
    field: rgb8(20, 22, 24),
    selection: rgb8(38, 88, 112),
    selection_text: rgb8(224, 246, 255),
    idle: ControlPaint::new(rgb8(41, 44, 48), Stroke::hairline(rgb8(97, 102, 107))),
    hover: ControlPaint::new(rgb8(82, 90, 98), Stroke::hairline(rgb8(97, 102, 107))),
    pressed: ControlPaint::new(rgb8(98, 108, 116), Stroke::hairline(rgb8(97, 102, 107))),
    metrics: BREEZE_WIDGETS.metrics,
    font: Face::NotoSans,
    font_size: 13.0,
    edge: WidgetEdge::Flat,
    // Breeze's accent is the same in both palettes — the blue is the identity, not a light-mode
    // choice.
    default_button: BREEZE_WIDGETS.default_button,
};

/// Mac OS 9 has no dark counterpart, so this is what a dark request resolves to as well.
pub(super) const PLATINUM_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: PLATINUM_FACE,
    text: BLACK,
    caret: Stroke::new(PERIOD_CARET, BLACK),
    field: WHITE,
    selection: rgb8(198, 208, 224),
    selection_text: rgb8(0, 0, 128),
    idle: ControlPaint::new(PLATINUM_FACE, Stroke::hairline(rgb8(85, 85, 85))),
    // Darkens on hover rather than lightening, which leaves room for the press state.
    hover: ControlPaint::new(rgb8(198, 198, 198), Stroke::hairline(rgb8(85, 85, 85))),
    pressed: ControlPaint::new(rgb8(170, 170, 170), Stroke::hairline(rgb8(85, 85, 85))),
    metrics: WidgetMetrics {
        button_padding: (10.0, 3.0),
        control_height: 20.0,
        item_spacing: (6.0, 4.0),
        // Square, not the 3pt tried first: `bevel::button` paints a bevel on square corners, and a
        // rounded fill under square edges reads as a mistake. Mac OS 9's own controls are
        // near-square anyway.
        corner_radius: 0,
    },
    font: Face::SourceSans,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: PLATINUM_RAISED,
        pressed: PLATINUM_PRESSED,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(BLACK)),
};

pub(super) const CDE_WIDGETS: Widgets = Widgets {
    base: Appearance::Light,
    panel: CDE_FACE,
    text: BLACK,
    caret: Stroke::new(PERIOD_CARET, BLACK),
    field: WHITE,
    selection: rgb8(175, 205, 202),
    selection_text: rgb8(20, 72, 70),
    idle: ControlPaint::new(CDE_FACE, Stroke::hairline(CDE_SHADOW)),
    hover: ControlPaint::new(rgb8(205, 205, 194), Stroke::hairline(CDE_SHADOW)),
    pressed: ControlPaint::new(rgb8(151, 151, 143), Stroke::hairline(CDE_SHADOW)),
    metrics: WidgetMetrics {
        button_padding: (9.0, 3.0),
        control_height: 22.0,
        item_spacing: (6.0, 4.0),
        corner_radius: 0,
    },
    font: Face::LiberationSans,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: CDE_RAISED,
        pressed: CDE_PRESSED,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(BLACK)),
};

pub(super) const CDE_WIDGETS_DARK: Widgets = Widgets {
    base: Appearance::Dark,
    panel: CDE_PANEL_DARK,
    text: rgb8(235, 240, 232),
    caret: Stroke::new(PERIOD_CARET, rgb8(235, 240, 232)),
    field: rgb8(30, 33, 32),
    selection: rgb8(51, 93, 90),
    selection_text: rgb8(232, 246, 241),
    idle: ControlPaint::new(CDE_FACE_DARK, Stroke::hairline(CDE_SHADOW_DARK)),
    hover: ControlPaint::new(rgb8(111, 119, 115), Stroke::hairline(CDE_SHADOW_DARK)),
    pressed: ControlPaint::new(rgb8(67, 72, 70), Stroke::hairline(CDE_SHADOW_DARK)),
    metrics: CDE_WIDGETS.metrics,
    font: Face::LiberationSans,
    font_size: 12.0,
    edge: WidgetEdge::Bevel {
        raised: CDE_RAISED_DARK,
        pressed: CDE_PRESSED_DARK,
    },
    default_button: DefaultButtonStyle::Outline(Stroke::hairline(rgb8(235, 240, 232))),
};
