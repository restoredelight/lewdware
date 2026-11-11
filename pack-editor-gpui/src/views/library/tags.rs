//! The Tags and Artists views: a namespace over the whole pack, as a searchable table of names
//! with how much each one is on, and the three edits a name supports -- rename, merge into another
//! name, delete. Ported from `pack-editor/src/lib/VocabularyPage.svelte`, `Tags.svelte`,
//! `Artists.svelte` and `MediaScopeMenu.svelte`.
//!
//! Also the reference for how a view edits through `Session` (see the README):
//!
//! - **Read** in `render`: the rows come from `Session::tag_rows` / `artist_rows`, re-read only
//!   when a change in their scope arrives, and the per-tab counts from the media index.
//! - **Redraw** on `EditorEvent::Changed` when its scopes concern the view -- including changes
//!   the view didn't make (undo, redo, the inspector).
//! - **Write** through `Editor::apply`, one call per author action.
//! - **Text commits on Enter** (or the Rename button); Escape cancels.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::*;
use gpui_kit::Focusable as _;
use gpui_kit::{
    div, px, AnyElement, Context, Entity, FocusHandle, FontWeight, SharedString, Subscription,
    Window,
};
use pack_editor_core::changes::{Scope, Scopes};
use pack_editor_core::session::library::{
    label_counts, LabelKind, MediaFilter, MediaTab, TabCounts,
};

use crate::editor::{Editor, EditorEvent};
use crate::shell::View;
use crate::theme::{self, c};
use crate::ui::{self, Button, DialogAction, IconButton, Select, SelectOption, Size, Variant};

/// One name in the table.
#[derive(Clone)]
struct Row {
    name: String,
    /// How many visible files carry it, per media tab.
    media: TabCounts,
    /// Tags only: captions and content groups naming it, and timeline stages selecting by it.
    content: u64,
    timeline: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Rename,
    Merge,
}

struct Editing {
    name: String,
    mode: Mode,
    /// The new name (rename).
    input: Entity<InputState>,
    /// The name to merge into.
    target: Option<String>,
    _events: Subscription,
}

/// The row actions' column (four icon buttons), shared by the header so the columns line up.
const ACTIONS_W: f32 = 140.;

