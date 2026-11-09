//! One media tab (Popups, Audio or All media): the toolbar, the files as a grid of thumbnails (an
//! audio list in Audio), the inspector, and the removal dialog and viewer. Ported from
//! `pack-editor/src/lib/MediaGrid.svelte`, `AudioList.svelte`, `MediaToolbar.svelte` and
//! `mediaSelection.svelte.ts`.
//!
//! Both layouts are a `uniform_list` of fixed-height rows, so only what's on screen is built,
//! whatever the size of the pack. The grid's rows are `columns` tiles wide, measured from the
//! space it had last frame. Thumbnails come from `thumbs`.
//!
//! Selection and filters are this tab's own (`TabState`), restored when the shell makes the view
//! again. The list itself is recomputed only when the media or the filters change, not per frame.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    canvas, div, img, px, AnyView, App, ClickEvent, Context, Entity, FocusHandle, FontWeight,
    ImageSource, KeyDownEvent, ObjectFit, Pixels, ScrollStrategy, SharedString, Subscription,
    UniformListScrollHandle, Window,
};
use pack_editor_core::changes::Scope;
use pack_editor_core::pack::MediaFile;
use pack_editor_core::session::library::{
    grid_step, is_popup_audio, label_names, KindFilter, LabelKind, MediaFilter, MediaTab,
    RoleFilter, Selection, SortKey, Step,
};
use shared::encode::FileInfo;

use super::inspector::{Inspector, InspectorEvent};
use super::thumbs::Thumbnails;
use super::viewer::{Viewer, ViewerEvent};
use super::{format_duration, format_size, TabState};
use crate::editor::{Editor, EditorEvent};
use crate::theme::{self, c};
use crate::ui::{self, Button, DialogAction, Select, SelectOption, Size, Variant};

const TILE_W: f32 = 148.;
const THUMB_H: f32 = 132.;
const ROW_H: f32 = 196.;
const GRID_PAD: f32 = 12.;
const GRID_GAP: f32 = 12.;
const AUDIO_ROW_H: f32 = 52.;

pub(crate) fn create(
    tab: MediaTab,
    editor: Entity<Editor>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    cx.new(|cx| MediaBrowser::new(tab, editor, window, cx))
        .into()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
    Tags,
    Artists,
}

pub(crate) struct MediaBrowser {
    tab: MediaTab,
    editor: Entity<Editor>,
    filter: MediaFilter,
    selection: Selection,
    /// The tab's files after its filters, in order. Shared with the list's render closure.
    list: Rc<Vec<u64>>,
    /// How many files the tab has before its filters.
    total: usize,
    /// Tiles per row, as measured from last frame's width.
    columns: Rc<Cell<usize>>,
    measured: Rc<Cell<bool>>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    search: Entity<InputState>,
    thumbs: Rc<RefCell<Thumbnails>>,
    inspector: Entity<Inspector>,
    viewer: Option<Entity<Viewer>>,
    removing: Option<Removal>,
    dialog_focus: FocusHandle,
    menu: Option<Menu>,
    reveal: Option<u64>,
    _subscriptions: Vec<Subscription>,
}

struct Removal {
    ids: Vec<u64>,
    /// The slots removing these files would also empty ("Wallpaper", ...).
    clears: Vec<String>,
}

