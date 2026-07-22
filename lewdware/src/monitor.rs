use std::collections::HashMap;

use mlua::{IntoLua, LuaSerdeExt, serde::SerializeOptions};
use serde::{Deserialize, Serialize};
use winit::{event_loop::ActiveEventLoop, monitor::MonitorHandle};

use crate::error::MonitorError;

pub struct Monitors {
    disabled: Vec<String>,
    by_platform: HashMap<MonitorId, Monitor>,
    by_id: HashMap<u64, MonitorId>,
    primary_monitor: Option<(MonitorId, Monitor)>,
    current_id: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Monitor {
    pub id: u64,
    pub primary: bool,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

impl IntoLua for Monitor {
    fn into_lua(self, lua: &mlua::Lua) -> mlua::Result<mlua::Value> {
        lua.to_value_with(&self, SerializeOptions::new().serialize_none_to_null(false))
    }
}

type Result<T, E = MonitorError> = std::result::Result<T, E>;

/// The identity `AppConfig::disabled_monitors` is keyed on.
///
/// Used by both `Monitors::refresh` and `list_monitors`, so what the config app stores is exactly
/// what gets compared here. Keeping these in one place is the point: they were previously derived
/// in two processes on two different display backends, and never matched.
///
/// Wayland sessions can report no name at all. Falling back to geometry keeps such a monitor
/// addressable -- matching on `name()` alone meant an unnamed monitor could never be disabled.
fn monitor_id(monitor: &MonitorHandle) -> String {
    monitor.name().unwrap_or_else(|| {
        let size = monitor.size();
        let position = monitor.position();
        shared::monitor::geometry_id(size.width, size.height, position.x, position.y)
    })
}

/// Prints this process's view of the monitors as JSON, then exits. Driven by
/// `shared::monitor::LIST_MONITORS_FLAG`.
///
/// The config app can't work these out for itself: it's a native Wayland app, while the engine
/// forces winit onto XWayland, and the two disagree about both names and geometry (see
/// `shared::monitor`). So it asks us, and stores whatever we say -- both sides go through
/// `monitor_id`, so `disabled_monitors` compares equal in `refresh` by construction.
///
/// This deliberately builds the same event loop as `main`, forced X11 and all: a probe on a
/// different backend would report different identities and reintroduce the bug it exists to fix.
pub fn list_monitors() -> anyhow::Result<()> {
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::EventLoop;
    use winit::window::WindowId;

    struct Probe;

    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let primary = event_loop.primary_monitor();

            let monitors: Vec<shared::monitor::MonitorInfo> = event_loop
                .available_monitors()
                .map(|monitor| {
                    let size = monitor.size();
                    let id = monitor_id(&monitor);

                    shared::monitor::MonitorInfo {
                        name: id.clone(),
                        id,
                        width: size.width,
                        height: size.height,
                        primary: primary.as_ref().is_some_and(|p| *p == monitor),
                    }
                })
                .collect();

            match serde_json::to_string(&monitors) {
                Ok(json) => println!("{json}"),
                Err(err) => eprintln!("could not serialise monitors: {err}"),
            }

            event_loop.exit();
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }

    let mut builder = EventLoop::with_user_event();

    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;

        builder.with_x11();
    }

    builder.build()?.run_app(&mut Probe)?;

    Ok(())
}

// #[derive(PartialEq, Eq, Hash, Clone)]
// enum MonitorId {
//     Number(u32),
//     String(String),
// }

impl Monitors {
    pub fn new(disabled: Vec<String>) -> Self {
        Self {
            disabled,
            by_platform: HashMap::new(),
            by_id: HashMap::new(),
            primary_monitor: None,
            current_id: 0,
        }
    }

    pub fn get_handle(&self, id: u64, event_loop: &ActiveEventLoop) -> Option<MonitorHandle> {
        let monitor_id = self.by_id.get(&id)?;

        event_loop
            .available_monitors()
            .find(|monitor| platform_id(monitor) == *monitor_id)
    }

    pub fn primary(&mut self, event_loop: &ActiveEventLoop) -> Result<Monitor> {
        self.refresh(event_loop);

        Ok(self
            .primary_monitor
            .as_ref()
            .ok_or(MonitorError::NoAvailableMonitors)?
            .1
            .clone())
    }

    pub fn list(&mut self, event_loop: &ActiveEventLoop) -> Vec<Monitor> {
        self.refresh(event_loop);

        self.by_platform.values().cloned().collect()
    }

    fn refresh(&mut self, event_loop: &ActiveEventLoop) {
        let monitors: Vec<_> = event_loop.available_monitors().collect();

        let primary_monitor = event_loop
            .primary_monitor()
            .filter(|monitor| !self.disabled.contains(&monitor_id(monitor)));

        let mut by_platform = HashMap::new();
        let mut by_id = HashMap::new();

        for monitor in monitors {
            if self.disabled.contains(&monitor_id(&monitor)) {
                continue;
            }

            let platform_id = platform_id(&monitor);

            let id = match self.by_platform.get(&platform_id) {
                Some(monitor) => monitor.id,
                None => {
                    let id = self.current_id;
                    self.current_id += 1;
                    id
                }
            };

            let scale_factor = monitor.scale_factor();
            let size = monitor.size().to_logical(scale_factor);

            let monitor = Monitor {
                id,
                primary: false,
                width: size.width,
                height: size.height,
                scale_factor,
            };

            #[allow(clippy::clone_on_copy)]
            by_platform.insert(platform_id.clone(), monitor);
            by_id.insert(id, platform_id);
        }

        self.by_platform = by_platform;
        self.by_id = by_id;

        self.primary_monitor = primary_monitor
            .and_then(|monitor| {
                let platform_id = platform_id(&monitor);

                self.by_platform.get_mut(&platform_id).map(|monitor| {
                    monitor.primary = true;
                    (platform_id, monitor.clone())
                })
            })
            .or_else(|| {
                self.primary_monitor.as_ref().and_then(|(platform_id, _)| {
                    self.by_platform.get_mut(platform_id).map(|monitor| {
                        monitor.primary = true;

                        #[allow(clippy::clone_on_copy)]
                        (platform_id.clone(), monitor.clone())
                    })
                })
            })
            .or_else(|| {
                self.by_platform
                    .iter_mut()
                    .next()
                    .map(|(platform_id, monitor)| {
                        monitor.primary = true;

                        #[allow(clippy::clone_on_copy)]
                        (platform_id.clone(), monitor.clone())
                    })
            })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
type MonitorId = u32;

#[cfg(target_os = "windows")]
type MonitorId = String;

fn platform_id(monitor: &MonitorHandle) -> MonitorId {
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::MonitorHandleExtX11;

        monitor.native_id()
    }
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::MonitorHandleExtMacOS;

        monitor.native_id()
    }
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::MonitorHandleExtWindows;

        monitor.native_id()
    }
}
