use super::*;
use crate::window::theme::Appearance;

const PANEL: f32 = 400.0;

fn buttons(count: usize) -> Vec<DialogButton> {
    (0..count)
        .map(|index| DialogButton {
            id: format!("b{index}"),
            label: format!("Button {index}"),
            default: index == 0,
        })
        .collect()
}

fn elements(button_count: usize) -> Vec<DialogElementState> {
    vec![
        DialogElementState::Input {
            id: "field".to_owned(),
            placeholder: None,
            value: String::new(),
        },
        DialogElementState::Buttons {
            id: None,
            options: buttons(button_count),
        },
    ]
}

/// Lay a dialog out in a `PANEL`-wide viewport and return the rect of every filled rectangle
/// egui emitted, plus where it put each piece of text.
///
/// `paint_dialog` only needs a `Ui`, so this exercises the real layout with no window, no wgpu
/// and no egui-winit — which is what makes the geometry testable at all.
fn layout(button_count: usize) -> (Vec<egui::Rect>, Vec<(String, egui::Pos2)>) {
    layout_themed(Theme::Plain, button_count)
}

/// As [`layout`], but with a theme's real style applied — the spacing and control metrics a
/// theme sets are exactly what can push content out of the region it was allocated, so a
/// harness on egui's defaults cannot see those problems at all.
fn layout_themed(
    theme: Theme,
    button_count: usize,
) -> (Vec<egui::Rect>, Vec<(String, egui::Pos2)>) {
    let (rects, texts) = layout_shapes(theme, button_count);
    (rects.into_iter().map(|(rect, _)| rect).collect(), texts)
}

/// As [`layout_themed`], but keeping each rectangle's fill.
#[allow(clippy::type_complexity)]
fn layout_shapes(
    theme: Theme,
    button_count: usize,
) -> (Vec<(egui::Rect, egui::Color32)>, Vec<(String, egui::Pos2)>) {
    let ctx = egui::Context::default();
    ctx.set_global_style(theme::window_style(theme, Appearance::Light, None));
    if let Some(fonts) = theme::widget_font_definitions(theme) {
        ctx.set_fonts(fonts);
    }

    let mut elements = elements(button_count);

    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(PANEL, 300.0),
        )),
        ..Default::default()
    };

    let output = ctx.run_ui(input, |ui| {
        paint_dialog(
            ui,
            &mut elements,
            Some("b0"),
            &theme.widget_edge(Appearance::Light),
            theme.default_button_style(Appearance::Light),
        );
    });

    let mut rects = Vec::new();
    let mut texts = Vec::new();
    for shape in &output.shapes {
        match &shape.shape {
            egui::epaint::Shape::Rect(rect) => rects.push((rect.rect, rect.fill)),
            egui::epaint::Shape::Text(text) => {
                texts.push((text.galley.text().to_owned(), text.pos))
            }
            _ => {}
        }
    }

    (rects, texts)
}

#[test]
fn unstyled_dialog_text_inherits_the_theme_face_but_explicit_fonts_win() {
    for &theme in crate::window::theme::ALL_THEMES {
        let ctx = egui::Context::default();
        ctx.set_global_style(theme::window_style(theme, Appearance::Light, None));

        let mut inherited = None;
        let mut explicit = None;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            inherited = Some(dialog_text_family(ui, lua::TextFont::Default));
            explicit = Some(dialog_text_family(ui, lua::TextFont::Mono));
        });

        assert_eq!(
            inherited,
            Some(text_font::font_family(theme.widget_font())),
            "{theme:?}"
        );
        // A named family, not egui's generic `Monospace`: the mono face is bundled like every
        // other, so nothing is left to egui's own choice of file.
        assert_eq!(
            explicit,
            Some(text_font::font_family(lua::TextFont::Mono)),
            "{theme:?}"
        );
    }
}

