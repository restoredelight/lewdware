//! One view per settings page. Each is created the first time its tab is shown and kept, so a
//! page's own state (a probe result, a slider mid-drag) survives switching away and back.

pub mod audio;
pub mod diagnostics;
pub mod monitors;
pub mod pack_mode;
pub mod safety;
pub mod scheduling;
pub mod theme_preview;
pub mod window_style;
