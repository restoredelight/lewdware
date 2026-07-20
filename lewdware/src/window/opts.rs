use winit::dpi::LogicalPosition;

use crate::lua::PopupSpawnOpts;
use crate::monitor::Monitor;

pub struct WindowOpts {
    pub popup_opts: PopupSpawnOpts,
    pub monitor: Monitor,
    /// Absolute screen position (monitor offset + `popup.x`/`popup.y`).
    pub position: LogicalPosition<i32>,
}
