//! One [`Chrome`] per theme per palette: the border, title bar and caption buttons each named
//! look is made of.
//!
//! A window's border, title bar and buttons, as the vocabulary `design/window-themes.md` settled
//! on: gradients and pinstripes for the two Mac themes, multi-ring bevels for the period ones,
//! circular and left-side button clusters for `aqua`/`platinum`. The engine paints these with
//! tiny-skia in `header.rs`; `config/` draws the same values as CSS.

use super::paint::*;
use super::{Color, Face, TextAlign};

/// Glyph sizes, as a fraction of the button's extent. Each is the platform's own proportion,
/// measured against a real window; see [`Button::glyph_ratio`] for why they are separate numbers.
///
/// A rectangular caption button (Windows, Mac OS 9, CDE) puts a small mark in a large slot.
pub(super) const RECT_GLYPH: f32 = 1.0 / 6.0;
/// macOS: the cross spans half its traffic light.
pub(super) const AQUA_GLYPH: f32 = 0.25;
/// KDE Breeze: a slightly more open mark than macOS's, in a slightly larger circle.
pub(super) const BREEZE_GLYPH: f32 = 0.2357;
/// GNOME: `window-close-symbolic` in a 24px circle leaves visible margin all round -- the mark is
/// nearer two fifths of the button than the half macOS draws.
pub(super) const ADWAITA_GLYPH: f32 = 0.2;

/// A close button that fills its corner of the bar, Windows-style.
pub(super) const fn rect_close(
    width_ratio: f32,
    idle: ButtonPaint,
    hover: ButtonPaint,
    active: ButtonPaint,
) -> Button {
    Button {
        action: ButtonAction::Close,
        shape: ButtonShape::Rect,
        glyph: Glyph::Cross,
        width_ratio,
        diameter_ratio: width_ratio,
        glyph_ratio: RECT_GLYPH,
        idle,
        hover,
        active,
    }
}

pub(super) const fn paint(fill: Color, glyph: Color) -> ButtonPaint {
    ButtonPaint {
        fill: Fill::Solid(fill),
        glyph,
        rim: None,
    }
}

pub(super) const fn rimmed(fill: Fill, glyph: Color, rim: Color) -> ButtonPaint {
    ButtonPaint {
        fill,
        glyph,
        rim: Some(rim),
    }
}

// plain. Deliberately the plainest thing in the catalogue: greyscale, hairline, square, a short bar
// and a small square close button. It imitates no platform, which is the point — chrome that behaves
// as furniture and does not compete with whatever art a pack puts inside it, a job none of the
// platform themes can do. Its one flourish is the loud red close button kept from the engine's
// original look: when windows are stacked in numbers, a close target that shouts is a feature.
pub(super) const PLAIN_HEADER: Color = rgb8(232, 232, 232);
pub(super) const PLAIN_CLOSE_HOVER: Color = rgb8(255, 0, 0);
pub(super) const PLAIN_CLOSE_ACTIVE: Color = rgb8(230, 0, 0);

/// `plain`'s close button: square rather than the wide slab a Windows caption uses, and monochrome
/// until the pointer reaches it.
pub(super) const fn plain_close(bar: Color, glyph: Color) -> Button {
    Button {
        action: ButtonAction::Close,
        shape: ButtonShape::Rect,
        glyph: Glyph::Cross,
        width_ratio: 1.0,
        diameter_ratio: 1.0,
        glyph_ratio: RECT_GLYPH,
        idle: ButtonPaint {
            fill: Fill::Solid(bar),
            glyph,
            rim: None,
        },
        hover: ButtonPaint {
            fill: Fill::Solid(PLAIN_CLOSE_HOVER),
            glyph: WHITE,
            rim: None,
        },
        active: ButtonPaint {
            fill: Fill::Solid(PLAIN_CLOSE_ACTIVE),
            glyph: WHITE,
            rim: None,
        },
    }
}

