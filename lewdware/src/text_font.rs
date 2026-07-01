use std::sync::Arc;

use egui::{Align, FontData, FontDefinitions, FontFamily, FontId, Vec2, text::LayoutJob};

use crate::lua::{TextAlign, TextFont};

static DISPLAY_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/Anton-Regular.ttf");

const DISPLAY_FAMILY_NAME: &str = "lewdware-display";

/// Returns the custom font definitions needed to render `font`, or `None` if the egui defaults
/// (used unmodified for `TextFont::Default` and `TextFont::Mono`) are sufficient.
pub fn build_font_definitions(font: TextFont) -> Option<FontDefinitions> {
    match font {
        TextFont::Default | TextFont::Mono => None,
        TextFont::Display => {
            let mut definitions = FontDefinitions::default();

            definitions.font_data.insert(
                "Anton-Regular".to_owned(),
                Arc::new(FontData::from_static(DISPLAY_FONT_BYTES)),
            );

            // Fall back to the default proportional font for glyphs Anton doesn't cover.
            let mut fallback = definitions
                .families
                .get(&FontFamily::Proportional)
                .cloned()
                .unwrap_or_default();
            fallback.insert(0, "Anton-Regular".to_owned());

            definitions
                .families
                .insert(FontFamily::Name(DISPLAY_FAMILY_NAME.into()), fallback);

            Some(definitions)
        }
    }
}

pub fn font_family(font: TextFont) -> FontFamily {
    match font {
        TextFont::Default => FontFamily::Proportional,
        TextFont::Mono => FontFamily::Monospace,
        TextFont::Display => FontFamily::Name(DISPLAY_FAMILY_NAME.into()),
    }
}

pub fn to_egui_align(align: TextAlign) -> Align {
    match align {
        TextAlign::Left => Align::Min,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Max,
    }
}

/// Measure the size text would take up, in logical points, when laid out with `font`/`font_size`
/// and wrapped at `wrap_width` (pass `f32::INFINITY` for the natural, unwrapped size).
///
/// Used to size a text popup before the window (and its real egui `Context`) exists.
pub fn measure(text: &str, font: TextFont, font_size: f32, wrap_width: f32) -> Vec2 {
    let ctx = egui::Context::default();

    if let Some(definitions) = build_font_definitions(font) {
        ctx.set_fonts(definitions);
    }

    // `Context::fonts`/`fonts_mut` panic until the first pass has run, so do an empty pass
    // first purely to initialize the font atlas with the definitions set above.
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});

    let font_id = FontId::new(font_size, font_family(font));
    let mut job = LayoutJob::single_section(text.to_owned(), egui::TextFormat {
        font_id,
        ..Default::default()
    });
    job.wrap.max_width = wrap_width;

    ctx.fonts_mut(|f| f.layout_job(job)).size()
}

/// Unit vectors evenly spaced around a circle, used to fake a stroke/outline by repainting the
/// same galley at `radius * offset` for each one. Unlike a fixed 8-direction (N/S/E/W + diagonal)
/// set, these are normalized to a consistent radius — un-normalized diagonals (e.g. `(1, 1)`,
/// magnitude √2) land further from the glyph than cardinal ones, which is visible as a lumpy,
/// octagon-ish stroke rather than a round one, and gets worse the larger the radius is.
pub fn outline_offsets(count: usize) -> impl Iterator<Item = Vec2> {
    (0..count).map(move |i| {
        let angle = (i as f32 / count as f32) * std::f32::consts::TAU;
        Vec2::new(angle.cos(), angle.sin())
    })
}

/// How many outline samples to use for a stroke of the given radius (in logical points). Thin
/// strokes look fine with few samples, but the gap between samples (arc length ≈
/// `2π · radius / count`) grows with the radius, so thicker strokes need more of them to avoid
/// visible faceting between samples.
pub fn outline_sample_count(radius: f32) -> usize {
    ((radius * 4.2).ceil() as usize).clamp(8, 24)
}
