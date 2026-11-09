//! The pieces the stage and transition editors are built from: cards, toggle rows, tag lists and
//! media slots.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, App, Context, Entity, Focusable, FontWeight, SharedString, Subscription,
    Window,
};
use shared::behaviour::MediaSlot;
use shared::encode::FileInfo;

use super::fields::Spec;
use super::TimelineView;
use crate::theme::{self, c};
use crate::ui::{self, Button, Size, Variant};

/// How many files a slot's picker lists before asking for a narrower search.
const PICKER_ROWS: usize = 40;

/// A toggle's handler with access to the view, as `cx.listener` gives a click's.
pub(super) fn listen_bool<V: 'static>(
    cx: &Context<V>,
    f: impl Fn(&mut V, &bool, &mut Window, &mut Context<V>) + 'static,
) -> impl Fn(bool, &mut Window, &mut App) + 'static {
    let view = cx.entity();
    move |on, window, cx| view.update(cx, |this, cx| f(this, &on, window, cx))
}

/// A card with a title and a line under it, and an optional control (a toggle) on the right.
pub(super) fn section(
    title: &'static str,
    description: impl Into<SharedString>,
    control: Option<AnyElement>,
) -> gpui_kit::Div {
    ui::card().p_4().gap_3().child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(ui::section_title(title).text_size(px(15.)))
                    .child(ui::hint(description)),
            )
            .children(control),
    )
}

/// A row with a label, a note under it, and a control on the right, seamed off from what's above.
pub(super) fn toggle_row(
    title: impl Into<SharedString>,
    note: Option<SharedString>,
    control: impl IntoElement,
    seam: bool,
) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .when(seam, |this| {
            this.pt(px(10.)).border_t_1().border_color(c(theme::BORDER))
        })
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                )
                .children(note.map(|note| ui::hint(note).text_size(px(11.)))),
        )
        .child(control)
}

/// Fields side by side, two to a row.
pub(super) fn field_grid(children: Vec<AnyElement>) -> gpui_kit::Div {
    let mut grid = div().flex().flex_col().gap_3();
    let mut children = children.into_iter();
    while let Some(first) = children.next() {
        let second = children.next();
        grid = grid.child(
            div()
                .flex()
                .items_start()
                .gap_3()
                .child(div().flex_1().min_w_0().child(first))
                .child(div().flex_1().min_w_0().children(second)),
        );
    }
    grid
}

/// A small explanatory note.
pub(super) fn note(text: impl Into<SharedString>) -> gpui_kit::Div {
    ui::hint(text).text_size(px(11.))
}

/// A slot's picker, open under it: a search over the pack's files of the kind the slot takes.
pub(super) struct Picking {
    slot: MediaSlot,
    search: Entity<InputState>,
    _events: Subscription,
}

/// What a slot can hold. The picker offers only what the feature would actually use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Accepts {
    Image,
    ImageOrVideo,
    Audio,
}

impl Accepts {
    fn of(slot: &MediaSlot) -> Self {
        match slot {
            MediaSlot::StageAudio { .. }
            | MediaSlot::StageEntrySound { .. }
            | MediaSlot::StagePromptSound { .. } => Accepts::Audio,
            MediaSlot::Splash | MediaSlot::StageEntrySplash { .. } => Accepts::ImageOrVideo,
            MediaSlot::Wallpaper | MediaSlot::StageWallpaper { .. } => Accepts::Image,
        }
    }

    fn takes(self, info: &FileInfo) -> bool {
        matches!(
            (self, info),
            (Accepts::Image, FileInfo::Image { .. })
                | (
                    Accepts::ImageOrVideo,
                    FileInfo::Image { .. } | FileInfo::Video { .. }
                )
                | (Accepts::Audio, FileInfo::Audio { .. })
        )
    }

    fn noun(self) -> &'static str {
        match self {
            Accepts::Image => "images",
            Accepts::ImageOrVideo => "images or videos",
            Accepts::Audio => "audio files",
        }
    }
}

fn kind_label(info: &FileInfo) -> &'static str {
    match info {
        FileInfo::Image { .. } => "image",
        FileInfo::Video { .. } => "video",
        FileInfo::Audio { .. } => "audio",
    }
}

/// A key for an element id or a field, unique per slot.
pub(super) fn slot_key(slot: &MediaSlot) -> String {
    match slot {
        MediaSlot::Wallpaper => "wallpaper".to_string(),
        MediaSlot::Splash => "splash".to_string(),
        MediaSlot::StageWallpaper { stage } => format!("stage-wallpaper-{stage}"),
        MediaSlot::StageAudio { stage } => format!("stage-audio-{stage}"),
        MediaSlot::StageEntrySplash { stage } => format!("stage-entry-splash-{stage}"),
        MediaSlot::StageEntrySound { stage } => format!("stage-entry-sound-{stage}"),
        MediaSlot::StagePromptSound { stage } => format!("stage-prompt-sound-{stage}"),
    }
}