pub(super) const PLAIN_CHROME: Chrome = Chrome {
    header: Fill::Solid(PLAIN_HEADER),
    border: &[BorderRing::Uniform(BLACK)],
    separator: None,
    title: TitleStyle {
        font: Face::Default,
        size: 12.0,
        color: BLACK,
        padding: 8.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 0.0,
        gap: 0.0,
        buttons: &[plain_close(PLAIN_HEADER, BLACK)],
        unclosable: None,
    },
};

// ── The catalogue ────────────────────────────────────────────────────────────────
//
// One concrete look each. Light only for now; appearance (light/dark) is a separate axis, and the
// invariance rule means it can be added without touching any of the metrics below.

// fluent — Windows 11. Today's chrome, corrected: a lighter bar and Windows' actual close red
// (#C42B1C) rather than pure red. Square corners; see `design/window-themes.md` on why rounding is
// not expressible yet.
// A shade lighter than `FLUENT_WIDGETS.panel` (#f3f3f3), which is how a real Win32 window under
// Windows 11 reads in light mode: a near-white caption over a slightly grey client area. Windows
// draws no separator line, so this tonal step is the only thing marking the bar — matching the
// platform, and the reason `fluent` does not take the hairline `breeze` does.
pub(super) const FLUENT_HEADER: Color = rgb8(251, 251, 251);
pub(super) const FLUENT_TEXT: Color = rgb8(26, 26, 26);

pub(super) const FLUENT_CHROME: Chrome = Chrome {
    header: Fill::Solid(FLUENT_HEADER),
    border: &[BorderRing::Uniform(rgb8(180, 180, 180))],
    separator: None,
    title: TitleStyle {
        font: Face::Selawik,
        size: 12.0,
        color: FLUENT_TEXT,
        // With no icon, Microsoft's title-bar guidance places the caption 16px from the edge.
        padding: 16.0,
        // Windows puts its title at the left, unlike the centred plain bar.
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 0.0,
        gap: 0.0,
        buttons: &[rect_close(
            // Windows reserves 46px for a caption control in its standard 32px title bar.
            46.0 / 32.0,
            paint(FLUENT_HEADER, FLUENT_TEXT),
            paint(rgb8(196, 43, 28), WHITE),
            paint(rgb8(168, 36, 25), WHITE),
        )],
        // Windows keeps the caption button and greys its glyph rather than dropping it.
        unclosable: Some(paint(FLUENT_HEADER, rgb8(155, 155, 155))),
    },
};

// redmond — Windows 95/98. A three-ring raised bevel, a solid navy bar, and the bundled W95FA.
pub(super) const REDMOND_FACE: Color = rgb8(192, 192, 192);
pub(super) const REDMOND_NAVY: Color = rgb8(0, 0, 128);

pub(super) const REDMOND_CHROME: Chrome = Chrome {
    header: Fill::Solid(REDMOND_NAVY),
    // Light on the top-left, dark on the bottom-right: the 3D raised edge, outermost ring first.
    border: &[
        BorderRing::Bevel {
            top_left: rgb8(223, 223, 223),
            bottom_right: BLACK,
        },
        BorderRing::Bevel {
            top_left: WHITE,
            bottom_right: rgb8(128, 128, 128),
        },
        BorderRing::Uniform(REDMOND_FACE),
    ],
    separator: None,
    title: TitleStyle {
        font: Face::Pixel,
        size: 12.0,
        color: WHITE,
        padding: 3.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 2.0,
        gap: 0.0,
        // Roughly square, as Win95's caption buttons are. Hover only lightens: these buttons had
        // no hover state at all, but a close target with no feedback is worse than a mild one.
        buttons: &[rect_close(
            0.9,
            paint(REDMOND_FACE, BLACK),
            paint(rgb8(212, 208, 200), BLACK),
            paint(rgb8(160, 160, 160), BLACK),
        )],
        // The greyed-out close box on a Win95 dialog that will not be dismissed -- a setup wizard
        // mid-copy, a modal progress box. The button face stays; only the glyph goes to the
        // system's disabled grey.
        unclosable: Some(paint(REDMOND_FACE, rgb8(128, 128, 128))),
    },
};

