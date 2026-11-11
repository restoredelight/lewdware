#![recursion_limit = "256"]
//! What the gpui apps (`config-gpui`, `pack-editor-gpui`) have in common: the design
//! guide's tokens and fonts (`theme`), the components that carry it (`ui`), Lucide icons (`icons`), and the tokio runtime
//! that background work runs on (`runtime`).
//!
//! Read `shared-ui/DESIGN.md` before changing anything here.

pub mod icons;
pub mod runtime;
pub mod theme;
pub mod ui;