/// An unstyled dialog caption is set at the theme's own body size, and an explicit size still
/// wins.
///
/// The size used to be a flat 32pt for every theme and every surface, which made a caption
/// more than twice the height of the controls under it — and, because the *face* already
/// followed the theme, made the text appear to change size from theme to theme as the face
/// changed beneath a constant number.
#[test]
fn unstyled_dialog_text_takes_the_themes_own_size() {
    for &theme in crate::window::theme::ALL_THEMES {
        let ctx = egui::Context::default();
        ctx.set_global_style(theme::window_style(theme, Appearance::Light, None));
        if let Some(fonts) = theme::widget_font_definitions(theme) {
            ctx.set_fonts(fonts);
        }

        let mut elements = vec![
            DialogElementState::Text {
                id: None,
                text: "inherited".to_owned(),
                style: TextStyle::default(),
            },
            DialogElementState::Text {
                id: None,
                text: "explicit".to_owned(),
                style: TextStyle {
                    font_size: Some(lua::FontSize::Value(40.0)),
                    ..Default::default()
                },
            },
        ];

        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(PANEL, 300.0),
            )),
            ..Default::default()
        };

        let output = ctx.run_ui(input, |ui| {
            paint_dialog(
                ui,
                &mut elements,
                None,
                &theme.widget_edge(Appearance::Light),
                theme.default_button_style(Appearance::Light),
            );
        });

        let sizes: HashMap<String, f32> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some((
                    text.galley.job.text.clone(),
                    text.galley.job.sections[0].format.font_id.size,
                )),
                _ => None,
            })
            .collect();

        let body = theme.widgets(Appearance::Light).font_size;
        assert_eq!(sizes.get("inherited"), Some(&body), "{theme:?}");
        assert_eq!(sizes.get("explicit"), Some(&40.0), "{theme:?}");
    }
}

/// egui paints an orange "Unaligned" marker (debug builds only) when a `Ui`'s content overflows
/// the region it was allocated. That is a layout error, not cosmetic noise, so nothing a dialog
/// lays out may provoke one.
#[test]
fn no_debug_warnings_are_emitted() {
    for &theme in crate::window::theme::ALL_THEMES {
        for count in [1, 2, 8] {
            let (_, texts) = layout_themed(theme, count);
            let warnings: Vec<&String> = texts
                .iter()
                .map(|(text, _)| text)
                .filter(|text| text.contains("Unaligned") || text.contains("Debug"))
                .collect();

            assert!(
                warnings.is_empty(),
                "{theme:?}, {count} buttons: egui flagged the layout: {warnings:?}"
            );
        }
    }
}

/// The dialog's background must cover the whole window, not just the strip its content
/// occupies — otherwise the surface's clear colour (black) shows through beneath.
///
/// This filled by accident until the button row stopped being infinitely wide: the infinite
/// rect stretched the frame's `min_rect` to the bottom of the window.
#[test]
fn the_dialog_background_covers_the_whole_window() {
    for count in [1, 2, 8] {
        let (rects, _) = layout(count);

        let covers = rects.iter().any(|rect| {
            rect.min.x <= 0.5
                && rect.min.y <= 0.5
                && rect.max.x >= PANEL - 0.5
                && rect.max.y >= 299.5
        });

        assert!(
            covers,
            "{count} buttons: nothing fills the 400x300 window; largest was {:?}",
            rects.iter().max_by(|a, b| a.area().total_cmp(&b.area()))
        );
    }
}

/// A theme's text field should be the same height as its buttons — a short field beside a tall
/// button is the sort of mismatch that makes a themed dialog look assembled rather than
/// designed. egui sizes a `TextEdit` from its font and ignores `interact_size`, so this only
/// holds because `paint_dialog` pads it out deliberately.
#[test]
fn a_text_field_matches_its_themes_control_height() {
    for &theme in crate::window::theme::ALL_THEMES {
        let (rects, _) = layout_themed(theme, 1);

        let input = rects
            .iter()
            .filter(|rect| rect.width() > PANEL / 2.0 && rect.height() < 60.0)
            .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .expect("the input should have been drawn");
        let button = button_rects(&rects)
            .into_iter()
            .next()
            .expect("the button should have been drawn");

        let difference = (input.height() - button.height()).abs();
        assert!(
            difference <= 2.0,
            "{theme:?}: the text field is {} tall but its buttons are {}",
            input.height(),
            button.height()
        );
    }
}

