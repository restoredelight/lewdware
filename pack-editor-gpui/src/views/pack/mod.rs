//! Pack-level settings: the modes a pack bundles and its metadata. Ported from
//! `pack-editor/src/lib/Modes.svelte` and `Options.svelte`.

mod metadata;
mod modes;

use gpui_kit::{AnyView, App, AppContext, Entity, Window};

use crate::editor::Editor;
use crate::shell::View;

pub fn create(view: View, editor: Entity<Editor>, window: &mut Window, cx: &mut App) -> AnyView {
    match view {
        View::Modes => cx.new(|cx| modes::ModesView::new(editor, cx)).into(),
        _ => {
            debug_assert!(matches!(view, View::Metadata));
            cx.new(|cx| metadata::MetadataView::new(editor, window, cx))
                .into()
        }
    }
}