// The Win95 control edge, outermost ring first. Light on the top and left, dark on the bottom and
// right, which is what makes it read as raised; the pressed pair is the same rings inverted, so the
// control appears to sink under the pointer rather than merely change colour.
pub(super) const REDMOND_RAISED: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: rgb8(223, 223, 223),
        bottom_right: BLACK,
    },
    BorderRing::Bevel {
        top_left: WHITE,
        bottom_right: rgb8(128, 128, 128),
    },
];
pub(super) const REDMOND_PRESSED: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: BLACK,
        bottom_right: rgb8(223, 223, 223),
    },
    BorderRing::Bevel {
        top_left: rgb8(128, 128, 128),
        bottom_right: WHITE,
    },
];

pub(super) const REDMOND_RAISED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: REDMOND_HIGHLIGHT_DARK,
        bottom_right: BLACK,
    },
    BorderRing::Bevel {
        top_left: REDMOND_BRIGHT_DARK,
        bottom_right: REDMOND_SHADOW_DARK,
    },
];
pub(super) const REDMOND_PRESSED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: BLACK,
        bottom_right: REDMOND_HIGHLIGHT_DARK,
    },
    BorderRing::Bevel {
        top_left: REDMOND_SHADOW_DARK,
        bottom_right: REDMOND_BRIGHT_DARK,
    },
];

// Mac OS 9's controls are outlined in a hard grey with a single highlight inside it.
pub(super) const PLATINUM_RAISED: &[BorderRing] = &[
    BorderRing::Uniform(rgb8(85, 85, 85)),
    BorderRing::Bevel {
        top_left: WHITE,
        bottom_right: rgb8(170, 170, 170),
    },
];
pub(super) const PLATINUM_PRESSED: &[BorderRing] = &[
    BorderRing::Uniform(rgb8(85, 85, 85)),
    BorderRing::Bevel {
        top_left: rgb8(170, 170, 170),
        bottom_right: WHITE,
    },
];

// aqua — macOS. Left-side traffic lights, a centred title, a gently graded bar. Minimise and zoom
// are present but inert and uncoloured, which is what macOS itself does on a window that can do
// neither — see `design/window-themes.md`, "decorations never lie about function".
pub(super) const AQUA_DISABLED: Color = rgb8(213, 213, 213);
pub(super) const AQUA_LIGHT_RATIO: f32 = 0.43;

pub(super) const fn aqua_disabled_light() -> ButtonPaint {
    rimmed(
        Fill::VerticalGradient {
            from: rgb8(226, 226, 226),
            to: AQUA_DISABLED,
        },
        TRANSPARENT,
        rgb8(190, 190, 190),
    )
}

pub(super) const fn aqua_red(fill_top: Color, fill_bottom: Color, glyph: Color) -> ButtonPaint {
    rimmed(
        Fill::VerticalGradient {
            from: fill_top,
            to: fill_bottom,
        },
        glyph,
        rgb8(218, 72, 66),
    )
}

/// One of aqua's inert lights: flat grey, no glyph, no reaction to the pointer.
pub(super) const fn aqua_inert() -> Button {
    Button {
        action: ButtonAction::Inert,
        shape: ButtonShape::Circle,
        glyph: Glyph::None,
        width_ratio: AQUA_LIGHT_RATIO,
        diameter_ratio: AQUA_LIGHT_RATIO,
        glyph_ratio: AQUA_GLYPH,
        idle: aqua_disabled_light(),
        hover: aqua_disabled_light(),
        active: aqua_disabled_light(),
    }
}

