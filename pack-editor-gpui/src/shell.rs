//! The open pack's window: toolbar (name, save state, import, undo/redo, save, the pack menu),
//! the sidebar's views, the active view, and what's drawn over them -- the import window, the
//! "drop to import" overlay and the Edgeware import warnings. Ported from
//! `pack-editor/src/lib/Editor.svelte` and `EditorToolbar.svelte`.
//!
//! The views themselves live in `views`, one module per track of the port. The shell only
//! switches between them.

use std::collections::HashMap;

use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyView, App, Context, Entity, ExternalPaths, FocusHandle, FontWeight, Global,
    SharedString, Subscription, WeakEntity, Window,
};
use pack_editor_core::changes::{Scope, Scopes};

use crate::app::{ClosePack, DiscardChanges, ImportFiles, ImportFolder, Redo, Save, SaveAs, Undo};
use crate::editor::{Editor, EditorEvent};
use crate::imports::{self, ImportWindow};
use crate::menu::{Menu, MenuEntry};
use crate::theme::{self, c};
use crate::ui::{self, Button, NavItem, Size, Variant};
use crate::views;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum View {
    Popups,
    Audio,
    AllMedia,
    Tags,
    Artists,
    Content,
    Timeline,
    Modes,
    Metadata,
}

impl View {
    pub fn label(self) -> &'static str {
        match self {
            View::Popups => "Popups",
            View::Audio => "Audio",
            View::AllMedia => "All media",
            View::Tags => "Tags",
            View::Artists => "Artists",
            View::Content => "Content",
            View::Timeline => "Timeline",
            View::Modes => "Modes",
            View::Metadata => "Metadata",
        }
    }

    /// What the view shows, so it redraws only when that changes.
    fn scopes(self) -> Scopes {
        match self {
            View::Popups | View::Audio | View::AllMedia => Scope::Media | Scope::Tags,
            View::Tags => Scope::Tags.into(),
            View::Artists => Scope::Artists.into(),
            View::Content => Scope::Entries | Scope::ContentGroups | Scope::Slots,
            View::Timeline => Scope::Timeline | Scope::Slots,
            View::Modes => Scope::Modes | Scope::Metadata | Scope::Timeline,
            View::Metadata => Scope::Metadata.into(),
        }
    }
}

const GROUPS: &[(&str, &[View])] = &[
    ("Media", &[View::Popups, View::Audio, View::AllMedia]),
    ("Organization", &[View::Tags, View::Artists]),
    ("Behaviour", &[View::Content, View::Timeline]),
    ("Pack", &[View::Modes, View::Metadata]),
];

/// The open pack's shell, so a view can switch to another (the Tags view's "Media" menu, the
/// inspector's "Used as"). Set when a shell is made.
struct CurrentShell(WeakEntity<Shell>);

impl Global for CurrentShell {}

/// Switches the open pack's window to `view`. Deferred, so a view can call it from its own
/// handlers even though switching may drop it (see `Shell::show`).
pub fn show_view(view: View, _window: &mut Window, cx: &mut App) {
    let Some(shell) = cx
        .try_global::<CurrentShell>()
        .and_then(|current| current.0.upgrade())
    else {
        return;
    };
    cx.defer(move |cx| shell.update(cx, |shell, cx| shell.show(view, cx)));
}

/// The import window's own state; what it shows comes from the editor.
#[derive(Default)]
struct ImportWindowState {
    minimized: bool,
    errors_open: bool,
    stopping: bool,
}

pub struct Shell {
    editor: Entity<Editor>,
    view: View,
    /// Each view, created when first shown (see `views`).
    views: HashMap<View, AnyView>,
    import_window: ImportWindowState,
    /// The Edgeware warnings dialog takes focus once, when it first appears, so Escape closes it.
    warnings_focus: FocusHandle,
    warnings_focused: bool,
    _changes: Subscription,
}

impl Shell {
    pub fn new(editor: Entity<Editor>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _changes = Self::subscribe(&editor, cx);
        cx.set_global(CurrentShell(cx.weak_entity()));
        Self {
            editor,
            view: View::Popups,
            views: HashMap::new(),
            import_window: ImportWindowState::default(),
            warnings_focus: cx.focus_handle(),
            warnings_focused: false,
            _changes,
        }
    }

