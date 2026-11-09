//! Everything the UI calls that isn't UI: config, packs, modes, and the supervisor. No Tauri
//! commands or DTOs -- views call these directly with `shared`'s own types.

pub mod diagnostics;
pub mod hardware;
pub mod modes;
pub mod pack;
pub mod schedule;
pub mod supervisor;
pub mod themes;
pub mod updates;
pub mod wallpaper;
