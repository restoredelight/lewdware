//! A full-size view of one file over the window, stepped through a list with the arrow keys or
//! the side buttons. Escape, the close button, or a click anywhere outside the media leaves.
//! Ported from `pack-editor/src/lib/MediaViewer.svelte` and `MediaOverlay.svelte`.
//!
//! In the Popups tab it is also the popup editor: the media is drawn as the popup it becomes
//! (with its caption, edited in place, in the titlebar), the options rail beside it sets what the
//! file does as a popup (`rail.rs`), and "Size & position" swaps the media for a frame standing
//! in for the screen (`placement.rs`). Opened over a selection, every control writes to all of it
//! and shows "Mixed" where the files disagree; the caption alone is only ever this file's.
//!
//! Walks the tab's list as filtered and sorted, or the selection when it was opened over several
//! files. Each step loads through a fresh generation, so a held arrow key only ever shows the
//! file it stopped on; dropping the viewer releases its preview (`media-gpui/README.md`).

mod placement;
mod rail;

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{SliderEvent, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, rgba, AnyElement, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    Pixels, SharedString, Subscription, Task, Window,
};
use media_gpui::MediaView;
use pack_editor_core::changes::Scope;
use pack_editor_core::pack::MediaFile;
use pack_editor_core::session::library::MediaTab;
use pack_editor_core::session::{Applied, Session};
use shared::behaviour::{PopupMedia, SpawnRegion};
use shared::encode::FileInfo;

use crate::editor::{CommitPending, Editor, EditorEvent};
use crate::icons::icon;
use crate::theme::{self, c};
use crate::views::timeline::{stage_membership, StageMembership};

pub(crate) enum ViewerEvent {
    Close,
    /// The viewer moved to this file; the grid follows it.
    Moved(u64),
}

impl EventEmitter<ViewerEvent> for Viewer {}

/// The options rail's width.
const RAIL_W: f32 = 280.;
/// The room kept around the media for the chip above it and the step buttons beside it.
const MEDIA_PAD_X: f32 = 56.;
/// The media sits below the file-name chip (12px down, about 60px tall), so a tall file's
/// popup titlebar is never under it, with this much room left beneath it.
const MEDIA_TOP: f32 = 84.;
const MEDIA_BOTTOM: f32 = 32.;
/// The popup frame's titlebar, which holds the caption.
const POPUP_HEADER_H: f32 = 26.;
/// MediaView's play/seek/volume bar under a video or audio preview.
const CONTROLS_H: f32 = media_gpui::CONTROLS_HEIGHT;

/// The text fields the viewer owns. Each commits on Enter, on losing focus, on a click anywhere
/// in the viewer, and before the viewer steps or closes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Field {
    Weight,
    Scale,
    Left,
    Top,
    Width,
    Height,
    Caption,
}

impl Field {
    const ALL: [Field; 7] = [
        Field::Weight,
        Field::Scale,
        Field::Left,
        Field::Top,
        Field::Width,
        Field::Height,
        Field::Caption,
    ];
}

/// A value across the files an edit applies to: the one they share, or that they disagree.
#[derive(Clone, Copy)]
struct Shared<T> {
    value: Option<T>,
    mixed: bool,
}

pub(crate) struct Viewer {
    editor: Entity<Editor>,
    ids: Vec<u64>,
    index: usize,
    /// Walking a selection rather than the whole list: edits apply to all of it.
    selection: bool,
    /// The Popups tab, where the viewer is also the popup editor.
    popups: bool,
    preview: Entity<MediaView>,
    generation: u64,
    _loading: Option<Task<()>>,
    focus: FocusHandle,
    /// The popup attributes of the files edits apply to, re-read when they change.
    attributes: HashMap<u64, PopupMedia>,
    inputs: HashMap<Field, Entity<InputState>>,
    /// What each field was last given to show: typing that differs from it is an edit.
    shown: HashMap<Field, String>,
    volume: Entity<SliderState>,
    /// The clip level under the slider's thumb, until the release writes it.
    dragged_volume: Option<f32>,
    membership: Option<Entity<StageMembership>>,
    /// In the size-and-position frame rather than looking at the media.
    placing: bool,
    placement: placement::State,
    _subscriptions: Vec<Subscription>,
}

