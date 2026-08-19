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
    rim: None,
};

const fn button(action: ButtonAction, width_ratio: f32) -> Button {
    Button {
        action,
        shape: ButtonShape::Circle,
        glyph: Glyph::None,
        width_ratio,
        diameter_ratio: width_ratio,
        glyph_ratio: 0.25,
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
            unclosable: None,
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

#[test]
fn only_header_background_is_a_drag_region() {
    let header = plain_header();
    let (button_x, _) = header.button_span(0);

    assert!(header.is_drag_region(at(20.0, 5.0)));
    assert!(!header.is_drag_region(at(button_x as f64 + 1.0, 5.0)));
    assert!(!header.is_drag_region(at(20.0, -1.0)));
    assert!(!header.is_drag_region(at(20.0, HEADER_H as f64)));
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
            unclosable: None,
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
        rim: None,
    };

    let mut header = header(
        Buttons {
            side: Side::Left,
            inset: 0.0,
            gap: 0.0,
            buttons: &TRAFFIC_LIGHTS,
            unclosable: None,
        },
        true,
    );
    // Pretend the pointer somehow settled on it, which `button_at` will not do.
    header.hovered = Some(1);

    assert_eq!(header.paint_for(1, &inert), inert.idle);
}

/// A theme with no disabled paint drops the cluster entirely on an unclosable window — GNOME,
/// KDE and Mac OS 9 all simply omit a close button a window does not have.
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

/// A theme that supplies one keeps the cluster and greys it: painted, but dead to the pointer
/// in every state. Drawn *because* it says "cannot be closed" — the function-honesty rule
/// forbids a control that looks live and isn't, not one that looks disabled and is.
#[test]
fn a_disabled_cluster_is_painted_but_never_live() {
    for &theme in crate::window::theme::ALL_THEMES {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let chrome = theme.chrome(appearance);
            let Some(disabled) = chrome.buttons.unclosable else {
                continue;
            };

            let mut header = Header::new(
                RedrawRequester::detached(),
                chrome,
                PhysicalSize::new(400, 100),
                1.0,
                theme.metrics().header_height,
                None,
                false,
            );

            assert_eq!(
                header.buttons().len(),
                chrome.buttons.buttons.len(),
                "{theme:?} {appearance:?} dropped its disabled cluster"
            );

            let (x, width) = header.button_span(0);
            let over = at(
                (x + width / 2.0) as f64,
                (theme.metrics().header_height / 2) as f64,
            );

            // Every button wears the disabled paint, whatever the pointer is doing.
            for (index, button) in header.buttons().iter().enumerate() {
                assert_eq!(
                    header.paint_for(index, button),
                    disabled,
                    "{theme:?} {appearance:?} button {index}"
                );
            }

            // And none of them can be hovered, pressed or activated.
            assert_eq!(header.button_at(over), None, "{theme:?} {appearance:?}");
            header.handle_cursor_moved(over);
            assert_eq!(header.hovered, None, "{theme:?} {appearance:?} hovered");
            header.handle_mouse_down();
            assert!(
                !header.handle_mouse_up(),
                "{theme:?} {appearance:?} closed an unclosable window"
            );
        }
    }
}

