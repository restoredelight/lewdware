pub mod attribution;
pub mod audio;

pub mod behaviour;
pub mod binaries;
pub mod db;
pub mod encode;
#[cfg(feature = "ipc")]
pub mod ipc;
pub mod logging;
pub mod mode;
pub mod monitor;
mod once;
pub mod pack;
pub mod schedule;
pub mod tags;
pub mod theme;
pub mod user_config;
pub mod utils;
pub mod wallpaper;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
