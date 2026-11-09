//! Text and number fields that commit on Enter or when they lose focus, as
//! `design/editor-model.md` section 5 has it, and cancel on Escape.
//!
//! A field is created the first time it's rendered and kept by key while it stays on screen. It
//! always shows what the pack holds, except while it's focused: then it shows what's being typed,
//! until the edit commits (and the field takes the stored value back, which may be the canonical
//! form of what was typed, or the old value if the edit was refused) or is cancelled.

use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, Context, Entity, Focusable, SharedString, Subscription, Window};

use super::TimelineView;
use crate::theme::{self, c};
use crate::ui;

type Commit = Rc<dyn Fn(&mut TimelineView, &str, &mut Context<TimelineView>)>;

pub(super) struct Field {
    input: Entity<InputState>,
    /// The stored value as text, as of the last render.
    shown: String,
    /// Take the stored value back on the next render even if focused: an edit just committed.
    resync: bool,
    /// Rendered this frame. Fields that weren't are dropped at the end of it.
    used: bool,
    /// Whether leaving the field commits it, or only Enter does.
    on_blur: bool,
    commit: Commit,
    _events: Subscription,
}

/// What a field shows and how it reads back.
pub(super) struct Spec {
    pub key: SharedString,
    pub current: String,
    pub placeholder: &'static str,
    pub on_blur: bool,
}

impl Spec {
    pub fn new(key: impl Into<SharedString>, current: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            current: current.into(),
            placeholder: "",
            on_blur: true,
        }
    }

    pub fn placeholder(mut self, placeholder: &'static str) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Only Enter commits: for an "add" field next to suggestions, where clicking a suggestion
    /// moves focus out of the field and mustn't add what was half-typed.
    pub fn enter_only(mut self) -> Self {
        self.on_blur = false;
        self
    }
}

/// A number the way a field shows it: `300`, `0.5`.
pub(super) fn number_text(value: f64) -> String {
    format!("{value}")
}

/// Reads a number field: `None` for blank, `Err` for anything that isn't a finite number.
pub(super) fn parse_number(text: &str) -> Result<Option<f64>, ()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(Some(value)),
        _ => Err(()),
    }
}

impl TimelineView {
    pub(super) fn begin_fields(&mut self) {
        for field in self.fields.values_mut() {
            field.used = false;
        }
    }

    pub(super) fn end_fields(&mut self) {
        self.fields.retain(|_, field| field.used);
    }

