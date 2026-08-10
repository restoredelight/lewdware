use std::sync::{Arc, LazyLock};

use ab_glyph::FontArc;
use egui::{Align, FontData, FontDefinitions, FontFamily, FontId, Vec2, text::LayoutJob};

use shared::theme::Face;

use crate::lua::{TextAlign, TextFont};

impl From<TextFont> for Face {
    fn from(font: TextFont) -> Self {
        match font {
            TextFont::Default => Self::Default,
            TextFont::Mono => Self::Mono,
            TextFont::Display => Self::Display,
            TextFont::Pixel => Self::Pixel,
        }
    }
}

/// A face bundled with the engine: its bytes, and the names egui knows it by.
struct Bundled {
    bytes: &'static [u8],
    /// The key the font data is registered under in a `FontDefinitions`.
    data_name: &'static str,
    /// The `FontFamily::Name` that resolves to it.
    family: &'static str,
}

/// The bundled face's data.
///
/// Every face is a file in `assets/fonts/`, including the two egui ships with — the engine used to
/// reach into `FontDefinitions::default()` for those, which meant `config/` had nothing it could
/// point an `@font-face` at and its preview of `plain` fell back to whatever sans the machine had.
/// Bundling them makes the two renderers agree by construction.
///
/// A free function rather than a method: [`Face`] is a description and lives with the themes, so
/// only the crate that actually embeds the bytes can hold the mapping to them.
fn bundled(face: Face) -> Bundled {
    match face {
        // egui's own two faces, now named explicitly rather than inherited.
        Face::Default => Bundled {
            bytes: include_bytes!("../../assets/fonts/Ubuntu-Light.ttf"),
            data_name: "Ubuntu-Light",
            family: "lewdware-default",
        },
        Face::Mono => Bundled {
            bytes: include_bytes!("../../assets/fonts/Hack-Regular.ttf"),
            data_name: "Hack",
            family: "lewdware-mono",
        },
        Face::Display => Bundled {
            bytes: include_bytes!("../../assets/fonts/Anton-Regular.ttf"),
            data_name: "Anton-Regular",
            family: "lewdware-display",
        },
        Face::Pixel => Bundled {
            bytes: include_bytes!("../../assets/fonts/W95FA.otf"),
            data_name: "W95FA",
            family: "lewdware-pixel",
        },
        Face::Selawik => Bundled {
            bytes: include_bytes!("../../assets/fonts/Selawik-Regular.ttf"),
            data_name: "Selawik-Regular",
            family: "lewdware-selawik",
        },
        Face::Inter => Bundled {
            bytes: include_bytes!("../../assets/fonts/Inter-Regular.ttf"),
            data_name: "Inter-Regular",
            family: "lewdware-inter",
        },
        Face::Cantarell => Bundled {
            bytes: include_bytes!("../../assets/fonts/Cantarell-Regular.otf"),
            data_name: "Cantarell-Regular",
            family: "lewdware-cantarell",
        },
        Face::NotoSans => Bundled {
            bytes: include_bytes!("../../assets/fonts/NotoSans.ttf"),
            data_name: "NotoSans",
            family: "lewdware-noto-sans",
        },
        Face::LiberationSans => Bundled {
            bytes: include_bytes!("../../assets/fonts/LiberationSans-Regular.ttf"),
            data_name: "LiberationSans-Regular",
            family: "lewdware-liberation-sans",
        },
        Face::LiberationSansBold => Bundled {
            bytes: include_bytes!("../../assets/fonts/LiberationSans-Bold.ttf"),
            data_name: "LiberationSans-Bold",
            family: "lewdware-liberation-sans-bold",
        },
        Face::SourceSans => Bundled {
            bytes: include_bytes!("../../assets/fonts/SourceSans3-Regular.ttf"),
            data_name: "SourceSans3-Regular",
            family: "lewdware-source-sans",
        },
        Face::SourceSansSemibold => Bundled {
            bytes: include_bytes!("../../assets/fonts/SourceSans3-Semibold.ttf"),
            data_name: "SourceSans3-Semibold",
            family: "lewdware-source-sans-semibold",
        },
    }
}

/// The font definitions needed to render `font`.
///
/// Always `Some`: every face is bundled, so egui is always told explicitly which file to draw
/// with rather than falling through to whichever of its own it would otherwise pick.
pub fn build_font_definitions(font: impl Into<Face>) -> Option<FontDefinitions> {
    let bundled = bundled(font.into());

    let mut definitions = FontDefinitions::default();
    definitions.font_data.insert(
        bundled.data_name.to_owned(),
        Arc::new(FontData::from_static(bundled.bytes)),
    );

    // Prepended to the proportional fallback chain rather than replacing it, so anything the face
    // lacks — most of Unicode, for a UI font — still comes out of egui's own fonts.
    let mut fallback = definitions
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    fallback.insert(0, bundled.data_name.to_owned());

    definitions
        .families
        .insert(FontFamily::Name(bundled.family.into()), fallback);

    Some(definitions)
}

/// One of egui's own fonts, as an `ab_glyph` face.
///
/// Parsed faces, keyed by the bundled data name. A face is parsed once and shared thereafter;
/// hundreds of windows may each have a header.
static BUNDLED_CHROME_FONTS: LazyLock<
    std::sync::Mutex<std::collections::HashMap<&'static str, Option<FontArc>>>,
> = LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// The face a window's title bar draws `font` with, or `None` if it could not be loaded — in which
/// case the title is simply not drawn, exactly as before this was themeable.
pub fn chrome_font(font: impl Into<Face>) -> Option<FontArc> {
    let bundled = bundled(font.into());

    let mut cache = BUNDLED_CHROME_FONTS
        .lock()
        .expect("chrome font cache poisoned");
    cache
        .entry(bundled.data_name)
        .or_insert_with(|| FontArc::try_from_slice(bundled.bytes).ok())
        .clone()
}

pub fn font_family(font: impl Into<Face>) -> FontFamily {
    // Every face is a named family of its own now, including the two that used to fall through to
    // egui's `Proportional`/`Monospace`.
    FontFamily::Name(bundled(font.into()).family.into())
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
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            font_id,
            ..Default::default()
        },
    );
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
