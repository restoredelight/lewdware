use std::collections::HashMap;

use mlua::{IntoLua, LuaSerdeExt, SerializeOptions};
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use winit::{event_loop::ActiveEventLoop, monitor::MonitorHandle};

use crate::error::MonitorError;

pub struct Monitors {
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

// #[derive(PartialEq, Eq, Hash, Clone)]
// enum MonitorId {
//     Number(u32),
//     String(String),
// }

impl Monitors {
    pub fn new() -> Self {
        Self {
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

    pub fn get(&mut self, id: u64, event_loop: &ActiveEventLoop) -> Result<Monitor> {
        self.refresh(event_loop);

        self.by_id
            .get(&id)
            .and_then(|platform_id| self.by_platform.get(platform_id))
            .cloned()
            .ok_or(MonitorError::MonitorNotFound)
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

    pub fn random(&mut self, event_loop: &ActiveEventLoop) -> Result<Monitor> {
        let monitors = self.list(event_loop);

        let mut rng = rand::rng();
        monitors
            .choose(&mut rng)
            .ok_or(MonitorError::NoAvailableMonitors)
            .cloned()
    }

    fn refresh(&mut self, event_loop: &ActiveEventLoop) {
        let monitors: Vec<_> = event_loop.available_monitors().collect();

        let primary_monitor = event_loop.primary_monitor();

        let mut by_platform = HashMap::new();
        let mut by_id = HashMap::new();

        for monitor in monitors {
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
        use winit::platform::MonitorHandleExtMacOS;

        monitor.native_id()
    }
    #[cfg(target_os = "windows")]
    {
        use winit::platform::windows::MonitorHandleExtWindows;

        monitor.native_id()
    }
}
