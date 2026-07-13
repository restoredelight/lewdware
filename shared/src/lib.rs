pub mod behaviour;
pub mod child;
pub mod db;
pub mod encode;
pub mod ipc;
pub mod logging;
pub mod mode;
mod once;
pub mod read_pack;
pub mod user_config;
pub mod utils;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