/// What a media slot row shows around the file.
pub(super) struct SlotSpec {
    pub slot: MediaSlot,
    pub title: &'static str,
    pub description: Option<&'static str>,
    /// Shown when the slot is empty: what leaving it empty means.
    pub empty_note: SharedString,
}

impl TimelineView {
    fn open_picker(&mut self, slot: MediaSlot, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search files"));
        let _events = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        search.update(cx, |search, cx| search.focus(window, cx));
        self.picking = Some(Picking {
            slot,
            search,
            _events,
        });
        cx.notify();
    }

    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picking = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn fill_slot(
        &mut self,
        slot: MediaSlot,
        media: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.picking = None;
        window.focus(&self.focus, cx);
        if matches!(slot, MediaSlot::StageAudio { .. }) {
            self.audio_picker_stage = None;
        }
        self.apply(cx, |session| session.set_stage_slot(&slot, media));
    }

    fn clear_slot(&mut self, slot: MediaSlot, cx: &mut Context<Self>) {
        self.picking = None;
        self.apply(cx, |session| session.clear_stage_slot(&slot));
    }

    /// One media slot: the file in it (or what leaving it empty means), "Choose from pack" and
    /// "Remove", and the picker when it's open.
    ///
    /// Importing a new file from disk straight into a slot isn't ported yet; it needs the import
    /// pipeline (Shell and lifecycle track). Import the file first, then choose it here.
    pub(super) fn media_slot(
        &self,
        spec: SlotSpec,
        current: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = slot_key(&spec.slot);
        let accepts = Accepts::of(&spec.slot);
        let session = self.editor.read(cx).session();
        let file = current.and_then(|id| session.media().get(id));
        let open = self
            .picking
            .as_ref()
            .is_some_and(|picking| picking.slot == spec.slot);

        let shown: AnyElement = match (current, file) {
            (Some(_), Some(file)) => div()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .child(file.file_name.clone()),
                )
                .child(ui::readout(kind_label(&file.file_info)).flex_none())
                .into_any_element(),
            (Some(id), None) => {
                note(format!("File {id} is no longer in the pack.")).into_any_element()
            }
            (None, _) => note(spec.empty_note.clone()).into_any_element(),
        };