pub(super) const AQUA_CHROME: Chrome = Chrome {
    header: Fill::VerticalGradient {
        from: rgb8(246, 246, 246),
        to: rgb8(232, 232, 232),
    },
    border: &[BorderRing::Uniform(rgb8(176, 176, 176))],
    separator: None,
    title: TitleStyle {
        font: Face::Inter,
        size: 13.0,
        color: rgb8(77, 77, 77),
        padding: 12.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Left,
        inset: 8.0,
        gap: 8.0,
        buttons: &[
            Button {
                action: ButtonAction::Close,
                shape: ButtonShape::Circle,
                glyph: Glyph::Cross,
                width_ratio: AQUA_LIGHT_RATIO,
                diameter_ratio: AQUA_LIGHT_RATIO,
                glyph_ratio: AQUA_GLYPH,
                // The glyph is transparent until hovered, exactly as Aqua hides its marks until
                // the pointer enters the cluster.
                idle: aqua_red(rgb8(255, 128, 122), rgb8(255, 95, 87), TRANSPARENT),
                hover: aqua_red(rgb8(255, 128, 122), rgb8(255, 95, 87), rgb8(77, 0, 0)),
                active: aqua_red(rgb8(211, 87, 81), rgb8(191, 71, 66), rgb8(77, 0, 0)),
            },
            aqua_inert(),
            aqua_inert(),
        ],
        // All three lights flat grey: macOS's own look for a window that cannot be closed, and
        // already this theme's idiom for the two it never offers.
        unclosable: Some(aqua_disabled_light()),
    },
};

// adwaita — GNOME. A tall flat headerbar with a centred title and a round close button.
// Current libadwaita defaults. Its foreground is 80%-opaque near-black; this is that colour
// composited over the white header bar, since our chrome has no translucent backing layer.
pub(super) const ADWAITA_HEADER: Color = WHITE;
pub(super) const ADWAITA_TEXT: Color = rgb8(51, 51, 56);

pub(super) const ADWAITA_CHROME: Chrome = Chrome {
    header: Fill::Solid(ADWAITA_HEADER),
    border: &[BorderRing::Uniform(rgb8(224, 224, 224))],
    separator: None,
    title: TitleStyle {
        font: Face::Cantarell,
        size: 14.0,
        color: ADWAITA_TEXT,
        padding: 12.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 7.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Circle,
            glyph: Glyph::Cross,
            // A 24px circular image inside a 34px image-button slot.
            width_ratio: 34.0 / 47.0,
            diameter_ratio: 24.0 / 47.0,
            glyph_ratio: ADWAITA_GLYPH,
            idle: paint(rgb8(230, 230, 231), ADWAITA_TEXT),
            hover: paint(rgb8(210, 210, 212), ADWAITA_TEXT),
            active: paint(rgb8(190, 190, 192), ADWAITA_TEXT),
        }],
        unclosable: None,
    },
};

// breeze — KDE Plasma. Cool neutral surfaces, compact geometry and KDE's blue accent. KWin
// decorations vary by distribution, so this follows upstream Breeze rather than a distro skin.
// BreezeLight's active Header background. #eff0f1 is its inactive header and window body.
pub(super) const BREEZE_HEADER: Color = rgb8(222, 224, 226);
pub(super) const BREEZE_TEXT: Color = rgb8(35, 38, 41);

pub(super) const BREEZE_CHROME: Chrome = Chrome {
    header: Fill::Solid(BREEZE_HEADER),
    border: &[BorderRing::Uniform(rgb8(189, 195, 199))],
    // Breeze's bar *is* the window colour — the flat, seamless look is the point — so the hairline
    // beneath it is the only thing marking where the chrome ends. KWin's Breeze decoration draws
    // exactly this line; it matches the window border, which is what keeps the frame reading as
    // one shape.
    separator: Some(rgb8(189, 195, 199)),
    title: TitleStyle {
        font: Face::NotoSans,
        size: 13.0,
        color: BREEZE_TEXT,
        padding: 10.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 6.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Circle,
            glyph: Glyph::Cross,
            width_ratio: 0.55,
            diameter_ratio: 0.55,
            glyph_ratio: BREEZE_GLYPH,
            idle: paint(BREEZE_HEADER, BREEZE_TEXT),
            // KWin lightens/darkens the scheme's #da4453 Negative foreground.
            hover: paint(rgb8(255, 102, 124), WHITE),
            active: paint(rgb8(109, 34, 42), BREEZE_HEADER),
        }],
        unclosable: None,
    },
};

