//! Media previews without a webview or media server. Decoding and startup happen on a worker;
//! the GPUI view polls at vsync and explicitly owns every image it uploads to the window atlas.

pub mod audio;
pub mod bounded_input;
pub mod convert;
mod element;
pub mod source;
pub mod video;
mod view;
mod worker;

pub use source::MediaSource;
pub use view::{CONTROLS_HEIGHT, MediaView};
pub use worker::{MediaKind, PreviewSource};
