use std::sync::Arc;

use anyhow::Result;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowLevel};

use super::opts::WindowOpts;

// ── Linux implementation ──────────────────────────────────────────────────────

/// On Linux we keep two pools (one per X11 visual depth) of hidden Dock-type windows.
/// Windows are moved offscreen rather than unmapped/destroyed to avoid the KWin strut
/// relayout freeze that Dock-type windows trigger on both XUnmapWindow and XDestroyWindow.
#[cfg(target_os = "linux")]
pub struct WindowPool {
    transparent: Vec<Arc<Window>>,
    opaque: Vec<Arc<Window>>,
}

#[cfg(target_os = "linux")]
impl WindowPool {
    pub fn new() -> Self {
        Self {
            transparent: Vec::new(),
            opaque: Vec::new(),
        }
    }

    /// Return a window ready to host new content, either from the pool or freshly created.
    pub fn acquire(
        &mut self,
        opts: &WindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<Arc<Window>> {
        let pool = if opts.transparent {
            &mut self.transparent
        } else {
            &mut self.opaque
        };
        if let Some(window) = pool.pop() {
            let _ = window.request_inner_size(LogicalSize::new(opts.outer_width, opts.outer_height));
            Ok(window)
        } else {
            new_window(opts, event_loop)
        }
    }

    /// Park the window offscreen and return it to the pool for later reuse.
    pub fn release(&mut self, window: Arc<Window>, transparent: bool) {
        window.set_outer_position(LogicalPosition::new(-32000i32, -32000i32));
        if transparent {
            self.transparent.push(window);
        } else {
            self.opaque.push(window);
        }
    }
}

// ── Non-Linux dummy ───────────────────────────────────────────────────────────

/// On non-Linux platforms there is no compositor freeze issue, so this is a simple
/// pass-through: every acquire creates a fresh window, every release drops it.
#[cfg(not(target_os = "linux"))]
pub struct WindowPool;

#[cfg(not(target_os = "linux"))]
impl WindowPool {
    pub fn new() -> Self {
        Self
    }

    pub fn acquire(
        &mut self,
        opts: &WindowOpts,
        event_loop: &ActiveEventLoop,
    ) -> Result<Arc<Window>> {
        new_window(opts, event_loop)
    }

    pub fn release(&mut self, window: Arc<Window>, _transparent: bool) {
        drop(window);
    }
}

// ── Shared window creation ────────────────────────────────────────────────────

fn new_window(opts: &WindowOpts, event_loop: &ActiveEventLoop) -> Result<Arc<Window>> {
    #[allow(unused_mut)]
    let mut attrs = WindowAttributes::default()
        .with_position(opts.position)
        .with_inner_size(LogicalSize::new(opts.outer_width, opts.outer_height))
        .with_decorations(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_resizable(false)
        .with_visible(false)
        .with_transparent(opts.transparent);

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::{WindowAttributesExtX11, WindowType};
        attrs = attrs.with_x11_window_type(vec![WindowType::Dock]);
    }

    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        attrs = attrs.with_skip_taskbar(true);
    }

    Ok(Arc::new(event_loop.create_window(attrs)?))
}
