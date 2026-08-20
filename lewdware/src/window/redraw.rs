use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use winit::event_loop::EventLoopProxy;
use winit::window::Window;

use crate::app::UserEvent;

/// A cloneable per-window dirty flag. On Windows, the first redraw request across the whole app
/// also posts a user event to wake winit; the rest are batched until `about_to_wait` drains them.
/// Other platforms retain winit's normal `request_redraw` path.
#[derive(Clone)]
pub struct RedrawRequester {
    /// `None` only in tests, which have no window to notify.
    #[cfg(not(target_os = "windows"))]
    window: Option<Arc<Window>>,
    redraw: Arc<AtomicBool>,
    #[cfg(target_os = "windows")]
    event_loop_proxy: EventLoopProxy<UserEvent>,
    #[cfg(target_os = "windows")]
    wakeup_pending: Arc<AtomicBool>,
}

impl RedrawRequester {
    pub fn new(
        _window: Arc<Window>,
        _event_loop_proxy: EventLoopProxy<UserEvent>,
        _wakeup_pending: Arc<AtomicBool>,
    ) -> Self {
        Self {
            #[cfg(not(target_os = "windows"))]
            window: Some(_window),
            redraw: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "windows")]
            event_loop_proxy: _event_loop_proxy,
            #[cfg(target_os = "windows")]
            wakeup_pending: _wakeup_pending,
        }
    }

    pub fn request_redraw(&self) {
        self.redraw.store(true, Ordering::Release);

        #[cfg(target_os = "windows")]
        if !self.wakeup_pending.swap(true, Ordering::AcqRel)
            && self
                .event_loop_proxy
                .send_event(UserEvent::RedrawRequested)
                .is_err()
        {
            // Permit a retry if the event loop is temporarily unavailable. Once it has closed,
            // this value no longer matters.
            self.wakeup_pending.store(false, Ordering::Release);
        }

        #[cfg(not(target_os = "windows"))]
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// A requester with nothing to notify, for tests that build window pieces headlessly.
    #[cfg(test)]
    pub fn detached() -> Self {
        Self {
            #[cfg(not(target_os = "windows"))]
            window: None,
            redraw: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "windows")]
            event_loop_proxy: unreachable!("detached() is not used on Windows"),
            #[cfg(target_os = "windows")]
            wakeup_pending: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn take_requested(&self) -> bool {
        self.redraw.swap(false, Ordering::AcqRel)
    }
}