/// The rects that are buttons: control-sized, and not the panel background.
///
/// The lower bound matters for the bevelled themes: those paint their edges as one-point strips,
/// so without it a two-button `redmond` dialog reports eighteen "buttons".
fn button_rects(rects: &[egui::Rect]) -> Vec<egui::Rect> {
    let candidates: Vec<_> = rects
        .iter()
        .copied()
        .filter(|rect| {
            rect.width() < PANEL / 2.0
                && rect.height() < 40.0
                && rect.width() > 8.0
                && rect.height() > 8.0
        })
        .collect();

    // A default-action outline is another control-sized rectangle nested just inside its
    // button. Keep only the outermost candidate so decoration is not mistaken for a second
    // control.
    candidates
        .iter()
        .copied()
        .filter(|rect| {
            !candidates.iter().any(|other| {
                other != rect
                    && other.contains(rect.min)
                    && other.contains(rect.max)
                    && other.area() > rect.area()
            })
        })
        .collect()
}

/// Nothing may be laid out at an infinite coordinate.
///
/// This is what `with_main_justify(true)` did on a *wrapping* layout: a wrapping layout reports
/// an infinite available main extent, so "fill the main axis" produced a button of infinite
/// width. Clipped to the panel it looked like a single wide bar, its label was drawn at
/// x = infinity where nothing is visible, and every button after the first vanished.
#[test]
fn nothing_is_laid_out_at_an_infinite_coordinate() {
    for count in [1, 2, 3, 8] {
        let (rects, texts) = layout(count);

        for rect in &rects {
            assert!(
                rect.min.x.is_finite() && rect.max.x.is_finite(),
                "{count} buttons: rect {rect:?} is not finite"
            );
        }
        for (text, pos) in &texts {
            assert!(
                pos.x.is_finite() && pos.y.is_finite(),
                "{count} buttons: {text:?} placed at {pos:?}"
            );
        }
    }
}

/// Every button is drawn, and every label lands inside the button it belongs to.
#[test]
fn every_button_is_drawn_with_its_label_inside_it() {
    for &theme in crate::window::theme::ALL_THEMES {
        for count in [1, 2, 3, 8] {
            let (rects, texts) = layout_themed(theme, count);
            let drawn = button_rects(&rects);

            assert_eq!(
                drawn.len(),
                count,
                "{theme:?}, {count} buttons: drew {}",
                drawn.len()
            );

            for index in 0..count {
                let label = format!("Button {index}");
                let (_, pos) = texts
                    .iter()
                    .find(|(text, _)| *text == label)
                    .unwrap_or_else(|| {
                        panic!("{theme:?}, {count} buttons: {label:?} was never drawn")
                    });

                assert!(
                    drawn.iter().any(|rect| rect.contains(*pos)),
                    "{theme:?}, {count} buttons: {label:?} at {pos:?} is outside every button"
                );
            }
        }
    }
}

/// Flat themes paint the default action as a filled primary button, rather than presenting
/// keyboard focus as a second outline around an otherwise ordinary control.
#[test]
fn flat_themes_fill_only_the_default_button_with_their_primary_colour() {
    for theme in [Theme::Plain, Theme::Fluent, Theme::Aqua, Theme::Adwaita] {
        let (shapes, _) = layout_shapes(theme, 2);
        let rects: Vec<_> = shapes.iter().map(|(rect, _)| *rect).collect();
        let buttons = button_rects(&rects);
        assert_eq!(buttons.len(), 2, "{theme:?}");

        let fills: Vec<_> = buttons
            .iter()
            .map(|button| {
                shapes
                    .iter()
                    .find(|(rect, _)| rect == button)
                    .expect("button should have a painted face")
                    .1
            })
            .collect();

        assert_ne!(
            fills[0], fills[1],
            "{theme:?}: default face was not distinct"
        );
    }
}

/// A bevelled theme's button really is two-toned: its edges are painted in more than one colour,
/// which is the whole reason those themes are not left to `egui::Style`.
#[test]
fn a_bevelled_theme_paints_a_two_tone_edge() {
    for &theme in crate::window::theme::ALL_THEMES {
        let bevelled = matches!(
            theme.widget_edge(Appearance::Light),
            WidgetEdge::Bevel { .. }
        );

        let (shapes, _) = layout_shapes(theme, 1);
        let button = button_rects(&shapes.iter().map(|(rect, _)| *rect).collect::<Vec<_>>())
            .into_iter()
            .next()
            .expect("a button should have been drawn");

        // The one-point strips inside the button's own bounds are its edges.
        let edge_tones: std::collections::BTreeSet<[u8; 4]> = shapes
            .iter()
            .filter(|(rect, _)| {
                button.contains(rect.center())
                    && (rect.width() <= 2.0 || rect.height() <= 2.0)
                    && rect.width() > 0.0
                    && rect.height() > 0.0
            })
            .map(|(_, fill)| fill.to_array())
            .collect();

        if bevelled {
            assert!(
                edge_tones.len() >= 2,
                "{theme:?} claims a bevel but paints {} edge tone(s)",
                edge_tones.len()
            );
        } else {
            assert!(
                edge_tones.is_empty(),
                "{theme:?} is flat but painted edge strips: {edge_tones:?}"
            );
        }
    }
}