// cde — the Common Desktop Environment's Motif language: blue-green active title, warm grey
// faces and deeply modelled bevels. The palette is intentionally workstation-like rather than a
// clone of any one vendor's CDE defaults, which differed across Solaris, HP-UX and AIX.
pub(super) const CDE_FACE: Color = rgb8(184, 184, 174);
pub(super) const CDE_SHADOW: Color = rgb8(82, 82, 76);
pub(super) const CDE_TITLE: Color = rgb8(45, 98, 96);
pub(super) const CDE_FACE_DARK: Color = rgb8(91, 98, 94);
pub(super) const CDE_PANEL_DARK: Color = rgb8(62, 67, 65);
pub(super) const CDE_SHADOW_DARK: Color = rgb8(25, 28, 27);
pub(super) const CDE_TITLE_DARK: Color = rgb8(26, 75, 73);

pub(super) const CDE_RAISED: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: rgb8(238, 238, 226),
        bottom_right: rgb8(70, 70, 65),
    },
    BorderRing::Bevel {
        top_left: rgb8(211, 211, 200),
        bottom_right: rgb8(118, 118, 109),
    },
];
pub(super) const CDE_PRESSED: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: rgb8(70, 70, 65),
        bottom_right: rgb8(238, 238, 226),
    },
    BorderRing::Bevel {
        top_left: rgb8(118, 118, 109),
        bottom_right: rgb8(211, 211, 200),
    },
];
pub(super) const CDE_RAISED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: rgb8(154, 164, 158),
        bottom_right: CDE_SHADOW_DARK,
    },
    BorderRing::Bevel {
        top_left: rgb8(124, 134, 129),
        bottom_right: rgb8(47, 51, 49),
    },
];
pub(super) const CDE_PRESSED_DARK: &[BorderRing] = &[
    BorderRing::Bevel {
        top_left: CDE_SHADOW_DARK,
        bottom_right: rgb8(154, 164, 158),
    },
    BorderRing::Bevel {
        top_left: rgb8(47, 51, 49),
        bottom_right: rgb8(124, 134, 129),
    },
];

pub(super) const CDE_CHROME: Chrome = Chrome {
    header: Fill::Solid(CDE_TITLE),
    border: &[
        BorderRing::Bevel {
            top_left: rgb8(238, 238, 226),
            bottom_right: rgb8(55, 55, 51),
        },
        BorderRing::Bevel {
            top_left: rgb8(211, 211, 200),
            bottom_right: CDE_SHADOW,
        },
        BorderRing::Uniform(CDE_FACE),
    ],
    separator: None,
    title: TitleStyle {
        font: Face::LiberationSansBold,
        size: 12.0,
        color: WHITE,
        padding: 4.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 2.0,
        gap: 0.0,
        buttons: &[rect_close(
            0.85,
            paint(CDE_FACE, BLACK),
            paint(rgb8(205, 205, 194), BLACK),
            paint(rgb8(145, 145, 137), BLACK),
        )],
        unclosable: None,
    },
};

