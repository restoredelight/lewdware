//! A full-size view of one file over the window, stepped through a list with the arrow keys or
//! the side buttons; Escape or the close button leaves. Ported from the looking half of
//! `pack-editor/src/lib/MediaViewer.svelte` (the popup editor it also is in Popups isn't ported).
//!
//! Walks the tab's list as filtered and sorted, or the selection when it was opened over several
//! files. Each step loads through a fresh generation, so a held arrow key only ever shows the
//! file it stopped on; dropping the viewer releases its preview (`media-gpui/README.md`).

use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, Context, Entity, EventEmitter, FocusHandle, FontWeight, SharedString, Task, Window,
};
use media_gpui::MediaView;

use super::plural;
use crate::editor::Editor;
use crate::theme::{self, c};

pub(crate) enum ViewerEvent {
    Close,
    /// The viewer moved to this file; the grid follows it.
    Moved(u64),
}

impl EventEmitter<ViewerEvent> for Viewer {}

pub(crate) struct Viewer {
    editor: Entity<Editor>,
    ids: Vec<u64>,
    index: usize,
    /// Walking a selection rather than the whole list.
    selection: bool,
    preview: Entity<MediaView>,
    generation: u64,
    _loading: Option<Task<()>>,
    focus: FocusHandle,
}

impl Viewer {
    pub fn new(
        editor: Entity<Editor>,
        ids: Vec<u64>,
        start: u64,
        selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let index = ids.iter().position(|&id| id == start).unwrap_or(0);
        let mut viewer = Self {
            editor,
            ids,
            index,
            selection,
            preview: cx.new(|cx| MediaView::new(window, cx)),
            generation: 0,
            _loading: None,
            focus: cx.focus_handle(),
        };
        viewer.load(window, cx);
        viewer
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn current(&self) -> Option<u64> {
        self.ids.get(self.index).copied()
    }

    /// Drops files that are gone (removed, or undone away); closes if none are left.
    pub fn retain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current();
        let media = self.editor.read(cx).session().media();
        self.ids.retain(|&id| media.get(id).is_some());
        if self.ids.is_empty() {
            cx.emit(ViewerEvent::Close);
            return;
        }
        match current.and_then(|id| self.ids.iter().position(|&other| other == id)) {
            Some(index) => self.index = index,
            None => {
                self.index = self.index.min(self.ids.len() - 1);
                self.load(window, cx);
            }
        }
        cx.notify();
    }

    fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.index as isize + by;
        if next < 0 || next as usize >= self.ids.len() {
            return;
        }
        self.index = next as usize;
        self.load(window, cx);
        if let Some(id) = self.current() {
            cx.emit(ViewerEvent::Moved(id));
        }
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self._loading = None;
        let generation = self.generation;
        let Some(file) = self
            .current()
            .and_then(|id| self.editor.read(cx).session().media().get(id).cloned())
        else {
            self.preview.update(cx, |preview, cx| preview.clear(window, cx));
            return;
        };
        self.preview
            .update(cx, |preview, cx| preview.begin_loading(window, cx));
        let opening = super::open_preview(self.editor.read(cx), &file);
        self._loading = Some(cx.spawn_in(window, async move |this, cx| {
            let result = opening.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.preview.update(cx, |preview, cx| match result {
                    Ok(source) => preview.load(source, window, cx),
                    Err(error) => preview.fail(format!("{error:#}"), cx),
                });
                // `load` focuses the preview for its own keys; the viewer keeps the arrows.
                window.focus(&this.focus, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn nav_button(
        id: &'static str,
        icon: gpui_kit::assets::IconName,
        enabled: bool,
        on_click: impl Fn(&mut Window, &mut gpui_kit::App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .size(px(40.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .bg(c(theme::SURFACE))
            .text_size(px(18.))
            .when(!enabled, |this| this.opacity(0.3))
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                    .on_click(move |_, window, cx| on_click(window, cx))
            })
            .child(icon)
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let name: SharedString = self
            .current()
            .and_then(|id| self.editor.read(cx).session().media().get(id))
            .map(|file| file.file_name.clone().into())
            .unwrap_or_default();
        let position = format!(
            "{} of {}{}",
            self.index + 1,
            self.ids.len(),
            if self.selection { " selected" } else { "" }
        );
        let entity = cx.entity();
        let (prev, next, close) = (entity.clone(), entity.clone(), entity.clone());
        let keys = entity.clone();
        let has_prev = self.index > 0;
        let has_next = self.index + 1 < self.ids.len();
        let _ = plural;

        gpui_kit::deferred(
            gpui_kit::anchored()
                .position(gpui_kit::point(px(0.), px(0.)))
                .child(
                    div()
                        .id("viewer")
                        .track_focus(&self.focus)
                        .occlude()
                        .w(viewport.width)
                        .h(viewport.height)
                        .flex()
                        .flex_col()
                        .bg(gpui_kit::rgba(0x0a0809f2))
                        .on_key_down(move |event, window, cx| {
                            let by = match event.keystroke.key.as_str() {
                                "left" => -1,
                                "right" => 1,
                                "escape" => {
                                    keys.update(cx, |_, cx| cx.emit(ViewerEvent::Close));
                                    cx.stop_propagation();
                                    return;
                                }
                                _ => return,
                            };
                            cx.stop_propagation();
                            keys.update(cx, |this, cx| this.step(by, window, cx));
                        })
                        .child(
                            div()
                                .flex_none()
                                .h(px(40.))
                                .px_3()
                                .flex()
                                .items_center()
                                .gap_3()
                                .border_b_1()
                                .border_color(c(theme::BORDER))
                                .bg(c(theme::SURFACE))
                                .child(
                                    div()
                                        .size(px(8.))
                                        .flex_none()
                                        .rounded_full()
                                        .bg(c(theme::ACCENT)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(theme::FONT_MONO)
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::BOLD)
                                        .child(name),
                                )
                                .child(crate::ui::readout(position))
                                .child(
                                    div()
                                        .id("viewer-close")
                                        .size(px(28.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(theme::RADIUS_SM)
                                        .text_color(c(theme::MUTED))
                                        .cursor_pointer()
                                        .hover(|style| {
                                            style.bg(c(theme::SURFACE_RAISED)).text_color(c(theme::TEXT))
                                        })
                                        .child(gpui_kit::assets::IconName::Close)
                                        .on_click(move |_, _, cx| {
                                            close.update(cx, |_, cx| cx.emit(ViewerEvent::Close))
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .p_4()
                                .flex()
                                .items_center()
                                .gap_4()
                                .child(Self::nav_button(
                                    "viewer-prev",
                                    gpui_kit::assets::IconName::ChevronLeft,
                                    has_prev,
                                    move |window, cx| {
                                        prev.update(cx, |this, cx| this.step(-1, window, cx))
                                    },
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .h_full()
                                        .child(self.preview.clone()),
                                )
                                .child(Self::nav_button(
                                    "viewer-next",
                                    gpui_kit::assets::IconName::ChevronRight,
                                    has_next,
                                    move |window, cx| {
                                        next.update(cx, |this, cx| this.step(1, window, cx))
                                    },
                                )),
                        ),
                ),
        )
        .with_priority(2)
    }
}