        let choose_slot = spec.slot.clone();
        let clear = spec.slot.clone();
        let row = div()
            .min_h(px(44.))
            .px_3()
            .py_2()
            .flex()
            .items_center()
            .gap_3()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::BG))
            .child(div().flex_1().min_w_0().child(shown))
            .child(
                Button::new(
                    SharedString::from(format!("{key}-choose")),
                    if open {
                        "Cancel"
                    } else if current.is_some() {
                        "Replace"
                    } else {
                        "Choose from pack"
                    },
                )
                .variant(Variant::Secondary)
                .size(Size::Compact)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if open {
                        this.close_picker(window, cx)
                    } else {
                        this.open_picker(choose_slot.clone(), window, cx)
                    }
                })),
            )
            .when(current.is_some(), |this| {
                this.child(
                    Button::new(SharedString::from(format!("{key}-remove")), "Remove")
                        .variant(Variant::Quiet)
                        .size(Size::Compact)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.clear_slot(clear.clone(), cx)),
                        ),
                )
            });

        let picker = open.then(|| self.picker(&spec.slot, accepts, current, window, cx));

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(spec.title),
                    )
                    .children(spec.description.map(note)),
            )
            .child(row)
            .children(picker)
            .into_any_element()
    }

    fn picker(
        &self,
        slot: &MediaSlot,
        accepts: Accepts,
        current: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(picking) = &self.picking else {
            return div().into_any_element();
        };
        let query = picking.search.read(cx).value().trim().to_lowercase();
        let session = self.editor.read(cx).session();
        let mut matching = session
            .media()
            .iter()
            .filter(|file| accepts.takes(&file.file_info))
            .filter(|file| query.is_empty() || file.file_name.to_lowercase().contains(&query));
        let shown: Vec<_> = matching.by_ref().take(PICKER_ROWS).collect();
        let more = matching.count();
        let focused = picking.search.read(cx).focus_handle(cx).is_focused(window);
        let key = slot_key(slot);

        let rows: Vec<AnyElement> = shown
            .iter()
            .map(|file| {
                let (slot, id) = (slot.clone(), file.id);
                let chosen = current == Some(id);
                div()
                    .id(SharedString::from(format!("{key}-pick-{id}")))
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(theme::RADIUS_SM)
                    .cursor_pointer()
                    .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                    .when(chosen, |this| {
                        this.border_l_2().border_color(c(theme::ACCENT_HOVER))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .child(file.file_name.clone()),
                    )
                    .child(ui::readout(kind_label(&file.file_info)).flex_none())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.fill_slot(slot.clone(), id, window, cx)
                    }))
                    .into_any_element()
            })
            .collect();

        let view = cx.entity();
        div()
            .p_2()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .bg(c(theme::SURFACE))
            .child(
                div()
                    .h(theme::CONTROL_COMPACT)
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(if focused { theme::FOCUS } else { theme::BORDER }))
                    .bg(c(theme::BG))
                    .on_key_down(move |event, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            view.update(cx, |this, cx| this.close_picker(window, cx));
                        }
                    })
                    .child(
                        Input::new(&picking.search)
                            .appearance(false)
                            .text_size(px(13.))
                            .flex_1(),
                    ),
            )
            .child(if rows.is_empty() {
                note(format!("No {} match.", accepts.noun())).into_any_element()
            } else {
                div()
                    .id(SharedString::from(format!("{key}-picker-list")))
                    .max_h(px(240.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(rows)
                    .into_any_element()
            })
            .when(more > 0, |this| {
                this.child(note(format!("{more} more. Type to narrow the list.")))
            })
            .into_any_element()
    }

    /// A list of tags as removable chips, with a field to add one and suggestions from the pack's
    /// tags. `owned` is marked as the stage's own.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tag_list(
        &mut self,
        key: String,
        tags: &[String],
        owned: Option<&str>,
        all_tags: &[String],
        change: impl Fn(&mut Self, String, bool, &mut Context<Self>) + Clone + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field_key: SharedString = format!("{key}-add").into();
        let add = change.clone();
        let input = self.field(
            Spec::new(field_key.clone(), "")
                .placeholder("Add tag...")
                .enter_only(),
            move |this, text, cx| {
                let tag = text.trim().to_string();
                if !tag.is_empty() {
                    add(this, tag, true, cx);
                }
            },
            window,
            cx,
        );
        let typed = input.read(cx).value().trim().to_lowercase();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);

        let chips: Vec<AnyElement> = tags
            .iter()
            .enumerate()
            .map(|(index, tag)| {
                let remove = change.clone();
                let name = tag.clone();
                let is_owned = owned == Some(tag.as_str());
                div()
                    .h(px(24.))
                    .pl_2()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(if is_owned {
                        theme::BORDER_STRONG
                    } else {
                        theme::BORDER
                    }))
                    .bg(c(theme::SURFACE_RAISED))
                    .text_size(px(12.))
                    .child(tag.clone())
                    .child(
                        div()
                            .id(SharedString::from(format!("{key}-remove-{index}")))
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(11.))
                            .text_color(c(theme::MUTED))
                            .cursor_pointer()
                            .hover(|style| style.text_color(c(theme::TEXT)))
                            .child(gpui_kit::assets::IconName::Close)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                remove(this, name.clone(), false, cx)
                            })),
                    )
                    .into_any_element()
            })
            .collect();

        // Suggestions while the field is in use: the pack's tags that aren't here yet.
        let suggestions: Vec<AnyElement> = if focused || !typed.is_empty() {
            all_tags
                .iter()
                .filter(|tag| !tags.contains(tag))
                .filter(|tag| typed.is_empty() || tag.to_lowercase().contains(&typed))
                .take(8)
                .enumerate()
                .map(|(index, tag)| {
                    let pick = change.clone();
                    let name = tag.clone();
                    let input = input.clone();
                    div()
                        .id(SharedString::from(format!("{key}-suggest-{index}")))
                        .h(px(24.))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded(theme::RADIUS_SM)
                        .border_1()
                        .border_color(c(theme::BORDER))
                        .text_size(px(12.))
                        .text_color(c(theme::MUTED))
                        .cursor_pointer()
                        .hover(|style| {
                            style
                                .text_color(c(theme::TEXT))
                                .bg(c(theme::SURFACE_RAISED))
                        })
                        .child(format!("+ {tag}"))
                        // On mouse down, before the field loses focus and the suggestions go.
                        .on_mouse_down(
                            gpui_kit::MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                input.update(cx, |input, cx| input.set_value("", window, cx));
                                pick(this, name.clone(), true, cx);
                            }),
                        )
                        .into_any_element()
                })
                .collect()
        } else {
            Vec::new()
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .children(chips)
                    .child(
                        self.field_box(&field_key, &input, None, window, cx)
                            .w(px(180.))
                            .h(px(26.)),
                    ),
            )
            .when(!suggestions.is_empty(), |this| {
                this.child(div().flex().flex_wrap().gap(px(6.)).children(suggestions))
            })
            .into_any_element()
    }
}