pub(super) const CDE_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(CDE_TITLE_DARK),
    border: &[
        BorderRing::Bevel {
            top_left: rgb8(154, 164, 158),
            bottom_right: rgb8(18, 20, 19),
        },
        BorderRing::Bevel {
            top_left: rgb8(124, 134, 129),
            bottom_right: CDE_SHADOW_DARK,
        },
        BorderRing::Uniform(CDE_FACE_DARK),
    ],
    separator: None,
    title: TitleStyle {
        font: Face::LiberationSansBold,
        size: 12.0,
        color: WHITE,
        padding: 4.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 2.0,
        gap: 0.0,
        buttons: &[rect_close(
            0.85,
            paint(CDE_FACE_DARK, WHITE),
            paint(rgb8(111, 119, 115), WHITE),
            paint(rgb8(67, 72, 70), WHITE),
        )],
        unclosable: None,
    },
};

// platinum — Mac OS 9. Pinstriped bar, black frame, close box on the *left*. No zoom or collapse
// box: Mac OS 9 omits them on windows that cannot do either, so nothing has to be faked.
pub(super) const PLATINUM_FACE: Color = rgb8(221, 221, 221);
/// The pinstripe pair is its own, darker-contrasting pair rather than a tint of `PLATINUM_FACE`:
/// at only a few levels apart the stripes vanished into flat grey, especially above 1x where each
/// logical line covers a fractional number of physical pixels.
pub(super) const PLATINUM_STRIPE_BASE: Color = rgb8(207, 207, 207);
pub(super) const PLATINUM_STRIPE: Color = rgb8(232, 232, 232);

pub(super) const PLATINUM_CHROME: Chrome = Chrome {
    header: Fill::Pinstripe {
        base: PLATINUM_STRIPE_BASE,
        stripe: PLATINUM_STRIPE,
        period: 2,
    },
    border: &[BorderRing::Uniform(BLACK)],
    separator: None,
    title: TitleStyle {
        font: Face::SourceSansSemibold,
        size: 12.0,
        color: BLACK,
        padding: 4.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Left,
        inset: 4.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Square,
            glyph: Glyph::Square,
            width_ratio: 0.6,
            diameter_ratio: 0.6,
            glyph_ratio: RECT_GLYPH,
            idle: paint(PLATINUM_FACE, BLACK),
            hover: paint(rgb8(238, 238, 238), BLACK),
            active: paint(rgb8(170, 170, 170), BLACK),
        }],
        unclosable: None,
    },
};

// ── Dark palettes ────────────────────────────────────────────────────────────────
//
// Same metrics, same button geometry, different colours — see `Appearance`. Only `platinum` has
// none: Mac OS 9 had no dark mode, and an invented one would be a look nobody recognises.

// plain dark. The loud red close button is kept deliberately: `plain` is the one theme not
// imitating anything, and a close target that stands out is a feature when windows are stacked.
// Neutral greys, not the slightly blue-tinted pair this had: `plain` is greyscale by definition.
pub(super) const PLAIN_HEADER_DARK: Color = rgb8(38, 38, 38);
pub(super) const PLAIN_TEXT_DARK: Color = rgb8(240, 240, 240);

pub(super) const PLAIN_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(PLAIN_HEADER_DARK),
    // Mid-grey rather than black: a dark border against a dark header leaves no visible edge.
    border: &[BorderRing::Uniform(rgb8(128, 128, 128))],
    separator: None,
    title: TitleStyle {
        font: Face::Default,
        size: 12.0,
        color: PLAIN_TEXT_DARK,
        padding: 8.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 0.0,
        gap: 0.0,
        buttons: &[plain_close(PLAIN_HEADER_DARK, PLAIN_TEXT_DARK)],
        unclosable: None,
    },
};

// fluent dark — Windows 11's own dark title bar, keeping the same close red.
// The same one-step separation as `FLUENT_HEADER`, in the direction dark mode moves: a caption
// slightly lighter than the #202020 panel beneath it.
pub(super) const FLUENT_HEADER_DARK: Color = rgb8(43, 43, 43);

