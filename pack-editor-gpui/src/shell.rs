//! The open pack's window: toolbar (editable name, save state, undo/redo, save, the pack menu),
//! the sidebar's views, the active view, and what's drawn over them -- the import window, the
//! "drop to import" overlay and the Edgeware import warnings. Ported from
//! `pack-editor/src/lib/Editor.svelte` and `EditorToolbar.svelte`.
//!
//! The views themselves live in `views`, one module per track of the port. The shell only
//! switches between them.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyView, App, Context, Entity, ExternalPaths, FocusHandle, Focusable, FontWeight,
    Global, KeyDownEvent, SharedString, Subscription, WeakEntity, Window,
};
use pack_editor_core::changes::{Scope, Scopes};
use pack_editor_core::pack::ops::lifecycle::MetadataField;

use crate::app::{ClosePack, DiscardChanges, ImportFiles, ImportFolder, Redo, Save, SaveAs, Undo};
use crate::editor::{CommitPending, Editor, EditorEvent};
use crate::imports::{self, ImportWindow};
use crate::menu::{Menu, MenuEntry};
use crate::theme::{self, c};
use crate::ui::{self, Button, IconButton, NavItem, Size, Variant};
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

impl View {
    fn icon(self) -> IconName {
        match self {
            View::Popups => IconName::Image,
            View::Audio => IconName::Music,
            View::AllMedia => IconName::LayoutGrid,
            View::Tags => IconName::Tag,
            View::Artists => IconName::Brush,
            View::Content => IconName::FileText,
            View::Timeline => IconName::Clock,
            View::Modes => IconName::CodeXml,
            View::Metadata => IconName::Settings,
        }
    }
}

/// Grouped by who reads them: the built-in modes (Sandbox and Sequence) use the popup and audio
/// media, the content and the timeline; custom modes draw from all the media and are bundled in
/// Modes; tags, artists and metadata belong to the pack whichever mode runs it.
const GROUPS: &[(&str, &[View])] = &[
    (
        "Built-in modes",
        &[View::Popups, View::Audio, View::Content, View::Timeline],
    ),
    ("Custom modes", &[View::AllMedia, View::Modes]),
    ("Pack", &[View::Tags, View::Artists, View::Metadata]),
];

/// The "Import" menu, in the media tabs' toolbars.
pub fn import_menu(disabled: bool) -> Menu {
    let dispatch = |action: Box<dyn gpui_kit::Action>| {
        move |window: &mut Window, cx: &mut App| window.dispatch_action(action.boxed_clone(), cx)
    };
    Menu::new("import-menu", "Import")
        .variant(Variant::Secondary)
        .icon(IconName::Upload)
        .disabled(disabled)
        .entry(MenuEntry::item(
            "Add files...",
            dispatch(Box::new(ImportFiles)),
        ))
        .entry(MenuEntry::item(
            "Add folder...",
            dispatch(Box::new(ImportFolder)),
        ))
}

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
    /// Dragged to here (its top left corner, in the shell); `None` is the bottom right.
    position: Option<gpui_kit::Point<gpui_kit::Pixels>>,
    drag: imports::MoveImportWindow,
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
    /// The pack's name, editable in place at the toolbar's left. It follows the pack while it
    /// isn't focused (see `render`), and commits on Enter, blur, or `CommitPending`.
    title: Entity<InputState>,
    _title_events: Subscription,
    _title_focus: [Subscription; 2],
    _changes: [Subscription; 2],
}

impl Shell {
    pub fn new(editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _changes = Self::subscribe(&editor, cx);
        cx.set_global(CurrentShell(cx.weak_entity()));
        let name = editor.read(cx).pack().name();
        let title = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let _title_focus = ui::track_focus_of(&title, window, cx);
        let _title_events = cx.subscribe(&title, |this, _, event: &InputEvent, cx| {
            match event {
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_title(cx),
                // The box grows with what's typed.
                InputEvent::Change => cx.notify(),
                _ => {}
            }
        });
        Self {
            editor,
            view: View::Popups,
            views: HashMap::new(),
            import_window: ImportWindowState::default(),
            warnings_focus: cx.focus_handle(),
            warnings_focused: false,
            title,
            _title_events,
            _title_focus,
            _changes,
        }
    }

