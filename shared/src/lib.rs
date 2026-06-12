#[cfg(feature = "dioxus")]
pub mod components;
pub mod db;
pub mod encode;
pub mod mode;
pub mod pack_config;
pub mod read_config;
pub mod read_pack;
pub mod target;
pub mod user_config;
pub mod utils;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");