pub struct VocabularyView {
    kind: LabelKind,
    editor: Entity<Editor>,
    rows: Vec<Row>,
    load_error: Option<SharedString>,
    search: Entity<InputState>,
    editing: Option<Editing>,
    deleting: Option<Row>,
    /// The row whose "Media" menu is open.
    menu: Option<String>,
    error: Option<SharedString>,
    dialog_focus: FocusHandle,
    /// The page's own focus, so Escape reaches it while a merge (which has no field) is open.
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl VocabularyView {
    pub fn new(
        kind: LabelKind,
        editor: Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(match kind {
                LabelKind::Tags => "Search tags...",
                LabelKind::Artists => "Search artists...",
            })
        });
        let mut subscriptions = vec![
            cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
                let EditorEvent::Changed(changes) = event;
                if changes.scopes.intersects(this.scopes()) {
                    this.reload(cx);
                }
            }),
            cx.subscribe_in(
                &search,
                window,
                |_, input, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        clean_search_field(
                            input,
                            pack_editor_core::session::library::clean_text,
                            window,
                            cx,
                        );
                        cx.notify();
                    }
                },
            ),
        ];
        subscriptions.extend(ui::track_focus_of(&search, window, cx));
        let mut view = Self {
            kind,
            editor,
            rows: Vec::new(),
            load_error: None,
            search,
            editing: None,
            deleting: None,
            menu: None,
            error: None,
            dialog_focus: cx.focus_handle(),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        view.reload(cx);
        view
    }

    fn scopes(&self) -> Scopes {
        match self.kind {
            // Behaviour edits can name or drop a tag (a caption's, a stage's).
            LabelKind::Tags => {
                Scope::Tags | Scope::Media | Scope::Entries | Scope::ContentGroups | Scope::Timeline
            }
            LabelKind::Artists => Scope::Artists | Scope::Media,
        }
    }

    fn noun(&self) -> &'static str {
        match self.kind {
            LabelKind::Tags => "tag",
            LabelKind::Artists => "artist",
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let session = self.editor.read(cx).session();
        let counts = label_counts(session.media(), self.kind);
        let rows = match self.kind {
            LabelKind::Tags => session.tag_rows().map(|rows| {
                rows.into_iter()
                    .map(|row| Row {
                        media: counts.get(&row.name).copied().unwrap_or(TabCounts {
                            all: row.media_count as usize,
                            ..TabCounts::default()
                        }),
                        name: row.name,
                        content: row.content_uses,
                        timeline: row.experience_uses,
                    })
                    .collect()
            }),
            LabelKind::Artists => session.artist_rows().map(|rows| {
                rows.into_iter()
                    .map(|row| Row {
                        media: counts.get(&row.name).copied().unwrap_or_default(),
                        name: row.name,
                        content: 0,
                        timeline: 0,
                    })
                    .collect()
            }),
        };
        match rows {
            Ok(rows) => {
                self.rows = rows;
                self.load_error = None;
            }
            Err(error) => self.load_error = Some(format!("{error:#}").into()),
        }
        // An edit whose row is gone (renamed by undo, deleted elsewhere) has nothing to edit.
        if let Some(editing) = &self.editing {
            if !self.rows.iter().any(|row| row.name == editing.name) {
                self.editing = None;
            }
        }
        cx.notify();
    }

    fn begin(&mut self, name: String, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        // A rename starts from the current name; a merge starts with nothing chosen.
        let initial = if mode == Mode::Rename {
            name.clone()
        } else {
            String::new()
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let _events =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.apply(window, cx),
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            );
        if mode == Mode::Rename {
            input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
        self.menu = None;
        self.error = None;
        self.editing = Some(Editing {
            name,
            mode,
            input,
            target: None,
            _events,
        });
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        cx.notify();
    }

    fn apply(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = &self.editing else {
            return;
        };
        let source = editing.name.clone();
        let target = match editing.mode {
            Mode::Rename => editing.input.read(cx).value().trim().to_string(),
            Mode::Merge => editing.target.clone().unwrap_or_default(),
        };
        if target.is_empty() || target == source {
            return;
        }
        let noun = self.noun();
        // Checked against every name, not only the ones the search is showing.
        if editing.mode == Mode::Rename && self.rows.iter().any(|row| row.name == target) {
            self.error = Some(
                format!("A {noun} named \"{target}\" already exists. Merge the {noun}s instead.")
                    .into(),
            );
            cx.notify();
            return;
        }
        let (kind, mode) = (self.kind, editing.mode);
        let done = self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| match (kind, mode) {
                (LabelKind::Tags, Mode::Rename) => session.rename_tag(&source, &target),
                (LabelKind::Tags, Mode::Merge) => session.merge_tag(&source, &target),
                (LabelKind::Artists, Mode::Rename) => session.rename_artist(&source, &target),
                (LabelKind::Artists, Mode::Merge) => session.merge_artist(&source, &target),
            })
        });
        match done {
            Some(()) => self.editing = None,
            None => self.error = self.editor.read(cx).error.clone(),
        }
        cx.notify();
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.deleting.take() else {
            return;
        };
        let kind = self.kind;
        let done = self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| match kind {
                LabelKind::Tags => session.delete_tag(&row.name),
                LabelKind::Artists => session.delete_artist(&row.name),
            })
        });
        if done.is_none() {
            self.error = self.editor.read(cx).error.clone();
        }
        window.blur(cx);
        cx.notify();
    }

    fn show_media(
        &mut self,
        name: &str,
        tab: MediaTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let mut filter = MediaFilter::default();
        match self.kind {
            LabelKind::Tags => filter.tags.insert(name.to_string()),
            LabelKind::Artists => filter.artists.insert(name.to_string()),
        };
        super::show_filtered(self.editor.entity_id(), tab, filter, window, cx);
    }

    // ── Rendering ───────────────────────────────────────────────────────────────────────────

    fn columns(&self) -> Vec<(&'static str, f32)> {
        match self.kind {
            LabelKind::Tags => vec![("Media", 64.), ("Content", 72.), ("Timeline", 72.)],
            LabelKind::Artists => vec![("Media", 64.)],
        }
    }

    fn values(&self, row: &Row) -> Vec<String> {
        match self.kind {
            LabelKind::Tags => vec![
                row.media.all.to_string(),
                row.content.to_string(),
                row.timeline.to_string(),
            ],
            LabelKind::Artists => vec![row.media.all.to_string()],
        }
    }

    /// "Media": a way into the files carrying this name, per media tab, each with its count.
    fn media_menu(&self, row: &Row, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.menu.as_deref() == Some(row.name.as_str());
        let toggle = cx.entity();
        let name = row.name.clone();
        let trigger = IconButton::new(
            SharedString::from(format!("media-{}", row.name)),
            gpui_kit::assets::IconName::Search,
            "Media",
        )
        .disabled(row.media.all == 0)
        .on_click(move |_, _, cx| {
            let name = name.clone();
            toggle.update(cx, |this, cx| {
                this.menu = if this.menu.as_ref() == Some(&name) {
                    None
                } else {
                    Some(name)
                };
                cx.notify();
            })
        });
        let menu = open.then(|| {
            let close = cx.entity();
            let entries = [
                (MediaTab::AllMedia, "All media"),
                (MediaTab::Popups, "Popups"),
                (MediaTab::Audio, "Audio"),
            ];
            let items: Vec<_> = entries
                .into_iter()
                .enumerate()
                .map(|(index, (tab, label))| {
                    let count = row.media.get(tab);
                    let entity = cx.entity();
                    let name = row.name.clone();
                    div()
                        .id(("media-menu-item", index))
                        .px_2()
                        .py(px(6.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .rounded(theme::RADIUS_SM)
                        .text_size(px(12.))
                        .child(label)
                        .child(ui::readout(count.to_string()))
                        .when(count == 0, |this| this.opacity(0.45))
                        .when(count > 0, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.bg(c(theme::SURFACE)))
                                .on_click(move |_, window, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.show_media(&name, tab, window, cx)
                                    })
                                })
                        })
                })
                .collect();
            let menu = crate::ui::floating(
                theme::CONTROL_COMPACT + px(4.),
                true,
                div()
                    .id("media-menu")
                    .occlude()
                    .w(px(180.))
                    .p_1()
                    .flex()
                    .flex_col()
                    .rounded(theme::RADIUS_MD)
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .bg(c(theme::SURFACE_RAISED))
                    .shadow(vec![gpui_kit::BoxShadow {
                        color: theme::shadow_pop(),
                        offset: gpui_kit::point(px(4.), px(4.)),
                        blur_radius: px(0.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .children(items),
            );
            let backdrop = ui::dismiss_layer("media-menu-backdrop", window, move |_, cx| {
                close.update(cx, |this, cx| {
                    this.menu = None;
                    cx.notify();
                })
            });
            [backdrop.into_any_element(), menu.into_any_element()]
        });
        div()
            .relative()
            .child(trigger)
            .children(menu.into_iter().flatten())
    }

    fn row(&self, index: usize, row: &Row, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let id = |what: &str| SharedString::from(format!("{what}-{}", row.name));
        let (rename, merge, delete) = (cx.entity(), cx.entity(), cx.entity());
        let (rename_name, merge_name, delete_row) =
            (row.name.clone(), row.name.clone(), row.clone());
        let main = div()
            .px_3()
            .min_h(px(44.))
            .flex()
            .items_center()
            .gap_2()
            .when(index > 0, |this| {
                this.border_t_1().border_color(c(theme::BORDER))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(row.name.clone()),
            )
            .children(
                self.columns()
                    .into_iter()
                    .zip(self.values(row))
                    .map(|((_, width), value)| ui::readout(value).w(px(width))),
            )
            .child(
                div()
                    .w(px(ACTIONS_W))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(2.))
                    .child(self.media_menu(row, window, cx))
                    .child(
                        IconButton::new(id("rename"), gpui_kit::assets::IconName::Pencil, "Rename")
                            .on_click(move |_, window, cx| {
                                let name = rename_name.clone();
                                rename.update(cx, |this, cx| {
                                    this.begin(name, Mode::Rename, window, cx)
                                })
                            }),
                    )
                    .child(
                        IconButton::new(id("merge"), gpui_kit::assets::IconName::Merge, "Merge")
                            .disabled(self.rows.len() < 2)
                            .on_click(move |_, window, cx| {
                                let name = merge_name.clone();
                                merge.update(cx, |this, cx| {
                                    this.begin(name, Mode::Merge, window, cx)
                                })
                            }),
                    )
                    .child(
                        IconButton::new(id("delete"), gpui_kit::assets::IconName::Trash, "Delete")
                            .on_click(move |_, window, cx| {
                                let row = delete_row.clone();
                                delete.update(cx, |this, cx| {
                                    this.menu = None;
                                    this.deleting = Some(row);
                                    window.focus(&this.dialog_focus, cx);
                                    cx.notify();
                                })
                            }),
                    ),
            );
        let editing = self
            .editing
            .as_ref()
            .filter(|editing| editing.name == row.name);
        div()
            .flex()
            .flex_col()
            .child(main)
            .children(editing.map(|editing| self.edit_row(editing, cx)))
            .into_any_element()
    }

    fn edit_row(&self, editing: &Editing, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, note) = match (editing.mode, self.kind) {
            (Mode::Rename, LabelKind::Tags) => (
                format!("Rename \"{}\"", editing.name),
                "Every media and behaviour reference will be updated.",
            ),
            (Mode::Rename, LabelKind::Artists) => (
                format!("Rename \"{}\"", editing.name),
                "Every media item will be updated.",
            ),
            (Mode::Merge, _) => (
                format!("Merge \"{}\" into", editing.name),
                "References will be combined and duplicates removed.",
            ),
        };
        let value = match editing.mode {
            Mode::Rename => editing.input.read(cx).value().trim().to_string(),
            Mode::Merge => editing.target.clone().unwrap_or_default(),
        };
        let ready = !value.is_empty() && value != editing.name;
        let cancel_key = cx.entity();
        let field = match editing.mode {
            Mode::Rename => div()
                .h(theme::CONTROL_COMPACT)
                .flex()
                .items_center()
                .rounded(theme::RADIUS_SM)
                .border_1()
                .border_color(c(theme::FOCUS))
                .bg(c(theme::SURFACE))
                .on_key_down(move |event, _, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        cancel_key.update(cx, |this, cx| this.cancel(cx));
                    }
                })
                .child(
                    Input::new(&editing.input)
                        .appearance(false)
                        .text_size(px(12.5))
                        .flex_1(),
                )
                .into_any_element(),
            Mode::Merge => {
                let entity = cx.entity();
                let options = self
                    .rows
                    .iter()
                    .filter(|row| row.name != editing.name)
                    .map(|row| SelectOption::new(Some(row.name.clone()), row.name.clone()))
                    .collect();
                Select::new("merge-target", editing.target.clone(), options)
                    .on_change(move |target, _, cx| {
                        entity.update(cx, |this, cx| {
                            if let Some(editing) = &mut this.editing {
                                editing.target = target;
                            }
                            cx.notify();
                        })
                    })
                    .into_any_element()
            }
        };
        let (cancel, apply) = (cx.entity(), cx.entity());
        div()
            .px_3()
            .py_3()
            .flex()
            .items_center()
            .gap_4()
            .border_t_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::BG))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .child(ui::hint(note).text_size(px(11.))),
            )
            .child(div().w(px(240.)).child(field))
            .child(
                Button::new("edit-cancel", "Cancel")
                    .size(Size::Compact)
                    .on_click(move |_, _, cx| cancel.update(cx, |this, cx| this.cancel(cx))),
            )
            .child(
                Button::new(
                    "edit-apply",
                    if editing.mode == Mode::Rename {
                        "Rename"
                    } else {
                        "Merge"
                    },
                )
                .variant(Variant::Primary)
                .size(Size::Compact)
                .disabled(!ready)
                .on_click(move |_, window, cx| apply.update(cx, |this, cx| this.apply(window, cx))),
            )
    }

    fn delete_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let row = self.deleting.as_ref()?;
        let noun = self.noun();
        let count =
            |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let files = count(row.media.all as u64, "file", "files");
        let references = row.content + row.timeline;
        let description = match self.kind {
            LabelKind::Tags if references > 0 => format!(
                "This removes the tag from {files} and from {} in Content and the Timeline. \
                 No media files will be deleted.",
                count(references, "setting", "settings"),
            ),
            LabelKind::Tags => {
                format!("This removes the tag from {files}. No media files will be deleted.")
            }
            LabelKind::Artists => {
                format!("This removes the artist from {files}. No media files will be deleted.")
            }
        };
        let (cancel, confirm, close) = (cx.entity(), cx.entity(), cx.entity());
        Some(ui::dialog(
            format!("Delete \"{}\"?", row.name),
            description,
            vec![
                DialogAction::new("Cancel", Variant::Secondary, move |_, _, cx| {
                    cancel.update(cx, |this, cx| {
                        this.deleting = None;
                        cx.notify();
                    })
                }),
                DialogAction::new(
                    format!("Delete {noun}"),
                    Variant::Destructive,
                    move |_, window, cx| {
                        confirm.update(cx, |this, cx| this.confirm_delete(window, cx))
                    },
                ),
            ],
            move |_, cx| {
                close.update(cx, |this, cx| {
                    this.deleting = None;
                    cx.notify();
                })
            },
            &self.dialog_focus,
            window,
        ))
    }
}

