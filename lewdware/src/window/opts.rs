use winit::dpi::LogicalPosition;

use crate::monitor::Monitor;

/// Fully resolved options for creating or reconfiguring a window.
///
/// Computed from [`SpawnWindowOpts`](crate::lua::SpawnWindowOpts) and other spawn-time inputs
/// once the monitor, sizes, and GPU availability are all known.
pub struct WindowOpts {
    /// Absolute screen position (monitor offset + x + y). Used for initial placement and for
    /// repositioning a pooled window.
    pub position: LogicalPosition<u32>,
    /// Monitor-relative x offset. Stored in `InnerWindow` for the move/animation system.
    pub x: u32,
    /// Monitor-relative y offset.
    pub y: u32,
    /// Inner content width (excluding decoration border).
    pub width: u32,
    /// Inner content height (excluding decoration border and header).
    pub height: u32,
    /// Outer width (including decoration border).
    pub outer_width: u32,
    /// Outer height (including decoration border and header).
    pub outer_height: u32,
    /// Whether to use GPU (wgpu) rendering. Already AND'd with wgpu availability.
    pub gpu: bool,
    /// Whether to request an alpha-capable surface. Already AND'd with `gpu`.
    pub transparent: bool,
    /// Whether to force opaque rendering even if the surface has alpha (`transparent = Some(false)`
    /// was set explicitly in Lua).
    pub force_opaque: bool,
    /// Initial opacity, in [0, 1].
    pub opacity: f32,
    pub click_through: bool,
    pub visible: bool,
    pub decorations: bool,
    pub title: Option<String>,
    pub closeable: bool,
    pub monitor: Monitor,
}
