pub mod attribution;
pub mod audio;
#[cfg(feature = "autostart")]
pub mod autostart;
pub mod behaviour;
pub mod child;
pub mod db;
pub mod encode;
pub mod ipc;
pub mod logging;
pub mod mode;
pub mod monitor;
mod once;
pub mod read_pack;
pub mod schedule;
pub mod tags;
pub mod theme;
pub mod user_config;
pub mod utils;
pub mod wallpaper;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