pub(super) const FLUENT_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(FLUENT_HEADER_DARK),
    border: &[BorderRing::Uniform(rgb8(74, 74, 74))],
    separator: None,
    title: TitleStyle {
        font: Face::Selawik,
        size: 12.0,
        color: WHITE,
        padding: 16.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 0.0,
        gap: 0.0,
        buttons: &[rect_close(
            46.0 / 32.0,
            paint(FLUENT_HEADER_DARK, WHITE),
            paint(rgb8(196, 43, 28), WHITE),
            paint(rgb8(168, 36, 25), WHITE),
        )],
        // See `FLUENT_CHROME`.
        unclosable: Some(paint(FLUENT_HEADER_DARK, rgb8(122, 122, 122))),
    },
};

// redmond dark. Not "Win95 with a dark mode" — Win95 shipped dark *appearance schemes* of its own
// (Eggplant, Plum), so this is a period-correct palette rather than an anachronism.
//
// Note the title bar is *lighter* than the light variant's, which is not a mistake: the light bar
// is `#000080` navy, already darker than most dark-mode chrome. What actually darkens is the face
// — the frame, the button fills and the dialog background around the content.
pub(super) const REDMOND_FACE_DARK: Color = rgb8(112, 108, 124);
pub(super) const REDMOND_PANEL_DARK: Color = rgb8(78, 77, 85);
pub(super) const REDMOND_TITLE_DARK: Color = rgb8(46, 32, 60);
pub(super) const REDMOND_HIGHLIGHT_DARK: Color = rgb8(174, 169, 190);
pub(super) const REDMOND_BRIGHT_DARK: Color = rgb8(211, 206, 222);
pub(super) const REDMOND_SHADOW_DARK: Color = rgb8(67, 63, 75);
pub(super) const REDMOND_GLYPH_DARK: Color = rgb8(240, 238, 245);

pub(super) const REDMOND_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(REDMOND_TITLE_DARK),
    border: &[
        BorderRing::Bevel {
            top_left: REDMOND_HIGHLIGHT_DARK,
            bottom_right: BLACK,
        },
        BorderRing::Bevel {
            top_left: REDMOND_BRIGHT_DARK,
            bottom_right: REDMOND_SHADOW_DARK,
        },
        BorderRing::Uniform(REDMOND_FACE_DARK),
    ],
    separator: None,
    title: TitleStyle {
        font: Face::Pixel,
        size: 12.0,
        color: WHITE,
        padding: 3.0,
        align: TextAlign::Left,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 2.0,
        gap: 0.0,
        buttons: &[rect_close(
            0.9,
            paint(REDMOND_FACE_DARK, REDMOND_GLYPH_DARK),
            paint(rgb8(124, 119, 148), REDMOND_GLYPH_DARK),
            paint(rgb8(90, 86, 112), REDMOND_GLYPH_DARK),
        )],
        // See `REDMOND_CHROME`.
        unclosable: Some(paint(REDMOND_FACE_DARK, rgb8(163, 159, 178))),
    },
};

// aqua dark. The traffic lights keep their colours — macOS does not grey them in dark mode; only
// the bar and title change. The inert pair moves to dark mode's own disabled grey.
pub(super) const AQUA_DISABLED_DARK: Color = rgb8(90, 90, 90);

pub(super) const fn aqua_disabled_dark_paint() -> ButtonPaint {
    rimmed(
        Fill::VerticalGradient {
            from: rgb8(105, 105, 105),
            to: AQUA_DISABLED_DARK,
        },
        TRANSPARENT,
        rgb8(65, 65, 65),
    )
}

pub(super) const fn aqua_inert_dark() -> Button {
    Button {
        action: ButtonAction::Inert,
        shape: ButtonShape::Circle,
        glyph: Glyph::None,
        width_ratio: AQUA_LIGHT_RATIO,
        diameter_ratio: AQUA_LIGHT_RATIO,
        glyph_ratio: AQUA_GLYPH,
        idle: aqua_disabled_dark_paint(),
        hover: aqua_disabled_dark_paint(),
        active: aqua_disabled_dark_paint(),
    }
}