    /// Writes the typed title as the pack's name. A blank or unchanged title writes nothing, and
    /// the field takes the stored name back on the next render once it loses focus.
    fn commit_title(&mut self, cx: &mut Context<Self>) {
        let text = self.title.read(cx).value().trim().to_string();
        if text.is_empty() || text == self.editor.read(cx).pack().name() {
            return;
        }
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| {
                session.set_metadata(MetadataField::Name(text))
            });
        });
    }

    fn title_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => {
                let name = self.editor.read(cx).pack().name();
                self.title
                    .update(cx, |input, cx| input.set_value(name, window, cx));
                window.blur(cx);
            }
            "enter" => window.blur(cx),
            _ => {}
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

    /// Switches to `view`, once the current one has committed the field being typed in.
    pub fn show(&mut self, view: View, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |editor, cx| editor.commit_pending(cx));
        let shell = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| shell.switch_to(view, cx));
        });
    }

    fn switch_to(&mut self, view: View, cx: &mut Context<Self>) {
        // Media tabs own live previews. Drop them on navigation so hidden audio, decoder threads
        // and the last atlas image are released.
        if self.view != view && matches!(self.view, View::Popups | View::Audio | View::AllMedia) {
            self.views.remove(&self.view);
        }
        self.view = view;
        cx.notify();
    }

    fn subscribe(editor: &Entity<Editor>, cx: &mut Context<Self>) -> [Subscription; 2] {
        [
            cx.subscribe(editor, |this, _, event: &EditorEvent, cx| match event {
                EditorEvent::Changed(changes) => {
                    // The toolbar shows the name.
                    if changes
                        .scopes
                        .intersects(this.view.scopes() | Scope::Metadata)
                    {
                        cx.notify();
                    }
                }
            }),
            cx.subscribe(editor, |this, _, _: &CommitPending, cx| {
                this.commit_title(cx)
            }),
        ]
    }

    fn toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.read(cx);
        let history = editor.history().clone();
        let dirty = editor.is_dirty();
        let saving = editor.saving;
        let error = editor.error.clone();
        let untitled = editor.pack().is_untitled();
        let busy = saving.is_some();

        // A routine state is a bare muted dot, with the detail in its tooltip; only saving
        // progress is spelled out (`shared-ui/DESIGN.md`, "Routine states are not warnings").
        let progress: Option<SharedString> = saving.map(|progress| {
            if progress.waiting_for_import {
                "Saving once the import finishes".into()
            } else if progress.total > 0 {
                format!("Saving {} of {}", progress.saved, progress.total).into()
            } else {
                "Saving...".into()
            }
        });
        let state: Option<SharedString> = if untitled {
            Some("Draft, not saved yet: choose where to save it on first save".into())
        } else if dirty {
            Some("Unsaved changes".into())
        } else {
            None
        };
        let modifier = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        let dispatch = |action: Box<dyn gpui_kit::Action>| {
            move |window: &mut Window, cx: &mut gpui_kit::App| {
                window.dispatch_action(action.boxed_clone(), cx)
            }
        };
        let icon_action = |id: &'static str,
                           icon: IconName,
                           tooltip: String,
                           action: Box<dyn gpui_kit::Action>| {
            IconButton::new(id, icon, tooltip)
                .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        };
        let dismiss = self.editor.clone();

        let mut more = Menu::new("pack-menu", "More pack actions")
            .icon_only(IconName::EllipsisVertical)
            .entry(
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

        let title_focused = self.title.read(cx).focus_handle(cx).is_focused(window);
        // The box hugs the name, so the save-state dot sits right after it.
        let typed: SharedString = self.title.read(cx).value().to_string().into();
        let font_size = px(14.);
        let text_width = window
            .text_system()
            .shape_line(
                typed.clone(),
                font_size,
                &[gpui_kit::TextRun {
                    len: typed.len(),
                    font: gpui_kit::Font {
                        weight: FontWeight::SEMIBOLD,
                        ..gpui_kit::font(theme::FONT_SANS)
                    },
                    color: c(theme::TEXT),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            )
            .width;
        // The input's 10px padding each side, the border, and room for the caret (more while
        // typing, so the text never scrolls under the padding).
        let slack = if title_focused { px(40.) } else { px(26.) };
        let title_width = (text_width + slack).clamp(px(80.), px(420.));
        let entity = cx.entity();
        let title = div()
            .id("pack-title")
            .w(title_width)
            .h(theme::CONTROL_COMPACT)
            .flex()
            .items_center()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .text_size(px(14.))
            .font_weight(FontWeight::SEMIBOLD)
            .map(|this| {
                if title_focused {
                    this.border_color(c(theme::BORDER_STRONG)).bg(c(theme::BG))
                } else {
                    this.border_color(gpui_kit::transparent_black())
                        .hover(|style| style.border_color(c(theme::BORDER)))
                }
            })
            .tooltip(|window, cx| Tooltip::new("Edit pack title").build(window, cx))
            .on_key_down(move |event, window, cx| {
                entity.update(cx, |this, cx| this.title_key_down(event, window, cx))
            })
            .child(
                Input::new(&self.title)
                    .appearance(false)
                    .text_size(px(14.))
                    .flex_1(),
            );

        div()
            .flex_none()
            .h(px(44.))
            .pl_1()
            .pr_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::SURFACE))
            .child(title)
            .when(busy || state.is_some(), |this| {
                let tooltip = progress.clone().or(state.clone()).unwrap_or_default();
                this.child(
                    div()
                        .id("save-state")
                        .flex_none()
                        .p_1()
                        .child(div().size(px(6.)).rounded_full().bg(c(theme::MUTED)))
                        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)),
                )
            })
            .children(progress.map(ui::readout))
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        icon_action(
                            "undo",
                            IconName::Undo2,
                            format!("Undo ({modifier}+Z)"),
                            Box::new(Undo),
                        )
                        .disabled(!history.can_undo || busy),
                    )
                    .child(
                        icon_action(
                            "redo",
                            IconName::Redo2,
                            format!("Redo ({modifier}+Shift+Z)"),
                            Box::new(Redo),
                        )
                        .disabled(!history.can_redo || busy),
                    ),
            )
            .child({
                // The button says where things stand, not just the dot beside the title:
                // red "Save" with work to save, "Saving" during, a quiet "Saved" after.
                let saved = !busy && !dirty && !untitled;
                let label = if busy {
                    "Saving"
                } else if saved {
                    "Saved"
                } else {
                    "Save"
                };
                let button = Button::new("save", label)
                    .variant(if saved {
                        Variant::Secondary
                    } else {
                        Variant::Primary
                    })
                    .size(Size::Compact)
                    .full_width()
                    .loading(busy)
                    .disabled(saved)
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(Save), cx));
                // Wide enough for the longest label, so the toolbar doesn't shift between them.
                div().flex_none().w(px(72.)).child(button)
            })
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
                        .px_2()
                        .pt_4()
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
                    .icon(view.icon())
                    .on_click(move |_, _, cx| shell.update(cx, |shell, cx| shell.show(view, cx)))
                }))
        });
        ui::resizable_sidebar(
            "nav-sidebar",
            ui::Side::Left,
            px(200.),
            (px(160.), px(360.)),
            div()
                .size_full()
                .px_2()
                .flex()
                .flex_col()
                .bg(c(theme::SURFACE))
                .children(groups),
            cx,
        )
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
                position: self.import_window.position,
                drag: self.import_window.drag.clone(),
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
        // The title follows the pack (undo, redo, the Metadata view, another editor after Save
        // As) whenever the author isn't typing in it.
        let name = self.editor.read(cx).pack().name();
        if !self.title.read(cx).focus_handle(cx).is_focused(window)
            && self.title.read(cx).value().as_ref() != name
        {
            self.title
                .update(cx, |input, cx| input.set_value(name, window, cx));
        }
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
            .child(self.toolbar(window, cx))
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
            .on_drag_move::<imports::MoveImportWindow>(cx.listener(
                |this, event: &gpui_kit::DragMoveEvent<imports::MoveImportWindow>, _, cx| {
                    let grab = event.drag(cx).0.get();
                    let bounds = event.bounds;
                    // The titlebar sits inside the window's 1px border.
                    let corner = event.event.position
                        - grab
                        - bounds.origin
                        - gpui_kit::point(px(1.), px(1.));
                    // Keep the titlebar reachable.
                    let x = corner
                        .x
                        .clamp(px(0.), (bounds.size.width - px(120.)).max(px(0.)));
                    let y = corner
                        .y
                        .clamp(px(0.), (bounds.size.height - px(32.)).max(px(0.)));
                    this.import_window.position = Some(gpui_kit::point(x, y));
                    cx.notify();
                },
            ))
            .children(import_window)
            .children(warnings)
    }
}
