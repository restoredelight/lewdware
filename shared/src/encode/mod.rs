//! Media probing and encoding for packs.
//!
//! `info`, `hash` and `parse_media_info` are plain data and always available. Everything that
//! touches media -- probing, encoding, the hardware-encoder test, the encode worker -- runs on
//! the linked ffmpeg libraries and sits behind the `encode` feature, which only the pack editor
//! and the converter enable.

#[cfg(feature = "encode")]
mod av;
#[cfg(feature = "encode")]
mod hardware;
mod hash;
mod info;
#[cfg(feature = "encode")]
mod paths;
mod probe;
#[cfg(feature = "encode")]
mod transcode;
#[cfg(feature = "encode")]
mod worker;

#[cfg(feature = "encode")]
pub use hardware::*;
pub use hash::*;
pub use info::*;
#[cfg(feature = "encode")]
pub use paths::*;
pub use probe::*;
#[cfg(feature = "encode")]
pub use transcode::*;
#[cfg(feature = "encode")]
pub use worker::{ENCODE_WORKER_ARG, serve_encode_worker_if_requested, use_encode_worker};

#[cfg(all(test, feature = "encode"))]
mod integration;

#[cfg(test)]
mod tests;
