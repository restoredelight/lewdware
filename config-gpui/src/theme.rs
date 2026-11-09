//! The shared design tokens (`shared_gpui::theme`), plus the window themes' faces, which only the
//! Window style previews draw with.

pub use shared_gpui::theme::*;

use gpui_kit::App;

/// The faces window themes name. The same files the engine draws with, so a preview's text
/// matches a real popup's. Inter is already loaded by the shared theme.
const WINDOW_THEME_FONTS: [&[u8]; 9] = [
    include_bytes!("../../assets/fonts/Ubuntu-Light.ttf"),
    include_bytes!("../../assets/fonts/W95FA.otf"),
    include_bytes!("../../assets/fonts/Selawik-Regular.ttf"),
    include_bytes!("../../assets/fonts/Cantarell-Regular.otf"),
    include_bytes!("../../assets/fonts/NotoSans.ttf"),
    include_bytes!("../../assets/fonts/LiberationSans-Regular.ttf"),
    include_bytes!("../../assets/fonts/LiberationSans-Bold.ttf"),
    include_bytes!("../../assets/fonts/SourceSans3-Regular.ttf"),
    include_bytes!("../../assets/fonts/SourceSans3-Semibold.ttf"),
];

pub fn install(cx: &mut App) {
    shared_gpui::theme::install(cx);
    add_fonts(cx, &WINDOW_THEME_FONTS);
}

/// The family and weight a window theme's face is registered under. gpui matches on the
/// typographic family (name ID 16) where a font has one, so a separately shipped weight is asked
/// for as "Ubuntu" at 300, not as "Ubuntu Light".
pub fn face(face: shared::theme::Face) -> (&'static str, gpui_kit::FontWeight) {
    use gpui_kit::FontWeight;
    use shared::theme::Face;
    match face {
        Face::Default => ("Ubuntu", FontWeight::LIGHT),
        Face::Pixel => ("W95FA", FontWeight::NORMAL),
        Face::Selawik => ("Selawik", FontWeight::NORMAL),
        Face::Cantarell => ("Cantarell", FontWeight::NORMAL),
        Face::Inter => (FONT_SANS, FontWeight::NORMAL),
        Face::NotoSans => ("Noto Sans", FontWeight::NORMAL),
        Face::LiberationSans => ("Liberation Sans", FontWeight::NORMAL),
        Face::LiberationSansBold => ("Liberation Sans", FontWeight::BOLD),
        Face::SourceSans => ("Source Sans 3", FontWeight::NORMAL),
        Face::SourceSansSemibold => ("Source Sans 3", FontWeight::SEMIBOLD),
        // Text-popup faces no theme can name.
        Face::Mono | Face::Display => (FONT_SANS, FontWeight::NORMAL),
    }
}
