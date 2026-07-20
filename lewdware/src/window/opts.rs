use winit::dpi::LogicalPosition;

use crate::lua::PopupSpawnOpts;
use crate::monitor::Monitor;

/// Fully resolved options for creating or reconfiguring a window: everything already resolved
/// against a monitor snapshot on the Lua thread (`popup` -- sizes, anchor math, clamping), plus
/// the two things that can only be read fresh on the main thread: which physical [`Monitor`]
/// `popup.monitor_id` currently refers to, and its current absolute `position` (combined with
/// `popup.x`/`popup.y` to place the window). See
/// [`LewdwareApp::finalize_window_opts`](crate::app::LewdwareApp::finalize_window_opts).
pub struct WindowOpts {
    pub popup_opts: PopupSpawnOpts,
    pub monitor: Monitor,
    /// Absolute screen position (monitor offset + `popup.x`/`popup.y`). Used for initial
    /// placement and for repositioning a pooled window.
    pub position: LogicalPosition<i32>,
}
