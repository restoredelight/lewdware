//! The pieces the stage and transition editors are built from: cards, toggle rows, tag lists and
//! media slots.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, App, Context, Focusable, FontWeight, SharedString, Window};
use shared::behaviour::MediaSlot;

use super::fields::Spec;
use super::TimelineView;
use crate::theme::{self, c};
use crate::ui;
use crate::views::media_slot::{slot_key, MediaSlotView, SlotEvent};

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
    // An empty description means none: no line under the title.
    let description: SharedString = description.into();
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
                    .children((!description.is_empty()).then(|| ui::hint(description))),
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

/// What a media slot shows around the file.
pub(super) struct SlotSpec {
    pub slot: MediaSlot,
    pub title: &'static str,
    pub description: Option<&'static str>,
    /// Shown when the slot is empty: what leaving it empty means.
    pub empty_note: SharedString,
}

impl TimelineView {
    /// One media slot: its title and description over the shared slot row (`MediaSlotView`),
    /// the same one Content's wallpaper and splash use.
    pub(super) fn media_slot(
        &mut self,
        spec: SlotSpec,
        current: Option<u64>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = slot_key(&spec.slot);
        let entity = match self.slots.get(&key) {
            Some(entity) => entity.clone(),
            None => {
                let editor = self.editor.clone();
                let slot = spec.slot.clone();
                let entity = cx.new(|cx| MediaSlotView::new(editor, slot.clone(), cx));
                let subscription = cx.subscribe(&entity, move |this, _, event: &SlotEvent, cx| {
                    let SlotEvent::Changed = event;
                    if matches!(slot, MediaSlot::StageAudio { .. }) {
                        this.audio_picker_stage = None;
                    }
                    cx.notify();
                });
                self._slot_events.push(subscription);
                self.slots.insert(key, entity.clone());
                entity
            }
        };
        entity.update(cx, |view, cx| {
            view.set(current, spec.empty_note.clone(), cx)
        });

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
            .child(entity)
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
                    .id(SharedString::from(format!("{key}-chip-{index}")))
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
                    .when(is_owned, |chip| {
                        chip.tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new("This stage's own tag")
                                .build(window, cx)
                        })
                    })
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
        let suggestions: Vec<AnyElement> = if focused {
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
                        .px_2()
                        .py(px(5.))
                        .rounded(theme::RADIUS_SM)
                        .text_size(px(12.))
                        .truncate()
                        .cursor_pointer()
                        .hover(|style| style.bg(c(theme::SURFACE)))
                        .child(tag.clone())
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
                        // The suggestions float under the field, over what follows it.
                        div()
                            .relative()
                            .child(
                                self.field_box(&field_key, &input, None, window, cx)
                                    .w(px(180.))
                                    .h(px(26.)),
                            )
                            .when(!suggestions.is_empty(), |this| {
                                this.child(ui::floating(
                                    px(30.),
                                    false,
                                    ui::popover_surface()
                                        .w(px(220.))
                                        .p_1()
                                        .flex()
                                        .flex_col()
                                        .children(suggestions),
                                ))
                            }),
                    ),
            )
            .into_any_element()
    }
}