#[test]
fn a_bevelled_theme_recesses_its_text_field() {
    for &theme in crate::window::theme::ALL_THEMES {
        let bevelled = matches!(
            theme.widget_edge(Appearance::Light),
            WidgetEdge::Bevel { .. }
        );
        let (shapes, _) = layout_shapes(theme, 1);
        let input = shapes
            .iter()
            .map(|(rect, _)| *rect)
            .filter(|rect| rect.width() > PANEL / 2.0 && rect.height() < 60.0)
            .min_by(|a, b| a.min.y.total_cmp(&b.min.y))
            .expect("the input should have been drawn");
        let edge_tones: std::collections::BTreeSet<_> = shapes
            .iter()
            .filter(|(rect, _)| {
                input.contains(rect.center())
                    && (rect.width() <= 2.0 || rect.height() <= 2.0)
                    && rect.width() > 0.0
                    && rect.height() > 0.0
            })
            .map(|(_, fill)| fill.to_array())
            .collect();

        if bevelled {
            assert!(edge_tones.len() >= 2, "{theme:?}: field is not recessed");
        } else {
            assert!(
                edge_tones.is_empty(),
                "{theme:?}: flat field drew edge strips"
            );
        }
    }
}

/// A row that fits is centred, matching the centred text above it. The enclosing
/// `top_down(Align::Center)` does the centring; the row only has to be allocated at its own
/// width rather than the full panel's.
#[test]
fn a_button_row_that_fits_is_centred_on_one_line() {
    let (rects, _) = layout(2);
    let drawn = button_rects(&rects);
    assert_eq!(drawn.len(), 2);

    // One line.
    assert_eq!(
        drawn[0].min.y, drawn[1].min.y,
        "two buttons should share a row: {drawn:?}"
    );

    let row = drawn[0].union(drawn[1]);
    let offset = (row.center().x - PANEL / 2.0).abs();
    assert!(
        offset < 2.0,
        "row is not centred: centre {}",
        row.center().x
    );
}

/// More buttons than fit wrap onto further lines rather than overflowing the dialog, so a mode
/// that declares a lot of them still leaves every one reachable.
#[test]
fn too_many_buttons_wrap_instead_of_overflowing() {
    let (rects, _) = layout(8);
    let drawn = button_rects(&rects);
    assert_eq!(drawn.len(), 8);

    let rows: std::collections::BTreeSet<i32> =
        drawn.iter().map(|rect| rect.min.y as i32).collect();
    assert!(rows.len() > 1, "8 buttons did not wrap: {drawn:?}");

    for rect in &drawn {
        assert!(
            rect.max.x <= PANEL,
            "button {rect:?} overflows the {PANEL}-wide dialog"
        );
    }
}

/// The row sits directly beneath the element above it.
///
/// `left_to_right(Align::Center)` sets the *cross* align, and the row's region spans all the
/// height left in the dialog — so centring stranded the buttons in the middle of the empty
/// space below the content instead of under it.
#[test]
fn the_button_row_follows_the_element_above_it() {
    let (rects, _) = layout(2);
    let drawn = button_rects(&rects);

    // The text field: full-width-ish, but not the panel background.
    let input = rects
        .iter()
        .filter(|rect| rect.width() > PANEL / 2.0 && rect.height() < 40.0)
        .max_by(|a, b| a.min.y.total_cmp(&b.min.y))
        .expect("the input should have been drawn");

    let gap = drawn[0].min.y - input.max.y;
    assert!(
        (0.0..=24.0).contains(&gap),
        "buttons are {gap} below the input, which is not directly beneath it"
    );
}