impl Viewer {
    pub fn new(
        editor: Entity<Editor>,
        tab: MediaTab,
        ids: Vec<u64>,
        start: u64,
        selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let index = ids.iter().position(|&id| id == start).unwrap_or(0);
        let mut subscriptions = Vec::new();
        let mut inputs = HashMap::new();
        for field in Field::ALL {
            let input = cx.new(|cx| {
                let input = InputState::new(window, cx);
                match field {
                    Field::Caption => input.placeholder("Caption, from the pool if blank"),
                    Field::Scale => input.placeholder("Auto"),
                    _ => input,
                }
            });
            subscriptions.push(cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_fields(cx);
                }
            }));
            subscriptions.extend(crate::ui::track_focus_of(&input, window, cx));
            inputs.insert(field, input);
        }
        let volume = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.05)
                .default_value(1.)
        });
        subscriptions.push(cx.subscribe_in(
            &volume,
            window,
            |this, _, event: &SliderEvent, window, cx| {
                let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
                let SliderValue::Single(value) = *value else {
                    return;
                };
                this.dragged_volume = Some(value);
                this.preview
                    .update(cx, |preview, cx| preview.set_volume(value, cx));
                if matches!(event, SliderEvent::Release(_)) {
                    let label = this.label("Set video volume", cx);
                    let targets = this.targets();
                    this.apply(cx, |s| {
                        s.set_popup_video_volume(&targets, Some(value as f64), &label)
                    });
                    this.dragged_volume = None;
                }
                let _ = window;
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
            let EditorEvent::Changed(changes) = event;
            if changes.scopes.contains(Scope::Attributes) {
                this.refresh_attributes(cx);
            }
        }));
        subscriptions.push(cx.subscribe(&editor, |this, _, _: &CommitPending, cx| {
            this.commit_fields(cx)
        }));

        let popups = tab == MediaTab::Popups;
        let mut viewer = Self {
            editor,
            ids,
            index,
            selection,
            popups,
            preview: cx.new(|cx| MediaView::new(window, cx)),
            generation: 0,
            _loading: None,
            focus: cx.focus_handle(),
            attributes: HashMap::new(),
            inputs,
            shown: HashMap::new(),
            volume,
            dragged_volume: None,
            membership: None,
            placing: false,
            placement: placement::State::default(),
            _subscriptions: subscriptions,
        };
        viewer.refresh_attributes(cx);
        viewer.load(window, cx);
        viewer
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn current(&self) -> Option<u64> {
        self.ids.get(self.index).copied()
    }

    fn file(&self, cx: &gpui_kit::App) -> Option<MediaFile> {
        let id = self.current()?;
        self.editor.read(cx).session().media().get(id).cloned()
    }

    /// The files an edit applies to: the whole selection when walking one, else this file.
    fn targets(&self) -> Vec<u64> {
        if self.selection {
            self.ids.clone()
        } else {
            self.current().into_iter().collect()
        }
    }

    /// Whether this file is being edited as a popup (the Popups tab, and not audio).
    fn editing(&self, cx: &gpui_kit::App) -> bool {
        self.popups
            && self
                .file(cx)
                .is_some_and(|file| !matches!(file.file_info, FileInfo::Audio { .. }))
    }

    /// An undo label naming what an edit changed.
    fn label(&self, what: &str, cx: &gpui_kit::App) -> String {
        let count = self.targets().len();
        match (count, self.file(cx)) {
            (1, Some(file)) => format!("{what} for \"{}\"", file.file_name),
            (1, None) => what.to_string(),
            _ => format!("{what} for {count} items"),
        }
    }

    fn apply(
        &self,
        cx: &mut Context<Self>,
        write: impl FnOnce(&mut Session) -> anyhow::Result<Applied<()>>,
    ) {
        self.editor.update(cx, |editor, cx| editor.apply(cx, write));
    }

    fn refresh_attributes(&mut self, cx: &mut Context<Self>) {
        let targets = self.targets();
        self.attributes = self
            .editor
            .read(cx)
            .session()
            .popup_attributes(&targets)
            .map(|rows| rows.into_iter().collect())
            .unwrap_or_default();
        cx.notify();
    }

    fn attributes_of(&self, id: u64) -> PopupMedia {
        self.attributes.get(&id).cloned().unwrap_or_default()
    }

    /// This file's attributes: what the frame and the caption show.
    fn current_attributes(&self) -> PopupMedia {
        self.current()
            .map(|id| self.attributes_of(id))
            .unwrap_or_default()
    }

    fn shared<T: PartialEq>(&self, field: impl Fn(&PopupMedia) -> Option<T>) -> Shared<T> {
        let mut values = self
            .targets()
            .into_iter()
            .map(|id| self.attributes.get(&id).and_then(&field));
        let first = values.next().flatten();
        let mixed = values.any(|value| value != first);
        Shared {
            value: if mixed { None } else { first },
            mixed,
        }
    }

    /// Follows the tab's `list` after a change: walking the whole list, it takes the new one
    /// (so imported files can be stepped to), keeping the file on screen in place if an edit
    /// filtered it out; walking a selection, it drops files that are gone. Closes if none are
    /// left.
    pub fn sync(&mut self, list: &[u64], window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current();
        let media = self.editor.read(cx).session().media();
        if self.selection {
            self.ids.retain(|&id| media.get(id).is_some());
        } else {
            let mut ids = list.to_vec();
            if let Some(id) = current.filter(|&id| media.get(id).is_some() && !list.contains(&id)) {
                ids.insert(self.index.min(ids.len()), id);
            }
            self.ids = ids;
        }
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
        self.commit_fields(cx);
        self.index = next as usize;
        // The frame is for walking a selection; a single file's step leaves it.
        if !self.selection {
            self.placing = false;
        }
        if !self.selection {
            self.refresh_attributes(cx);
        }
        self.load(window, cx);
        if let Some(id) = self.current() {
            cx.emit(ViewerEvent::Moved(id));
        }
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        cx.emit(ViewerEvent::Close);
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation += 1;
        self._loading = None;
        self.dragged_volume = None;
        let generation = self.generation;
        let Some(file) = self.file(cx) else {
            self.preview
                .update(cx, |preview, cx| preview.clear(window, cx));
            return;
        };
        if let Some(membership) = &self.membership {
            membership.update(cx, |membership, cx| membership.set_media(file.id, cx));
        } else if self.popups {
            self.membership = Some(stage_membership(self.editor.clone(), file.id, cx));
        }
        self.preview
            .update(cx, |preview, cx| preview.begin_loading(window, cx));
        // In the popup editor the clip plays at the level the pack gives it, so the author hears
        // what players will.
        let level = self
            .editing(cx)
            .then(|| self.attributes_of(file.id).video_volume.unwrap_or(1.) as f32);
        let placing = self.placing;
        self.preview.update(cx, |preview, cx| {
            if let Some(level) = level {
                preview.set_volume(level, cx);
            }
            preview.set_controls(!placing, cx);
        });
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

    fn set_placing(&mut self, placing: bool, cx: &mut Context<Self>) {
        self.commit_fields(cx);
        self.placing = placing;
        self.placement = placement::State::default();
        self.preview
            .update(cx, |preview, cx| preview.set_controls(!placing, cx));
        cx.notify();
    }

    // ── Fields ──────────────────────────────────────────────────────────────────────────────

    fn input(&self, field: Field) -> &Entity<InputState> {
        &self.inputs[&field]
    }

    /// What `field` shows for the stored attributes.
    fn stored_text(&self, field: Field) -> String {
        let number = |value: Option<f64>| value.map(format_number).unwrap_or_default();
        let current = self.current_attributes();
        let region = current.region.unwrap_or(SpawnRegion::FULL).sanitized();
        let percent = |value: f64| format!("{}", (value * 100.).round());
        match field {
            Field::Weight => {
                let weight = self.shared(|a| a.weight);
                if weight.mixed {
                    String::new()
                } else {
                    number(weight.value)
                }
            }
            Field::Scale => number(current.scale),
            Field::Left => percent(region.x),
            Field::Top => percent(region.y),
            Field::Width => percent(region.width),
            Field::Height => percent(region.height),
            Field::Caption => current.caption.unwrap_or_default(),
        }
    }

    /// Puts the stored values into every field the author isn't typing in.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in Field::ALL {
            let stored = self.stored_text(field);
            let input = self.input(field).clone();
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            if !focused && input.read(cx).value().as_ref() != stored {
                input.update(cx, |input, cx| input.set_value(stored.clone(), window, cx));
            }
            self.shown.insert(field, stored);
        }
    }

    /// Writes every field whose text differs from what it was given to show.
    fn commit_fields(&mut self, cx: &mut Context<Self>) {
        for field in Field::ALL {
            let typed = self.input(field).read(cx).value().trim().to_string();
            let shown = self.shown.get(&field).map(|s| s.trim().to_string());
            if shown.as_deref().is_none_or(|shown| shown == typed) {
                continue;
            }
            self.shown.insert(field, typed.clone());
            self.write_field(field, &typed, cx);
        }
    }

    fn write_field(&mut self, field: Field, text: &str, cx: &mut Context<Self>) {
        let targets = self.targets();
        let number = text
            .trim_end_matches(['×', 'x', '%'])
            .trim()
            .parse::<f64>()
            .ok();
        match field {
            Field::Weight => {
                let label = self.label("Set popup frequency", cx);
                self.apply(cx, |s| s.set_popup_weight(&targets, number, &label));
            }
            Field::Scale => {
                let label = self.label("Set popup size", cx);
                self.apply(cx, |s| s.set_popup_scale(&targets, number, &label));
            }
            Field::Left | Field::Top | Field::Width | Field::Height => {
                let Some(value) = number.filter(|n| n.is_finite()) else {
                    // Nonsense puts the stored value back.
                    cx.notify();
                    return;
                };
                let value = value / 100.;
                let mut region = self
                    .current_attributes()
                    .region
                    .unwrap_or(SpawnRegion::FULL)
                    .sanitized();
                match field {
                    Field::Left => region.x = value,
                    Field::Top => region.y = value,
                    Field::Width => region.width = value,
                    _ => region.height = value,
                }
                let label = self.label("Set popup area", cx);
                self.apply(cx, |s| s.set_popup_region(&targets, Some(region), &label));
            }
            Field::Caption => {
                // A sentence about this picture: never written to the rest of a selection.
                let Some(file) = self.file(cx) else {
                    return;
                };
                let label = format!("Set caption for \"{}\"", file.file_name);
                let text = text.to_string();
                self.apply(cx, |s| s.set_popup_caption(&[file.id], Some(&text), &label));
            }
        }
    }

    fn key_down(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let typing = self
            .inputs
            .values()
            .any(|input| input.read(cx).focus_handle(cx).is_focused(window));
        match event.keystroke.key.as_str() {
            "escape" if typing => {
                // Put the stored value back and leave the field.
                self.shown.clear();
                window.focus(&self.focus, cx);
                self.sync_fields(window, cx);
            }
            "escape" if self.placing => self.set_placing(false, cx),
            "escape" => self.close(cx),
            "tab" => self.commit_fields(cx),
            "left" if !typing => self.step(-1, window, cx),
            "right" if !typing => self.step(1, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    // ── Rendering ───────────────────────────────────────────────────────────────────────────

    /// The media's own pixel size, if it has one.
    fn media_size(file: &MediaFile) -> Option<(f32, f32)> {
        match file.file_info {
            FileInfo::Image { width, height, .. } | FileInfo::Video { width, height, .. } => {
                Some((width as f32, height as f32))
            }
            FileInfo::Audio { .. } => None,
        }
    }

    /// The media sized to fit `room` without growing past its own size, plus the controls bar a
    /// timed preview draws under it.
    fn fitted(file: &MediaFile, room: (f32, f32)) -> (Pixels, Pixels) {
        let timed = !matches!(file.file_info, FileInfo::Image { .. });
        let controls = if timed { CONTROLS_H } else { 0. };
        match Self::media_size(file) {
            Some((width, height)) if width > 0. && height > 0. => {
                let fit = (room.0 / width)
                    .min((room.1 - controls).max(40.) / height)
                    .min(1.);
                (px(width * fit), px(height * fit + controls))
            }
            // Audio: just the player.
            _ => (px(room.0.min(480.)), px(controls)),
        }
    }

    /// The media as the popup it becomes: the caption in its titlebar, the media below.
    fn popup_frame(
        &self,
        file: &MediaFile,
        room: (f32, f32),
        window: &Window,
        cx: &gpui_kit::App,
    ) -> AnyElement {
        let (width, height) = Self::fitted(file, (room.0, room.1 - POPUP_HEADER_H));
        let caption = self.input(Field::Caption).clone();
        let typing = caption.read(cx).focus_handle(cx).is_focused(window);
        let focus = caption.read(cx).focus_handle(cx);
        // A field, not a label: it shows a box on hover and while typing, and a pencil until
        // clicked, so it reads as somewhere to type.
        let field = div()
            .id("caption-field")
            .flex_1()
            .min_w_0()
            .h(px(20.))
            .mr(px(3.))
            .pr_1()
            .flex()
            .items_center()
            .gap_1()
            .rounded(px(3.))
            .border_1()
            .cursor_text()
            .map(|this| {
                if typing {
                    this.border_color(rgba(0x1c1c1c99)).bg(gpui_kit::white())
                } else {
                    this.border_color(gpui_kit::transparent_black())
                        .hover(|style| style.border_color(rgba(0x0000004d)).bg(rgba(0xffffff99)))
                }
            })
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new("Edit caption").build(window, cx)
            })
            .on_click(move |_, window, cx| window.focus(&focus, cx))
            .child(
                Input::new(&caption)
                    .appearance(false)
                    .text_size(px(12.))
                    .flex_1(),
            )
            .when(!typing, |this| {
                this.child(
                    icon(IconName::Pencil)
                        .size(px(11.))
                        .flex_none()
                        .text_color(rgba(0x1c1c1c8c)),
                )
            });
        div()
            .id("popup-frame")
            .occlude()
            .flex_none()
            .w(width.max(px(260.)))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(4.))
            .border_1()
            .border_color(rgba(0xffffff73))
            .bg(rgba(0xf8f8f8ff))
            .shadow(vec![gpui_kit::BoxShadow {
                color: gpui_kit::hsla(0., 0., 0., 0.6),
                offset: gpui_kit::point(px(0.), px(18.)),
                blur_radius: px(50.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(
                div()
                    .flex_none()
                    .h(px(POPUP_HEADER_H))
                    .pl_2()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .border_b_1()
                    .border_color(rgba(0x00000026))
                    .bg(rgba(0xe4e4e4ff))
                    .text_color(rgba(0x1c1c1cff))
                    .child(
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded_full()
                            .bg(c(theme::ACCENT)),
                    )
                    .child(field),
            )
            .child(
                div()
                    .h(height)
                    .flex()
                    .justify_center()
                    .bg(gpui_kit::black())
                    // The frame's full width, so the controls under a narrow (portrait) clip span
                    // the frame; the picture is fitted inside it.
                    .child(div().w_full().h_full().child(self.preview.clone())),
            )
            .into_any_element()
    }

    /// The media alone, at full size: every tab but Popups, and audio.
    fn plain_media(&self, file: &MediaFile, room: (f32, f32)) -> AnyElement {
        let (width, height) = Self::fitted(file, room);
        div()
            .id("viewer-media")
            .occlude()
            .flex_none()
            .w(width)
            .h(height)
            .child(self.preview.clone())
            .into_any_element()
    }

    fn round_button(
        id: &'static str,
        icon_name: IconName,
        size: f32,
        on_click: impl Fn(&mut Window, &mut gpui_kit::App) + 'static,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id(id)
            .occlude()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(rgba(0x00000099))
            .text_color(rgba(0xffffffcc))
            .cursor_pointer()
            .hover(|style| style.bg(rgba(0x000000cc)).text_color(gpui_kit::white()))
            .child(icon(icon_name).size(px(size * 0.5)))
            .on_click(move |_, window, cx| on_click(window, cx))
    }
}

/// A stored number as typed: `1.5`, `2`.
fn format_number(value: f64) -> String {
    let rounded = (value * 1000.).round() / 1000.;
    format!("{rounded}")
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_fields(window, cx);
        let viewport = window.viewport_size();
        let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
        let file = self.file(cx);
        let editing = self.editing(cx);
        let rail_w = if editing { RAIL_W } else { 0. };
        let room = (
            (vw - rail_w - MEDIA_PAD_X * 2.).max(120.),
            (vh - MEDIA_TOP - MEDIA_BOTTOM).max(120.),
        );
        let name: SharedString = file
            .as_ref()
            .map(|file| file.file_name.clone().into())
            .unwrap_or_default();
        let position = format!(
            "{} of {}{}",
            self.index + 1,
            self.ids.len(),
            if self.selection { " selected" } else { "" }
        );
        let entity = cx.entity();
        let has_prev = self.index > 0;
        let has_next = self.index + 1 < self.ids.len();

        let media = file.as_ref().map(|file| {
            if editing && self.placing {
                self.placement(file, room, window, cx)
            } else if editing {
                self.popup_frame(file, room, window, cx)
            } else {
                self.plain_media(file, room)
            }
        });
        let rail = (editing && file.is_some()).then(|| self.rail(window, cx));

        let close = entity.clone();
        let backdrop = entity.clone();
        let keys = entity.clone();
        let commit = entity.clone();
        let (prev, next) = (entity.clone(), entity.clone());

        gpui_kit::deferred(
            gpui_kit::anchored()
                .position(gpui_kit::point(px(0.), px(0.)))
                .child(
                    div()
                        .id("viewer")
                        .track_focus(&self.focus)
                        .occlude()
                        .relative()
                        .w(viewport.width)
                        .h(viewport.height)
                        // Capture, so the arrows step even after a click gave the preview focus
                        // (its own handler would otherwise take them).
                        .capture_key_down(move |event, window, cx| {
                            keys.update(cx, |this, cx| this.key_down(event, window, cx))
                        })
                        // A click anywhere commits what's typed: the field it leaves may not say so.
                        .capture_any_mouse_down(move |_, _, cx| {
                            commit.update(cx, |this, cx| this.commit_fields(cx))
                        })
                        // The scrim. A click on it, anywhere outside the media and the rail, closes.
                        .child(
                            div()
                                .id("viewer-backdrop")
                                .absolute()
                                .size_full()
                                .bg(rgba(0x000000cc))
                                .on_click(move |_, _, cx| {
                                    backdrop.update(cx, |this, cx| this.close(cx))
                                }),
                        )
                        .child(
                            div()
                                .absolute()
                                .size_full()
                                .flex()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .h_full()
                                        .pt(px(MEDIA_TOP))
                                        .pb(px(MEDIA_BOTTOM))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .children(media),
                                )
                                .children(rail),
                        )
                        // The file-name chip.
                        .child(
                            div()
                                .absolute()
                                .top_3()
                                .left_3()
                                .max_w(px((vw - rail_w - 80.).clamp(120., 704.)))
                                .px_3()
                                .py_2()
                                .rounded(px(6.))
                                .bg(rgba(0x000000a6))
                                .text_color(gpui_kit::white())
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(name),
                                )
                                .child(
                                    div()
                                        .mt(px(2.))
                                        .text_size(px(11.))
                                        .text_color(rgba(0xffffffa6))
                                        .child(position),
                                ),
                        )
                        .child(
                            Self::round_button(
                                "viewer-close",
                                IconName::Close,
                                36.,
                                move |_, cx| close.update(cx, |this, cx| this.close(cx)),
                            )
                            .absolute()
                            .top_3()
                            .right(px(rail_w + 12.)),
                        )
                        .when(has_prev && !self.placing, |this| {
                            this.child(
                                Self::round_button(
                                    "viewer-prev",
                                    IconName::ChevronLeft,
                                    40.,
                                    move |window, cx| {
                                        prev.update(cx, |this, cx| this.step(-1, window, cx))
                                    },
                                )
                                .absolute()
                                .left_2()
                                .top(px(vh / 2. - 20.)),
                            )
                        })
                        .when(has_next && !self.placing, |this| {
                            this.child(
                                Self::round_button(
                                    "viewer-next",
                                    IconName::ChevronRight,
                                    40.,
                                    move |window, cx| {
                                        next.update(cx, |this, cx| this.step(1, window, cx))
                                    },
                                )
                                .absolute()
                                .right(px(rail_w + 10.))
                                .top(px(vh / 2. - 20.)),
                            )
                        }),
                ),
        )
        .with_priority(2)
    }
}