impl MediaBrowser {
    fn new(
        tab: MediaTab,
        editor: Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = super::load_tab(editor.entity_id(), tab, cx);
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(if tab == MediaTab::Audio {
                    "Search audio..."
                } else {
                    "Search media..."
                })
                .default_value(state.filter.query.clone())
        });
        let inspector = cx.new(|cx| Inspector::new(tab, editor.clone(), window, cx));

        let mut subscriptions = vec![
            cx.subscribe_in(&editor, window, |this, _, event: &EditorEvent, window, cx| {
                let EditorEvent::Changed(changes) = event;
                if changes
                    .scopes
                    .intersects(Scope::Media | Scope::Tags | Scope::Artists)
                {
                    this.refresh(window, cx);
                }
            }),
            cx.subscribe_in(&search, window, |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter.query = input.read(cx).value().to_string();
                    this.refresh(window, cx);
                }
            }),
            cx.subscribe_in(
                &inspector,
                window,
                |this, _, event: &InspectorEvent, window, cx| match event {
                    InspectorEvent::Remove(ids) => this.request_removal(ids.clone(), window, cx),
                    InspectorEvent::View(id) => this.open_viewer(*id, window, cx),
                },
            ),
        ];
        // The shell drops this view when another is shown: keep what the author set up here, and
        // take the thumbnails out of the atlas.
        subscriptions.push(cx.on_release_in(window, |this, window, cx| {
            this.thumbs.borrow_mut().clear(window);
            let state = TabState {
                filter: this.filter.clone(),
                selection: this.selection.clone(),
                reveal: None,
            };
            super::store_tab(this.editor.entity_id(), this.tab, state, cx);
        }));

        let mut browser = Self {
            tab,
            editor,
            filter: state.filter,
            selection: state.selection,
            list: Rc::new(Vec::new()),
            total: 0,
            columns: Rc::new(Cell::new(1)),
            measured: Rc::new(Cell::new(false)),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            search,
            thumbs: Rc::default(),
            inspector,
            viewer: None,
            removing: None,
            dialog_focus: cx.focus_handle(),
            menu: None,
            reveal: state.reveal,
            _subscriptions: subscriptions,
        };
        browser.refresh(window, cx);
        window.focus(&browser.focus, cx);
        browser
    }

    // ── The list and the selection ──────────────────────────────────────────────────────────

    /// Recomputes the list from the index and the filters, and keeps the selection, inspector and
    /// viewer in step with it.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.editor.read(cx).session();
        let media = session.media();
        self.total = media.iter().filter(|file| self.tab.includes(file)).count();
        let list = self.filter.apply(self.tab, media.iter());
        self.selection.retain(&list);
        self.list = Rc::new(list);
        if let Some(viewer) = &self.viewer {
            viewer.update(cx, |viewer, cx| viewer.retain(window, cx));
        }
        self.sync_inspector(window, cx);
        cx.notify();
    }

    fn sync_inspector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.selection.ordered(&self.list);
        let primary = self.selection.primary;
        self.inspector.update(cx, |inspector, cx| {
            inspector.set_selection(ids, primary, window, cx)
        });
        cx.notify();
    }

    fn set_filter(&mut self, change: impl FnOnce(&mut MediaFilter), window: &mut Window, cx: &mut Context<Self>) {
        change(&mut self.filter);
        self.refresh(window, cx);
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.refresh(window, cx);
    }

    fn columns(&self) -> usize {
        match self.tab {
            MediaTab::Audio => 1,
            _ => self.columns.get().max(1),
        }
    }

    fn scroll_to(&self, id: u64) {
        if let Some(index) = self.list.iter().position(|&other| other == id) {
            self.scroll
                .scroll_to_item(index / self.columns(), ScrollStrategy::Nearest);
        }
    }

    fn click(&mut self, id: u64, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.menu = None;
        if event.click_count() >= 2 {
            self.selection.select_single(id);
            self.sync_inspector(window, cx);
            self.open_viewer(id, window, cx);
            return;
        }
        let modifiers = event.modifiers();
        self.selection
            .click(&self.list, id, modifiers.shift, modifiers.secondary(cx));
        self.sync_inspector(window, cx);
    }

    fn clear_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selection.clear();
        self.sync_inspector(window, cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Keys typed into the inspector's fields bubble through here too; they aren't ours.
        if !self.focus.is_focused(window) {
            return;
        }
        let keystroke = &event.keystroke;
        let secondary = keystroke.modifiers.secondary(cx);
        let step = match keystroke.key.as_str() {
            "left" if self.tab != MediaTab::Audio => Some(Step::Left),
            "right" if self.tab != MediaTab::Audio => Some(Step::Right),
            "up" => Some(Step::Up),
            "down" => Some(Step::Down),
            "home" => Some(Step::Home),
            "end" => Some(Step::End),
            _ => None,
        };
        if let Some(step) = step {
            cx.stop_propagation();
            let from = self
                .selection
                .active
                .and_then(|id| self.list.iter().position(|&other| other == id));
            if let Some(to) = grid_step(step, from, self.list.len(), self.columns()) {
                self.selection
                    .move_to(&self.list, to, keystroke.modifiers.shift, secondary);
                self.scroll
                    .scroll_to_item(to / self.columns(), ScrollStrategy::Nearest);
                self.sync_inspector(window, cx);
            }
            return;
        }
        match keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                self.menu = None;
                self.clear_selection(window, cx);
            }
            "space" => {
                if let Some(active) = self.selection.active {
                    cx.stop_propagation();
                    self.selection.toggle(active);
                    if self.selection.anchor.is_none() {
                        self.selection.anchor = Some(active);
                    }
                    self.sync_inspector(window, cx);
                }
            }
            "a" if secondary => {
                cx.stop_propagation();
                self.selection.select_all(&self.list);
                self.sync_inspector(window, cx);
            }
            "delete" | "backspace" if !self.selection.is_empty() => {
                cx.stop_propagation();
                let ids = self.selection.ordered(&self.list);
                self.request_removal(ids, window, cx);
            }
            "enter" => {
                if let Some(active) = self.selection.active {
                    cx.stop_propagation();
                    self.open_viewer(active, window, cx);
                }
            }
            _ => {}
        }
    }

    // ── Removing ────────────────────────────────────────────────────────────────────────────

    fn request_removal(&mut self, ids: Vec<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.editor.read(cx).session();
        let ids: Vec<u64> = ids
            .into_iter()
            .filter(|id| session.media().get(*id).is_some())
            .collect();
        if ids.is_empty() {
            return;
        }
        let mut clears = BTreeSet::new();
        for &id in &ids {
            for usage in session.media_usage(id).unwrap_or_default() {
                clears.insert(usage.label);
            }
        }
        self.removing = Some(Removal {
            ids,
            clears: clears.into_iter().collect(),
        });
        window.focus(&self.dialog_focus, cx);
        cx.notify();
    }

    fn cancel_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.removing = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn confirm_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(removal) = self.removing.take() else {
            return;
        };
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| session.remove_files(&removal.ids))
        });
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn removal_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let removal = self.removing.as_ref()?;
        let count = removal.ids.len();
        let name = (count == 1)
            .then(|| self.editor.read(cx).session().media().get(removal.ids[0]))
            .flatten()
            .map(|file| file.file_name.clone());
        let mut description = match name {
            Some(name) => format!("\"{name}\" will be removed from this pack."),
            None => format!("These {count} media items will be removed from this pack."),
        };
        if !removal.clears.is_empty() {
            description.push_str(&format!(" This also clears {}.", list_words(&removal.clears)));
        }
        description.push_str(if count == 1 {
            " The original file on your computer will not be deleted."
        } else {
            " The original files on your computer will not be deleted."
        });
        let (cancel, confirm, close) = (cx.entity(), cx.entity(), cx.entity());
        Some(ui::dialog(
            if count == 1 {
                "Remove media from pack?".to_string()
            } else {
                format!("Remove {count} items from pack?")
            },
            description,
            vec![
                DialogAction::new("Cancel", Variant::Secondary, move |_, window, cx| {
                    cancel.update(cx, |this, cx| this.cancel_removal(window, cx))
                }),
                DialogAction::new(
                    if count == 1 {
                        "Remove item".to_string()
                    } else {
                        format!("Remove {count} items")
                    },
                    Variant::Destructive,
                    move |_, window, cx| {
                        confirm.update(cx, |this, cx| this.confirm_removal(window, cx))
                    },
                ),
            ],
            move |window, cx| close.update(cx, |this, cx| this.cancel_removal(window, cx)),
            &self.dialog_focus,
            window,
        ))
    }

    // ── The viewer ──────────────────────────────────────────────────────────────────────────

    fn open_viewer(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        // Opened over a selection of several, it walks the selection; otherwise the whole list.
        let selected = self.selection.ordered(&self.list);
        let walking_selection = selected.len() > 1 && selected.contains(&id);
        let ids = if walking_selection {
            selected
        } else {
            self.list.to_vec()
        };
        if !ids.contains(&id) {
            return;
        }
        let editor = self.editor.clone();
        let viewer =
            cx.new(|cx| Viewer::new(editor, ids, id, walking_selection, window, cx));
        let subscription = cx.subscribe_in(
            &viewer,
            window,
            |this, _, event: &ViewerEvent, window, cx| match event {
                ViewerEvent::Close => this.close_viewer(window, cx),
                ViewerEvent::Moved(id) => {
                    this.selection.active = Some(*id);
                    if this.selection.len() <= 1 {
                        this.selection.select_single(*id);
                        this.sync_inspector(window, cx);
                    }
                    this.scroll_to(*id);
                    cx.notify();
                }
            },
        );
        self._subscriptions.push(subscription);
        viewer.update(cx, |viewer, cx| viewer.focus(window, cx));
        self.viewer = Some(viewer);
        cx.notify();
    }

    fn close_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Dropping the entity stops its preview and frees its image (`media-gpui`).
        self.viewer = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    // ── Rendering ───────────────────────────────────────────────────────────────────────────

    fn header(&self) -> impl IntoElement {
        let (title, lede) = match self.tab {
            MediaTab::Popups => (
                "Popups",
                "The images and videos the built-in modes spawn as popups.",
            ),
            MediaTab::Audio => (
                "Audio",
                "The built-in modes (Sandbox and Sequence) play background audio for the session \
                 and popup audio when a popup appears, one sound at a time. Custom modes can \
                 query the same files and tags, but choose their own playback.",
            ),
            MediaTab::AllMedia => (
                "All media",
                "Everything in the pack, including wallpapers, splashes and subliminals. Custom \
                 modes can draw from all of it.",
            ),
        };
        div()
            .flex_none()
            .px_4()
            .pt_4()
            .pb_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(ui::page_title(title))
            .child(ui::hint(lede))
    }

    fn filter_menu(&self, menu: Menu, cx: &mut Context<Self>) -> impl IntoElement {
        let (label, selected, kind) = match menu {
            Menu::Tags => ("Tags", &self.filter.tags, LabelKind::Tags),
            Menu::Artists => ("Artists", &self.filter.artists, LabelKind::Artists),
        };
        let open = self.menu == Some(menu);
        let count = selected.len();
        let names = if open {
            label_names(self.editor.read(cx).session().media(), kind)
        } else {
            Vec::new()
        };
        let toggle = cx.entity();
        let close = cx.entity();
        let trigger = div()
            .id(SharedString::from(format!("filter-{label}")))
            .h(theme::CONTROL_COMPACT)
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(if count > 0 {
                theme::ACCENT_HOVER
            } else {
                theme::BORDER_STRONG
            }))
            .bg(c(theme::SURFACE))
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
            .child(label)
            .when(count > 0, |this| this.child(ui::readout(count.to_string()).text_color(c(theme::TEXT))))
            .on_click(move |_, _, cx| {
                toggle.update(cx, |this, cx| {
                    this.menu = if this.menu == Some(menu) { None } else { Some(menu) };
                    cx.notify();
                })
            });
        let popover = open.then(|| {
            let entity = cx.entity();
            let selected = selected.clone();
            gpui_kit::deferred(
                div()
                    .id(SharedString::from(format!("filter-menu-{label}")))
                    .absolute()
                    .top(theme::CONTROL_COMPACT + px(4.))
                    .left_0()
                    .w(px(220.))
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .p_1()
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
                    .on_mouse_down_out(move |_, _, cx| {
                        close.update(cx, |this, cx| {
                            this.menu = None;
                            cx.notify();
                        })
                    })
                    .when(names.is_empty(), |this| {
                        this.child(
                            div().px_2().py(px(6.)).child(ui::hint(match menu {
                                Menu::Tags => "No tags yet.",
                                Menu::Artists => "No artists yet.",
                            })),
                        )
                    })
                    .children(names.into_iter().enumerate().map(move |(index, name)| {
                        let checked = selected.contains(&name);
                        let entity = entity.clone();
                        div()
                            .id(("filter-option", index))
                            .px_2()
                            .py(px(6.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(theme::RADIUS_SM)
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|style| style.bg(c(theme::SURFACE)))
                            .child(ui::checkbox(checked))
                            .child(div().min_w_0().truncate().child(name.clone()))
                            .on_click(move |_, window, cx| {
                                let name = name.clone();
                                entity.update(cx, |this, cx| {
                                    this.set_filter(
                                        |filter| {
                                            let set = match menu {
                                                Menu::Tags => &mut filter.tags,
                                                Menu::Artists => &mut filter.artists,
                                            };
                                            if !set.remove(&name) {
                                                set.insert(name);
                                            }
                                        },
                                        window,
                                        cx,
                                    )
                                })
                            })
                    })),
            )
            .with_priority(1)
        });
        div().relative().child(trigger).children(popover)
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let kind_or_role = if self.tab == MediaTab::Audio {
            let entity = entity.clone();
            Select::new(
                "audio-role-filter",
                self.filter.role,
                vec![
                    SelectOption::new(RoleFilter::All, "All roles"),
                    SelectOption::new(RoleFilter::Background, "Background"),
                    SelectOption::new(RoleFilter::Popup, "Popup"),
                ],
            )
            .on_change(move |role, window, cx| {
                entity.update(cx, |this, cx| this.set_filter(|f| f.role = role, window, cx))
            })
            .into_any_element()
        } else {
            let entity = entity.clone();
            let mut options = vec![
                SelectOption::new(KindFilter::All, "All types"),
                SelectOption::new(KindFilter::Image, "Images"),
                SelectOption::new(KindFilter::Video, "Videos"),
            ];
            if self.tab == MediaTab::AllMedia {
                options.push(SelectOption::new(KindFilter::Audio, "Audio"));
            }
            Select::new("kind-filter", self.filter.kind, options)
                .on_change(move |kind, window, cx| {
                    entity.update(cx, |this, cx| this.set_filter(|f| f.kind = kind, window, cx))
                })
                .into_any_element()
        };
        let sort = {
            let entity = entity.clone();
            Select::new(
                "sort",
                (self.filter.sort, self.filter.descending),
                vec![
                    SelectOption::new((SortKey::Created, true), "Newest first"),
                    SelectOption::new((SortKey::Created, false), "Oldest first"),
                    SelectOption::new((SortKey::Name, false), "Name: A to Z"),
                    SelectOption::new((SortKey::Name, true), "Name: Z to A"),
                    SelectOption::new((SortKey::Size, true), "Largest first"),
                    SelectOption::new((SortKey::Size, false), "Smallest first"),
                ],
            )
            .on_change(move |(sort, descending), window, cx| {
                entity.update(cx, |this, cx| {
                    this.set_filter(
                        |f| {
                            f.sort = sort;
                            f.descending = descending;
                        },
                        window,
                        cx,
                    )
                })
            })
        };
        let visible = self.list.len();
        let noun = |n| if n == 1 { "file" } else { "files" };
        let count = if self.filter.is_active() && visible != self.total {
            format!("{visible} of {} {}", self.total, noun(self.total))
        } else {
            format!("{} {}", self.total, noun(self.total))
        };
        let clear = entity.clone();
        div()
            .flex_none()
            .min_h(px(48.))
            .px_4()
            .py(px(6.))
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .w(px(220.))
                    .h(theme::CONTROL_COMPACT)
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(theme::RADIUS_MD)
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .bg(c(theme::BG))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(c(theme::MUTED))
                            .child(gpui_kit::assets::IconName::Search),
                    )
                    .child(
                        Input::new(&self.search)
                            .appearance(false)
                            .text_size(px(12.))
                            .flex_1(),
                    ),
            )
            .child(div().w(px(130.)).child(kind_or_role))
            .child(self.filter_menu(Menu::Tags, cx))
            .child(self.filter_menu(Menu::Artists, cx))
            .child(div().w(px(150.)).child(sort))
            .when(self.filter.is_active(), |this| {
                this.child(
                    Button::new("clear-filters", "Clear filters")
                        .variant(Variant::Quiet)
                        .size(Size::Compact)
                        .on_click(move |_, window, cx| {
                            clear.update(cx, |this, cx| this.clear_filters(window, cx))
                        }),
                )
            })
            .child(div().flex_1())
            .child(ui::readout(count))
    }

    fn empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered = self.total > 0;
        let entity = cx.entity();
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .p_6()
            .child(ui::label(if filtered {
                "No matching media"
            } else if self.tab == MediaTab::Audio {
                "No audio in this pack yet"
            } else {
                "Nothing here yet"
            }))
            .child(ui::hint(if filtered {
                "Nothing matches the current search, type, tag or artist filters."
            } else {
                "Importing media isn't ported to this editor yet."
            }))
            .when(filtered, |this| {
                this.child(
                    Button::new("empty-clear-filters", "Clear filters")
                        .size(Size::Compact)
                        .on_click(move |_, window, cx| {
                            entity.update(cx, |this, cx| this.clear_filters(window, cx))
                        }),
                )
            })
    }

    /// The thumbnail grid: rows of `columns` tiles, only the visible rows built.
    fn grid(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let columns = self.columns();
        let list = self.list.clone();
        let rows = list.len().div_ceil(columns);
        let editor = self.editor.clone();
        let thumbs = self.thumbs.clone();
        let browser = cx.weak_entity();
        let selection = Rc::new(self.selection.clone());
        let focused = self.focus.is_focused(window);
        thumbs.borrow_mut().begin_frame();

        let items = uniform_list_rows(rows, move |range, _, cx| {
            let editor = editor.read(cx);
            let media = editor.session().media();
            let mut thumbs = thumbs.borrow_mut();
            range
                .map(|row| {
                    let start = row * columns;
                    let end = (start + columns).min(list.len());
                    div()
                        .h(px(ROW_H))
                        .px(px(GRID_PAD))
                        .pt(px(GRID_GAP))
                        .flex()
                        .justify_between()
                        .children((start..start + columns).map(|index| {
                            if index >= end {
                                return div().w(px(TILE_W)).into_any_element();
                            }
                            let Some(file) = media.get(list[index]) else {
                                return div().w(px(TILE_W)).into_any_element();
                            };
                            let thumbnail = match file.file_info {
                                FileInfo::Audio { .. } => None,
                                _ => thumbs.want(file.id),
                            };
                            let missing = thumbs.is_missing(file.id);
                            tile(file, thumbnail, missing, &selection, focused, browser.clone())
                                .into_any_element()
                        }))
                })
                .collect()
        })
        .track_scroll(&self.scroll);

        let width_columns = self.columns.clone();
        let measured = self.measured.clone();
        let pack = self.editor.read(cx).pack().clone();
        let thumbs = self.thumbs.clone();
        let owner = cx.weak_entity();
        let clear = cx.entity();
        div()
            .id("media-grid")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(c(theme::BG))
            .on_click(move |_, window, cx| {
                clear.update(cx, |this, cx| this.clear_selection(window, cx))
            })
            .child(items.size_full())
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let usable = f32::from(bounds.size.width) - GRID_PAD * 2. + GRID_GAP;
                        let fit = ((usable / (TILE_W + GRID_GAP)).floor() as usize).max(1);
                        let first = !measured.replace(true);
                        if fit != width_columns.get() || first {
                            width_columns.set(fit);
                            if let Some(owner) = owner.upgrade() {
                                owner.update(cx, |_, cx| cx.notify());
                            }
                        }
                        Thumbnails::end_frame(&thumbs, &pack, owner.clone(), window, cx);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
    }

    /// The Audio tab's list: one row per file with its role control.
    fn audio_list(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.list.clone();
        let editor = self.editor.clone();
        let browser = cx.weak_entity();
        let selection = Rc::new(self.selection.clone());
        let focused = self.focus.is_focused(window);
        let measured = self.measured.clone();
        let owner = cx.weak_entity();
        let items = uniform_list_rows(list.len(), move |range, _, cx| {
            let editor_entity = editor.clone();
            let editor = editor.read(cx);
            let media = editor.session().media();
            range
                .filter_map(|index| media.get(list[index]))
                .map(|file| {
                    audio_row(file, &selection, focused, browser.clone(), editor_entity.clone())
                        .into_any_element()
                })
                .collect()
        })
        .track_scroll(&self.scroll);
        let clear = cx.entity();
        div()
            .id("audio-list")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(c(theme::BG))
            .on_click(move |_, window, cx| {
                clear.update(cx, |this, cx| this.clear_selection(window, cx))
            })
            .child(items.size_full())
            .child(
                canvas(
                    move |_, _, cx| {
                        if !measured.replace(true) {
                            if let Some(owner) = owner.upgrade() {
                                owner.update(cx, |_, cx| cx.notify());
                            }
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
    }
}

fn uniform_list_rows<R: IntoElement>(
    count: usize,
    render: impl 'static + Fn(std::ops::Range<usize>, &mut Window, &mut App) -> Vec<R>,
) -> gpui_kit::UniformList {
    gpui_kit::uniform_list("media-rows", count, render)
}

fn list_words(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

/// One file in the grid: its thumbnail (or what stands in for one) and its name.
fn tile(
    file: &MediaFile,
    thumbnail: Option<std::sync::Arc<gpui_kit::RenderImage>>,
    missing: bool,
    selection: &Selection,
    focused: bool,
    browser: gpui_kit::WeakEntity<MediaBrowser>,
) -> impl IntoElement {
    let id = file.id;
    let selected = selection.contains(id);
    let active = focused && selection.active == Some(id);
    let picture = match (&file.file_info, thumbnail) {
        (_, Some(image)) => img(ImageSource::Render(image))
            .max_w_full()
            .max_h_full()
            .object_fit(ObjectFit::Contain)
            .into_any_element(),
        (FileInfo::Audio { duration }, None) => div()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(ui::label("Audio"))
            .child(ui::readout(format_duration(*duration)))
            .into_any_element(),
        (_, None) if missing => ui::readout("No preview").into_any_element(),
        // Still decoding: an empty frame, so nothing jumps when it lands.
        (_, None) => div().into_any_element(),
    };
    let video = matches!(file.file_info, FileInfo::Video { .. });
    div()
        .id(("tile", id as usize))
        .w(px(TILE_W))
        .h(px(ROW_H - GRID_GAP))
        .p(px(4.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .rounded(theme::RADIUS_MD)
        .border_2()
        .border_color(if active {
            c(theme::FOCUS)
        } else if selected {
            c(theme::ACCENT_HOVER)
        } else {
            gpui_kit::transparent_black()
        })
        .when(selected, |this| this.bg(c(theme::SURFACE_RAISED)))
        .when(!selected, |this| {
            this.hover(|style| style.bg(c(theme::SURFACE)))
        })
        .cursor_pointer()
        .child(
            div()
                .relative()
                .h(px(THUMB_H))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .child(picture)
                .when(video, |this| {
                    this.child(
                        div()
                            .absolute()
                            .left(px(4.))
                            .bottom(px(4.))
                            .px(px(4.))
                            .py(px(2.))
                            .rounded(theme::RADIUS_SM)
                            .bg(gpui_kit::rgba(0x000000a8))
                            .text_size(px(10.))
                            .text_color(c(theme::TEXT))
                            .child(gpui_kit::assets::IconName::Play),
                    )
                }),
        )
        .child(
            div()
                .px_1()
                .text_size(px(11.))
                .line_height(px(14.))
                .text_center()
                .line_clamp(2)
                .child(file.file_name.clone()),
        )
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            let _ = browser.update(cx, |this, cx| this.click(id, event, window, cx));
        })
}

/// One row in the Audio list: name, length and size, and its role as a two-state control that
/// acts on this row alone (the inspector changes a whole selection).
fn audio_row(
    file: &MediaFile,
    selection: &Selection,
    focused: bool,
    browser: gpui_kit::WeakEntity<MediaBrowser>,
    editor: Entity<Editor>,
) -> impl IntoElement {
    let id = file.id;
    let selected = selection.contains(id);
    let active = focused && selection.active == Some(id);
    let popup = is_popup_audio(file);
    let duration = match file.file_info {
        FileInfo::Audio { duration } => format_duration(duration),
        _ => String::new(),
    };
    let role_button = |label: &'static str, on: bool, popup: bool| {
        let editor = editor.clone();
        div()
            .id((label, id as usize))
            .px_2()
            .py(px(3.))
            .rounded(theme::RADIUS_SM)
            .text_size(px(11.5))
            .border_1()
            .when(on, |this| {
                this.bg(c(theme::SURFACE_RAISED))
                    .border_color(c(theme::ACCENT_HOVER))
                    .text_color(c(theme::TEXT))
            })
            .when(!on, |this| {
                this.border_color(gpui_kit::transparent_black())
                    .text_color(c(theme::MUTED))
                    .cursor_pointer()
                    .hover(|style| style.text_color(c(theme::TEXT)))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        editor.update(cx, |editor, cx| {
                            editor.apply(cx, |session| session.set_popup_audio(&[id], popup))
                        });
                    })
            })
            .child(label)
    };
    div()
        .id(("audio-row", id as usize))
        .relative()
        .h(px(AUDIO_ROW_H))
        .px_4()
        .flex()
        .items_center()
        .gap_3()
        .border_b_1()
        .border_color(c(theme::BORDER))
        .when(selected, |this| this.bg(c(theme::SURFACE_RAISED)))
        .when(!selected, |this| this.hover(|style| style.bg(c(theme::SURFACE))))
        .child(
            // The selection edge, or the focus ring's left side on the active row.
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(px(2.))
                .when(selected, |this| this.bg(c(theme::ACCENT_HOVER)))
                .when(active, |this| this.bg(c(theme::FOCUS))),
        )
        .when(active, |this| this.border_1().border_color(c(theme::FOCUS)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.5))
                        .child(file.file_name.clone()),
                )
                .child(ui::readout(format!("{duration} - {}", format_size(file.size)))),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .gap(px(2.))
                .p(px(2.))
                .rounded(theme::RADIUS_SM)
                .border_1()
                .border_color(c(theme::BORDER))
                .bg(c(theme::BG))
                .child(role_button("Background", !popup, false))
                .child(role_button("Popup", popup, true)),
        )
        .cursor_pointer()
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            let _ = browser.update(cx, |this, cx| this.click(id, event, window, cx));
        })
}

impl Render for MediaBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.measured.get() {
            if let Some(id) = self.reveal.take() {
                self.scroll_to(id);
            }
        }
        let body = if self.list.is_empty() {
            self.empty(cx).into_any_element()
        } else if self.tab == MediaTab::Audio {
            self.audio_list(window, cx).into_any_element()
        } else {
            self.grid(window, cx).into_any_element()
        };
        let dialog = self.removal_dialog(window, cx);
        let entity = cx.entity();
        let _: Pixels = px(0.);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.header())
            .child(self.toolbar(cx))
            .child(
                div()
                    .id("media-body")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .track_focus(&self.focus)
                    .on_key_down(move |event, window, cx| {
                        entity.update(cx, |this, cx| this.key_down(event, window, cx))
                    })
                    .child(body)
                    .child(self.inspector.clone()),
            )
            .children(self.viewer.clone())
            .children(dialog)
    }
}
