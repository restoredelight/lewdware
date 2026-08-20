use std::collections::HashMap;

use mlua::{IntoLua, LuaSerdeExt, serde::SerializeOptions};
use serde::{Deserialize, Serialize};
use shared::monitor::MonitorRegion;
use winit::{event_loop::ActiveEventLoop, monitor::MonitorHandle};

use crate::error::MonitorError;

pub struct Monitors {
    disabled: Vec<String>,
    /// The user's chosen sub-area of each monitor, keyed like `disabled`. Read at startup and
    /// never reloaded: a session runs on the config it was started with.
    regions: HashMap<String, MonitorRegion>,
    by_platform: HashMap<MonitorId, Monitor>,
    by_id: HashMap<u64, MonitorId>,
    /// Each monitor's region origin, in logical pixels from that monitor's top-left corner. Kept
    /// out of `Monitor` deliberately: that struct goes to Lua, where the region *is* the monitor
    /// and an offset into a screen the mode cannot see would be meaningless.
    region_origins: HashMap<u64, (i32, i32)>,
    primary_monitor: Option<(MonitorId, Monitor)>,
    current_id: u64,
}

/// A monitor as a *mode* sees it — which is to say, the region of it the user allowed. `width` and
/// `height` are the region's, not the panel's, and a window's `x`/`y` are relative to the region's
/// top-left corner; `LewdwareApp::finalize_window_opts` adds the region origin on the way to a
/// real window. See `shared::monitor::MonitorRegion`.
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

fn monitor_id(monitor: &MonitorHandle) -> String {
    // If a monitor is not given a name, compute an id from it's position and size
    monitor.name().unwrap_or_else(|| {
        let size = monitor.size();
        let position = monitor.position();
        shared::monitor::geometry_id(size.width, size.height, position.x, position.y)
    })
}

/// Prints this process's view of the monitors as JSON, then exits.
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
                    let position = monitor.position();
                    let id = monitor_id(&monitor);

                    shared::monitor::MonitorInfo {
                        name: id.clone(),
                        id,
                        width: size.width,
                        height: size.height,
                        primary: primary.as_ref().is_some_and(|p| *p == monitor),
                        x: position.x,
                        y: position.y,
                        scale_factor: monitor.scale_factor(),
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

impl Monitors {
    pub fn new(disabled: Vec<String>, regions: HashMap<String, MonitorRegion>) -> Self {
        Self {
            disabled,
            regions,
            by_platform: HashMap::new(),
            by_id: HashMap::new(),
            region_origins: HashMap::new(),
            primary_monitor: None,
            current_id: 0,
        }
    }

    /// Where the monitor's region starts, in logical pixels from the monitor's own top-left
    /// corner. Only meaningful straight after a `list`/`primary` call, which is the only thing
    /// that refreshes it.
    ///
    /// An id this doesn't know is not an error worth failing a spawn over: the window lands at the
    /// monitor's origin, which is exactly where it would have landed before regions existed.
    pub fn region_origin(&self, id: u64) -> (i32, i32) {
        self.region_origins.get(&id).copied().unwrap_or((0, 0))
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
        let mut region_origins = HashMap::new();

        for monitor in monitors {
            let config_id = monitor_id(&monitor);

            if self.disabled.contains(&config_id) {
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

            let region = self
                .regions
                .get(&config_id)
                .copied()
                .unwrap_or_default()
                .resolve(size.width, size.height);

            let monitor = Monitor {
                id,
                primary: false,
                width: region.width,
                height: region.height,
                scale_factor,
            };

            #[allow(clippy::clone_on_copy)]
            by_platform.insert(platform_id.clone(), monitor);
            by_id.insert(id, platform_id);
            region_origins.insert(id, (region.x, region.y));
        }

        self.by_platform = by_platform;
        self.by_id = by_id;
        self.region_origins = region_origins;

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