/// The catalogue's own division: only the platforms that really do grey a caption button keep
/// one on an unclosable window. Pinned so a future theme has to make the choice deliberately
/// rather than inheriting whichever default it was copied from.
#[test]
fn only_windows_and_macos_grey_their_close_button() {
    use crate::window::theme::Theme;

    for &theme in crate::window::theme::ALL_THEMES {
        let greys = theme.chrome(Appearance::Light).buttons.unclosable.is_some();
        let expected = matches!(theme, Theme::Fluent | Theme::Redmond | Theme::Aqua);
        assert_eq!(greys, expected, "{theme:?}");

        // Light and dark cannot disagree about whether the theme greys at all.
        assert_eq!(
            theme.chrome(Appearance::Dark).buttons.unclosable.is_some(),
            greys,
            "{theme:?} greys in one appearance but not the other"
        );
    }
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
        let pixmap = rasterise_text(
            "Title",
            &font,
            PxScale::from(20.0),
            4.0,
            20.0,
            color,
            size,
            (0.0, size.width as f32),
        );

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

/// A title too long for its bar is cut off at the buttons, and never draws behind them.
///
/// Compared against the same header with no title at all rather than against a colour, so it
/// holds for every theme: any pixel in the buttons' own columns that the title changes is a
/// pixel of caption showing through the gaps around a circular or rounded button — which is
/// what made a long caption look like it ran past the close button.
#[test]
fn a_long_title_never_reaches_the_buttons_columns() {
    const LONG: &str = "default caption. no mood associated. and it keeps going, and going";

    for &theme in crate::window::theme::ALL_THEMES {
        for appearance in [Appearance::Light, Appearance::Dark] {
            for scale in [1.0, 1.5, 2.0] {
                let build = |title: Option<String>| {
                    Header::new(
                        RedrawRequester::detached(),
                        theme.chrome(appearance),
                        LogicalSize::new(200u32, 100).to_physical(scale),
                        scale,
                        theme.metrics().header_height,
                        title,
                        true,
                    )
                };

                let mut bare = build(None);
                let mut titled = build(Some(LONG.to_owned()));

                // The columns the button cluster owns, in physical pixels.
                let extent = bare.buttons_extent() * scale as f32;
                let full = bare.physical_size.width as f32;
                let (from, to) = match theme.chrome(appearance).buttons.side {
                    Side::Left => (0.0, extent),
                    Side::Right => (full - extent, full),
                };

                for x in (from.round().max(0.0) as u32)..(to.round().min(full) as u32) {
                    for y in 0..bare.physical_size.height {
                        assert_eq!(
                            pixel(&mut titled, x, y),
                            pixel(&mut bare, x, y),
                            "{theme:?} {appearance:?} at {scale}x: the title bled into the \
                             buttons' columns at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}

/// The clip is a hard cut, not a whole-title drop: a caption that overflows still shows the
/// part of itself that fits.
#[test]
fn a_clipped_title_still_draws_the_part_that_fits() {
    let font = text_font::chrome_font(crate::lua::TextFont::Default).expect("default font");
    let size = PhysicalSize::new(200, 30);
    let color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    let painted = |clip: (f32, f32)| {
        let pixmap = rasterise_text(
            "A title far too long for the space it has",
            &font,
            PxScale::from(20.0),
            4.0,
            20.0,
            color,
            size,
            clip,
        );

        let columns: Vec<usize> = pixmap
            .data()
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, px)| px[3] > 0)
            .map(|(index, _)| index % size.width as usize)
            .collect();

        (
            *columns.iter().min().expect("some text drawn"),
            *columns.iter().max().expect("some text drawn"),
        )
    };

    let (unclipped_left, unclipped_right) = painted((0.0, size.width as f32));
    assert!(unclipped_right > 100, "the sample text needs to overflow");

    let (left, right) = painted((0.0, 100.0));
    assert_eq!(left, unclipped_left, "the start of the title is untouched");
    assert!(right < 100, "drawn past the clip, at column {right}");
    assert!(right > 80, "clipped far too early, at column {right}");
}

/// The themes whose bar is the same colour as the panel below it must be separated from it
/// somehow, or a small popup reads as one undifferentiated block — worst of all on a window
/// with no close button to give the bar away.
///
/// Two mechanisms are allowed, matching what each platform does: a hairline (`breeze`, which
/// is KWin's own), or a tonal step between bar and panel (`fluent`, since Windows draws no
/// line). This asserts each theme has *one* of them, not which.
#[test]
fn a_flat_theme_separates_its_bar_from_its_panel() {
    for &theme in crate::window::theme::ALL_THEMES {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let chrome = theme.chrome(appearance);
            let Fill::Solid(bar) = chrome.header else {
                // A gradient or pinstripe is already visibly not a flat panel.
                continue;
            };
            let panel = theme.widgets(appearance).panel;

            let distance = |a: crate::lua::Color, b: crate::lua::Color| {
                ((a.r - b.r).abs() + (a.g - b.g).abs() + (a.b - b.b).abs()) / 3.0
            };

            // A bar that already differs from the panel needs no help.
            if distance(bar, panel) > 0.01 {
                continue;
            }

            assert!(
                chrome.separator.is_some(),
                "{theme:?} {appearance:?}: the bar is the panel colour and has no separator, \
                 so the header has no edge at all"
            );
        }
    }
}

/// The separator is drawn over the buttons, not just the bar: it is the window's structure,
/// and a gap where a button sits would make it read as part of the bar's fill.
#[test]
fn the_separator_runs_the_full_width_of_the_bar() {
    let theme = crate::window::theme::Theme::Breeze;
    let chrome = theme.chrome(Appearance::Light);
    let separator = chrome.separator.expect("breeze draws a separator");
    let expected = [
        (separator.r * 255.0).round() as u8,
        (separator.g * 255.0).round() as u8,
        (separator.b * 255.0).round() as u8,
        255,
    ];

    // Including fractional scale factors, where an antialiased hairline would blend back into
    // the bar and stop reading as an edge at all.
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let mut header = Header::new(
            RedrawRequester::detached(),
            chrome,
            LogicalSize::new(200u32, 100).to_physical(scale),
            scale,
            theme.metrics().header_height,
            Some("A title long enough to reach the buttons".to_owned()),
            true,
        );

        let (width, height) = (header.physical_size.width, header.physical_size.height);

        for x in 0..width {
            assert_eq!(
                pixel(&mut header, x, height - 1),
                expected,
                "at {scale}x, column {x} of the separator row"
            );
        }

        // And it is a line on the bar, not a band: above it, the bar is still the bar.
        assert_ne!(pixel(&mut header, 2, height / 2), expected, "at {scale}x");
    }
}

/// Each platform's mark is its own proportion, and changing one must not move another.
///
/// This is the property the old design could not hold: the size lived on the [`Glyph`] variant,
/// so every theme drawing a cross shared one number. Sizing macOS's light correctly took
/// Breeze's cross to 94% of its circle, and moved GNOME's without anyone touching Adwaita.
/// The numbers below are per-theme measurements, so a wrong one is wrong on its own.
#[test]
fn every_theme_marks_its_close_button_in_its_own_proportion() {
    use crate::window::theme::Theme;

    // (theme, the mark's span as a fraction of its button)
    let expected = [
        (Theme::Plain, 1.0 / 3.0),
        (Theme::Fluent, 1.0 / 3.0),
        (Theme::Redmond, 1.0 / 3.0),
        (Theme::Platinum, 1.0 / 3.0),
        (Theme::Cde, 1.0 / 3.0),
        // macOS spans half its traffic light; GNOME leaves visible margin round a symbolic
        // icon in a much larger circle; Breeze sits between them.
        (Theme::Aqua, 0.5),
        (Theme::Adwaita, 0.4),
        (Theme::Breeze, 0.4714),
    ];

    for (theme, span) in expected {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let chrome = theme.chrome(appearance);
            let close = chrome
                .buttons
                .buttons
                .iter()
                .find(|button| button.glyph != Glyph::None)
                .unwrap_or_else(|| panic!("{theme:?} has no marked button"));

            assert!(
                (close.glyph_ratio * 2.0 - span).abs() < 0.001,
                "{theme:?} {appearance:?}: marks its close button at {:.1}% of the button, \
                 expected {:.1}%",
                close.glyph_ratio * 200.0,
                span * 100.0
            );
        }
    }
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
                let reach = header.glyph_reach(button, width);

                // The cross's corners, which are its furthest points from the centre.
                let (centre_x, centre_y) = (x + width / 2.0, header_height / 2.0);
                let corner_x = reach;
                let corner_y = reach;

                match button.shape {
                    ButtonShape::Rect | ButtonShape::Square => {
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
                        let radius = (header_height * button.diameter_ratio)
                            .min(width)
                            .min(header_height)
                            / 2.0;
                        let corner = (corner_x * corner_x + corner_y * corner_y).sqrt();
                        // Fitting is not enough: a mark whose corners graze the rim reads as a
                        // cross wedged into a circle rather than a mark drawn inside one.
                        // `breeze` sat at 94% of its radius before this bound existed, which
                        // "stays inside its button" was perfectly happy with.
                        assert!(
                            corner <= radius * 0.8,
                            "{theme:?} {appearance:?} button {index}: the cross reaches \
                             {corner} of a {radius} radius — it crowds its circle"
                        );
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
            unclosable: None,
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