pub(super) const AQUA_CHROME_DARK: Chrome = Chrome {
    header: Fill::VerticalGradient {
        from: rgb8(58, 58, 60),
        to: rgb8(44, 44, 46),
    },
    border: &[BorderRing::Uniform(rgb8(74, 74, 76))],
    separator: None,
    title: TitleStyle {
        font: Face::Inter,
        size: 13.0,
        color: rgb8(176, 176, 180),
        padding: 12.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Left,
        inset: 8.0,
        gap: 8.0,
        buttons: &[
            Button {
                action: ButtonAction::Close,
                shape: ButtonShape::Circle,
                glyph: Glyph::Cross,
                width_ratio: AQUA_LIGHT_RATIO,
                diameter_ratio: AQUA_LIGHT_RATIO,
                glyph_ratio: AQUA_GLYPH,
                idle: aqua_red(rgb8(255, 128, 122), rgb8(255, 95, 87), TRANSPARENT),
                hover: aqua_red(rgb8(255, 128, 122), rgb8(255, 95, 87), rgb8(77, 0, 0)),
                active: aqua_red(rgb8(211, 87, 81), rgb8(191, 71, 66), rgb8(77, 0, 0)),
            },
            aqua_inert_dark(),
            aqua_inert_dark(),
        ],
        // See `AQUA_CHROME`.
        unclosable: Some(aqua_disabled_dark_paint()),
    },
};

// adwaita dark — GNOME's own dark headerbar.
pub(super) const ADWAITA_HEADER_DARK: Color = rgb8(46, 46, 50);

pub(super) const ADWAITA_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(ADWAITA_HEADER_DARK),
    border: &[BorderRing::Uniform(rgb8(29, 29, 32))],
    separator: None,
    title: TitleStyle {
        font: Face::Cantarell,
        size: 14.0,
        color: WHITE,
        padding: 12.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 7.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Circle,
            glyph: Glyph::Cross,
            width_ratio: 34.0 / 47.0,
            diameter_ratio: 24.0 / 47.0,
            glyph_ratio: ADWAITA_GLYPH,
            idle: paint(rgb8(67, 67, 71), WHITE),
            hover: paint(rgb8(83, 83, 87), WHITE),
            active: paint(rgb8(101, 101, 105), WHITE),
        }],
        unclosable: None,
    },
};

pub(super) const BREEZE_HEADER_DARK: Color = rgb8(41, 44, 48);

pub(super) const BREEZE_CHROME_DARK: Chrome = Chrome {
    header: Fill::Solid(BREEZE_HEADER_DARK),
    border: &[BorderRing::Uniform(rgb8(23, 26, 28))],
    // See `BREEZE_CHROME`. A step darker than the bar rather than the near-black window border,
    // which at this size would read as a gap in the window rather than a line on it.
    separator: Some(rgb8(38, 42, 46)),
    title: TitleStyle {
        font: Face::NotoSans,
        size: 13.0,
        color: rgb8(239, 240, 241),
        padding: 10.0,
        align: TextAlign::Center,
    },
    buttons: Buttons {
        side: Side::Right,
        inset: 6.0,
        gap: 0.0,
        buttons: &[Button {
            action: ButtonAction::Close,
            shape: ButtonShape::Circle,
            glyph: Glyph::Cross,
            width_ratio: 0.55,
            diameter_ratio: 0.55,
            glyph_ratio: BREEZE_GLYPH,
            idle: paint(BREEZE_HEADER_DARK, rgb8(252, 252, 252)),
            hover: paint(rgb8(255, 102, 124), WHITE),
            // KWin uses the title-bar colour here, but that is nearly black in Breeze Dark and
            // misses even a basic contrast floor against the darkened red. Keep the canonical
            // pressed fill and the scheme's white foreground so the close mark remains legible.
            active: paint(rgb8(109, 34, 42), WHITE),
        }],
        unclosable: None,
    },
};
