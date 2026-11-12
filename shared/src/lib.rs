pub mod attribution;
pub mod audio;

pub mod behaviour;
pub mod binaries;
pub mod db;
pub mod encode;
pub mod input_monitoring;
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

/// `Lewdware.app`'s bundle identifier (`deploy/macos/build_installer.sh`). The engine and the
/// supervisor run from executables inside it, and send their notifications as it.
pub const MACOS_BUNDLE_ID: &str = "com.lewdware";