    /// Points the shell at another editor: Save As opens the copy it wrote, and discarding
    /// changes reopens the pack.
    pub fn set_editor(&mut self, editor: Entity<Editor>, cx: &mut Context<Self>) {
        self._changes = Self::subscribe(&editor, cx);
        self.editor = editor;
        // Views made for the old editor would keep editing it.
        self.views.clear();
        self.import_window = ImportWindowState::default();
        cx.notify();
    }

    /// Switches to `view`.
    pub fn show(&mut self, view: View, cx: &mut Context<Self>) {
        // Media tabs own live previews. Drop them on navigation so hidden audio, decoder threads
        // and the last atlas image are released.
        if self.view != view && matches!(self.view, View::Popups | View::Audio | View::AllMedia) {
            self.views.remove(&self.view);
        }
        self.view = view;
        cx.notify();
    }

    fn subscribe(editor: &Entity<Editor>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(editor, |this, _, event: &EditorEvent, cx| match event {
            EditorEvent::Changed(changes) => {
                if changes.scopes.intersects(this.view.scopes()) {
                    cx.notify();
                }
            }
        })
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.read(cx);
        let history = editor.history().clone();
        let name = editor.pack().name();
        let dirty = editor.is_dirty();
        let saving = editor.saving;
        let error = editor.error.clone();
        let untitled = editor.pack().is_untitled();
        let busy = saving.is_some();

        let state: SharedString = match saving {
            Some(progress) if progress.total > 0 => {
                format!("Saving {} of {}", progress.saved, progress.total).into()
            }
            Some(_) => "Saving...".into(),
            None if untitled => "Draft, not saved yet".into(),
            None if dirty => "Unsaved changes".into(),
            None => "Saved".into(),
        };
        let action = |id: &'static str, label: &'static str, action: Box<dyn gpui_kit::Action>| {
            Button::new(id, label)
                .variant(Variant::Quiet)
                .size(Size::Compact)
                .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        };
        let dispatch = |action: Box<dyn gpui_kit::Action>| {
            move |window: &mut Window, cx: &mut gpui_kit::App| {
                window.dispatch_action(action.boxed_clone(), cx)
            }
        };
        let modifier = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        let dismiss = self.editor.clone();

        let import = Menu::new("import-menu", "Import")
            .variant(Variant::Secondary)
            .disabled(busy)
            .entry(MenuEntry::item(
                "Add files...",
                dispatch(Box::new(ImportFiles)),
            ))
            .entry(MenuEntry::item(
                "Add folder...",
                dispatch(Box::new(ImportFolder)),
            ));
        let mut more = Menu::new("pack-menu", "More").entry(
            MenuEntry::item("Save as...", dispatch(Box::new(SaveAs)))
                .hint(format!("{modifier}+Shift+S"))
                .disabled(busy),
        );
        if dirty && !untitled {
            more = more.entry(
                MenuEntry::item("Discard changes", dispatch(Box::new(DiscardChanges)))
                    .destructive()
                    .disabled(busy),
            );
        }
        let more = more.entry(MenuEntry::Separator).entry(
            MenuEntry::item("Close pack", dispatch(Box::new(ClosePack)))
                .hint(format!("{modifier}+W"))
                .disabled(busy),
        );

        div()
            .flex_none()
            .h(px(44.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::SURFACE))
            .child(
                div()
                    .min_w_0()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(name),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when(dirty || busy, |this| {
                        this.child(div().size(px(6.)).rounded_full().bg(c(theme::MUTED)))
                    })
                    .child(ui::readout(state)),
            )
            .children(error.map(|error| {
                div()
                    .id("edit-error")
                    .ml_2()
                    .min_w_0()
                    .truncate()
                    .cursor_pointer()
                    .text_color(c(theme::DANGER))
                    .text_size(px(12.))
                    .child(error)
                    .on_click(move |_, _, cx| {
                        dismiss.update(cx, |editor, cx| editor.dismiss_error(cx))
                    })
            }))
            .child(div().flex_1())
            .child(import)
            .child(action("undo", "Undo", Box::new(Undo)).disabled(!history.can_undo || busy))
            .child(action("redo", "Redo", Box::new(Redo)).disabled(!history.can_redo || busy))
            .child(
                Button::new("save", "Save")
                    .variant(Variant::Primary)
                    .size(Size::Compact)
                    .loading(busy)
                    .disabled(!dirty && !untitled)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(Save), cx)),
            )
            .child(more)
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = GROUPS.iter().map(|(heading, views)| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .text_size(px(11.))
                        .text_color(c(theme::MUTED))
                        .child(*heading),
                )
                .children(views.iter().map(|&view| {
                    let shell = cx.entity();
                    NavItem::new(
                        SharedString::from(format!("view-{view:?}")),
                        view.label(),
                        view == self.view,
                    )
                    .on_click(move |_, _, cx| shell.update(cx, |shell, cx| shell.show(view, cx)))
                }))
        });
        div()
            .flex_none()
            .w(px(200.))
            .h_full()
            .px_2()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::SURFACE))
            .children(groups)
    }

    fn import_window(&mut self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let editor = self.editor.read(cx);
        let progress = editor.import_progress.clone();
        let preparing = editor.importing() && !progress.running();
        if !editor.importing() {
            self.import_window.stopping = false;
        }
        if !ImportWindow::visible(&progress, preparing) {
            return None;
        }
        let entity = cx.entity();
        let (minimize, errors, stop, dismiss) =
            (entity.clone(), entity.clone(), entity.clone(), entity);
        Some(
            ImportWindow {
                progress,
                preparing,
                minimized: self.import_window.minimized,
                errors_open: self.import_window.errors_open,
                stopping: self.import_window.stopping,
                on_toggle_minimized: Box::new(move |_, cx| {
                    minimize.update(cx, |this, cx| {
                        this.import_window.minimized = !this.import_window.minimized;
                        cx.notify();
                    })
                }),
                on_toggle_errors: Box::new(move |_, cx| {
                    errors.update(cx, |this, cx| {
                        this.import_window.errors_open = !this.import_window.errors_open;
                        cx.notify();
                    })
                }),
                on_stop: Box::new(move |_, cx| {
                    stop.update(cx, |this, cx| {
                        this.import_window.stopping = true;
                        this.editor
                            .update(cx, |editor, cx| editor.cancel_import(cx));
                        cx.notify();
                    })
                }),
                on_dismiss: Box::new(move |_, cx| {
                    dismiss.update(cx, |this, cx| {
                        this.import_window.errors_open = false;
                        this.editor
                            .update(cx, |editor, cx| editor.dismiss_import_report(cx));
                        cx.notify();
                    })
                }),
            }
            .render(),
        )
    }

    fn warnings_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let warnings = self.editor.read(cx).import_warnings.clone();
        if warnings.is_empty() {
            self.warnings_focused = false;
            return None;
        }
        if !self.warnings_focused {
            self.warnings_focused = true;
            window.focus(&self.warnings_focus, cx);
        }
        let editor = self.editor.clone();
        Some(imports::warnings_dialog(
            &warnings,
            move |_, cx| editor.update(cx, |editor, cx| editor.dismiss_import_warnings(cx)),
            &self.warnings_focus,
            window,
        ))
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.clone();
        let content = self
            .views
            .entry(self.view)
            .or_insert_with(|| views::create(self.view, editor, window, cx))
            .clone();
        let import_window = self.import_window(cx);
        let warnings = self.warnings_dialog(window, cx);
        let can_drop = self.editor.read(cx).saving.is_none();
        let drop_target = self.editor.clone();
        // Debug builds can pin the overlay on to check how it looks (it can't be dragged onto in
        // the headless check).
        let show_overlay =
            cfg!(debug_assertions) && std::env::var_os("PACK_EDITOR_CHECK_DROP_OVERLAY").is_some();

        div()
            .id("shell")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(self.toolbar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.sidebar(cx))
                    .child(div().flex_1().min_w_0().h_full().child(content)),
            )
            .when(can_drop, |this| {
                this.on_drop(move |paths: &ExternalPaths, _, cx| {
                    let paths = paths.paths().to_vec();
                    drop_target.update(cx, |editor, cx| editor.import_paths(paths, cx));
                })
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .when(!show_overlay, |this| this.opacity(0.))
                        .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(1.))
                        .child(imports::drop_overlay()),
                )
            })
            .children(import_window)
            .children(warnings)
    }
}