    /// The input for `spec`, made on first use. `commit` runs with the typed text when it differs
    /// from the stored value.
    pub(super) fn field(
        &mut self,
        spec: Spec,
        commit: impl Fn(&mut Self, &str, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let Spec {
            key,
            current,
            placeholder,
            on_blur,
        } = spec;
        if !self.fields.contains_key(&key) {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(current.clone())
            });
            let events_key = key.clone();
            let _events =
                cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, _, cx| {
                    match event {
                        InputEvent::PressEnter { .. } => this.commit_field(&events_key, cx),
                        InputEvent::Blur => {
                            if this.fields.get(&events_key).is_some_and(|f| f.on_blur) {
                                this.commit_field(&events_key, cx);
                            }
                        }
                        // Suggestions and the like follow what's typed.
                        InputEvent::Change => cx.notify(),
                        InputEvent::Focus => {}
                    }
                });
            self.fields.insert(
                key.clone(),
                Field {
                    input,
                    shown: current.clone(),
                    resync: false,
                    used: true,
                    on_blur,
                    commit: Rc::new(commit),
                    _events,
                },
            );
        } else if let Some(field) = self.fields.get_mut(&key) {
            field.commit = Rc::new(commit);
            field.on_blur = on_blur;
        }

        let field = self.fields.get_mut(&key).expect("inserted above");
        field.used = true;
        let input = field.input.clone();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        if (field.resync || !focused) && input.read(cx).value().as_ref() != current {
            input.update(cx, |input, cx| input.set_value(current.clone(), window, cx));
        }
        field.resync = false;
        field.shown = current;
        input
    }

    fn commit_field(&mut self, key: &SharedString, cx: &mut Context<Self>) {
        let Some(field) = self.fields.get_mut(key) else {
            return;
        };
        let text = field.input.read(cx).value().to_string();
        field.resync = true;
        let changed = text.trim() != field.shown.trim();
        let commit = field.commit.clone();
        if changed {
            commit(self, &text, cx);
        }
        cx.notify();
    }

    /// Escape: put the stored value back and leave the field, which then has nothing to commit.
    pub(super) fn cancel_field(
        &mut self,
        key: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.fields.get(key) else {
            return;
        };
        let (input, shown) = (field.input.clone(), field.shown.clone());
        input.update(cx, |input, cx| input.set_value(shown, window, cx));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Commits whichever field has focus, before the view switches away from it.
    pub(super) fn commit_focused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .fields
            .iter()
            .find(|(_, field)| field.input.read(cx).focus_handle(cx).is_focused(window))
            .map(|(key, field)| (key.clone(), field.on_blur));
        if let Some((key, on_blur)) = focused {
            if on_blur {
                self.commit_field(&key, cx);
            }
            window.focus(&self.focus, cx);
        }
    }

    /// The bordered box an input sits in, with Escape wired to cancel. `suffix` is a unit.
    pub(super) fn field_box(
        &self,
        key: &SharedString,
        input: &Entity<InputState>,
        suffix: Option<&'static str>,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui_kit::Div {
        self.field_box_sized(key, input, suffix, px(13.), window, cx)
    }

    /// [`Self::field_box`] with another text size: the stage name is a title.
    pub(super) fn field_box_sized(
        &self,
        key: &SharedString,
        input: &Entity<InputState>,
        suffix: Option<&'static str>,
        text_size: gpui_kit::Pixels,
        window: &Window,
        cx: &Context<Self>,
    ) -> gpui_kit::Div {
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        let view = cx.entity();
        let key = key.clone();
        div()
            .h(theme::CONTROL_COMPACT)
            .min_w_0()
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(if focused { theme::FOCUS } else { theme::BORDER }))
            .bg(c(theme::BG))
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    view.update(cx, |this, cx| this.cancel_field(&key, window, cx));
                }
            })
            .child(
                Input::new(input)
                    .appearance(false)
                    .text_size(text_size)
                    .flex_1(),
            )
            .children(suffix.map(ui::readout))
    }

    /// A labelled text field. Returns the field's box under its label.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn text_field(
        &mut self,
        label: &'static str,
        spec: Spec,
        commit: impl Fn(&mut Self, &str, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let key = spec.key.clone();
        let input = self.field(spec, commit, window, cx);
        labelled(label, None, self.field_box(&key, &input, None, window, cx))
    }

    /// A labelled number field. `apply` gets the parsed value (`None` for blank); text that isn't a
    /// number is refused, and the field shows the stored value again.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn number_field(
        &mut self,
        label: &'static str,
        description: Option<SharedString>,
        spec: Spec,
        suffix: Option<&'static str>,
        apply: impl Fn(&mut Self, Option<f64>, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let key = spec.key.clone();
        let input = self.field(
            spec,
            move |this, text, cx| {
                if let Ok(value) = parse_number(text) {
                    apply(this, value, cx);
                }
            },
            window,
            cx,
        );
        labelled(
            label,
            description,
            self.field_box(&key, &input, suffix, window, cx),
        )
    }
}

/// A control under a small label, with an optional note under it.
pub(super) fn labelled(
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    control: impl IntoElement,
) -> gpui_kit::Div {
    div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(c(theme::TEXT))
                .child(label.into()),
        )
        .child(control)
        .children(description.map(|description| ui::hint(description).text_size(px(11.))))
}
