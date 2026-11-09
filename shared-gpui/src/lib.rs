#![recursion_limit = "256"]
//! What the gpui apps (`config-gpui`, and the pack editor's port) have in common: the design
//! guide's tokens and fonts (`theme`), the components that carry it (`ui`), and the tokio runtime
//! that background work runs on (`runtime`).
//!
//! The Svelte equivalent is `shared-ui/`. Read `shared-ui/DESIGN.md` before changing anything here.

pub mod runtime;
pub mod theme;
pub mod ui;