impl Render for VocabularyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let (title, empty) = match self.kind {
            LabelKind::Tags => (
                "Tags",
                "Tags are created when you tag media or use them in Content and Timeline settings.",
            ),
            LabelKind::Artists => (
                "Artists",
                "Artists are created when you credit media in the inspector.",
            ),
        };
        let query = self.search.read(cx).value().trim().to_lowercase();
        let visible: Vec<Row> = self
            .rows
            .iter()
            .filter(|row| query.is_empty() || row.name.to_lowercase().contains(&query))
            .cloned()
            .collect();

        let table = if let Some(error) = &self.load_error {
            ui::hint(format!("Couldn't load {}: {error}", title.to_lowercase())).into_any_element()
        } else if visible.is_empty() {
            let searching = !query.is_empty();
            let entity = cx.entity();
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_3()
                .child(ui::label(if searching {
                    format!("No matching {}", title.to_lowercase())
                } else {
                    format!("No {} yet", title.to_lowercase())
                }))
                .child(ui::hint(if searching {
                    format!(
                        "No {} match this search. Clear it to see every {} in the pack.",
                        title.to_lowercase(),
                        self.noun()
                    )
                } else {
                    empty.to_string()
                }))
                .child(if searching {
                    Button::new("clear-search", "Clear search")
                        .size(Size::Compact)
                        .on_click(move |_, window, cx| {
                            entity.update(cx, |this, cx| {
                                this.search
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                cx.notify();
                            })
                        })
                } else {
                    Button::new("go-all-media", "Go to All media")
                        .size(Size::Compact)
                        .on_click(|_, window, cx| {
                            crate::shell::show_view(View::AllMedia, window, cx)
                        })
                })
                .into_any_element()
        } else {
            let head = div()
                .px_3()
                .h(px(34.))
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(c(theme::BORDER))
                .bg(c(theme::BG))
                .rounded_t(theme::RADIUS_MD)
                .child(
                    ui::readout(match self.kind {
                        LabelKind::Tags => "Tag",
                        LabelKind::Artists => "Artist",
                    })
                    .flex_1(),
                )
                .children(
                    self.columns()
                        .into_iter()
                        .map(|(label, width)| ui::readout(label).w(px(width))),
                )
                .child(div().w(px(ACTIONS_W)));
            let rows: Vec<AnyElement> = visible
                .iter()
                .enumerate()
                .map(|(index, row)| self.row(index, row, window, cx))
                .collect();
            ui::card().child(head).children(rows).into_any_element()
        };

        let error = self.error.clone().map(|error| {
            let entity = cx.entity();
            div()
                .id("vocabulary-error")
                .px_3()
                .py_2()
                .rounded(theme::RADIUS_MD)
                .border_1()
                .border_color(c(theme::DANGER_BORDER))
                .bg(c(theme::DANGER_BG))
                .text_size(px(12.))
                .text_color(c(theme::DANGER))
                .cursor_pointer()
                .child(error)
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.error = None;
                        cx.notify();
                    })
                })
        });
        let dialog = self.delete_dialog(window, cx);

        let escape = cx.entity();
        div()
            .id("vocabulary-page")
            .track_focus(&self.focus)
            .on_key_down(move |event, _, cx| {
                if event.keystroke.key == "escape" {
                    escape.update(cx, |this, cx| {
                        if this.editing.is_some() || this.menu.is_some() {
                            cx.stop_propagation();
                            this.menu = None;
                            this.cancel(cx);
                        }
                    })
                }
            })
            .size_full()
            .overflow_y_scrollbar()
            .child(
                div()
                    .mx_auto()
                    .w_full()
                    .max_w(px(960.))
                    .p_8()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .child(
                        div().flex().items_end().justify_end().gap_6().child(
                            ui::search_field(&self.search, search_focused)
                                .flex_none()
                                .w(px(220.)),
                        ),
                    )
                    .children(error)
                    .child(table),
            )
            .children(dialog)
    }
}

/// Rewrites a search field's text with `clean` when that changes it, so a paste that brought
/// invisible characters (or, searching file names, a whole path) shows what's searched for.
/// See `pack_editor_core::session::library::{clean_text, search_query}`.
pub(crate) fn clean_search_field(
    input: &Entity<InputState>,
    clean: fn(&str) -> String,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let typed = input.read(cx).value().to_string();
    let cleaned = clean(&typed);
    if cleaned != typed {
        input.update(cx, |input, cx| input.replace_all(cleaned, window, cx));
    }
}
