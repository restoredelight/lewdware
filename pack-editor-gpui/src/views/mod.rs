//! The views the sidebar switches between, one module per track of the port:
//!
//! | Module | Views | Track |
//! | --- | --- | --- |
//! | `library` | Popups, Audio, All media, Tags, Artists | Media library |
//! | `content` | Content | Behaviour: content |
//! | `timeline` | Timeline | Behaviour: timeline |
//! | `pack` | Modes, Metadata | Shell and lifecycle |
//!
//! Each module's `create` makes the view for the `View`s it owns. The shell creates a view the
//! first time it's shown and keeps it until another pack (or a Save As copy) is opened.

pub mod content;
pub mod library;
pub mod pack;
pub mod timeline;

use gpui_kit::{AnyView, App, Entity, Window};

use crate::editor::Editor;
use crate::shell::View;

pub fn create(view: View, editor: Entity<Editor>, window: &mut Window, cx: &mut App) -> AnyView {
    match view {
        View::Popups | View::Audio | View::AllMedia | View::Tags | View::Artists => {
            library::create(view, editor, window, cx)
        }
        View::Content => content::create(view, editor, window, cx),
        View::Timeline => timeline::create(view, editor, window, cx),
        View::Modes | View::Metadata => pack::create(view, editor, window, cx),
    }
}
