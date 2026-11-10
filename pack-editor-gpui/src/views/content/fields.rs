//! Editing one field at a time, the way `views/library/tags.rs` renames a tag.
//!
//! A field shows its stored value until it's clicked; then it becomes an input, and the input is
//! the only one in the view. Text commits on Enter or leaving the field (blur, or a click
//! anywhere outside it), and Escape puts it back as it was (`design/editor-model.md`,
//! section 5). Multi-line text takes Shift+Enter for a new line.
//!
//! A tag field is the same input with suggestions under it: Enter adds the highlighted tag (or
//! creates the typed one) and stays open for the next; leaving it adds nothing.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::*;
use gpui_kit::Focusable as _;
use gpui_kit::{
    div, px, AnyElement, App, Context, ElementId, Entity, SharedString, Subscription, Window,
};
use pack_editor_core::pack::ops::content::EntryKind;

use super::ContentView;
use crate::theme::{self, c};

/// One editable field in the view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Field {
    /// A caption's, prompt's or notification's text, or a web link's URL.
    Text(EntryKind, i64),
    NotificationTitle(i64),
    PromptTimeout(i64),
    /// The "add tag" input under an entry.
    EntryTag(EntryKind, i64),
    GroupLabel(String),
    GroupDescription(String),
    GroupTag(String),
}

impl Field {
    fn is_tag(&self) -> bool {
        matches!(self, Field::EntryTag(..) | Field::GroupTag(_))
    }

    fn multiline(&self) -> bool {
        matches!(
            self,
            Field::Text(
                EntryKind::Caption | EntryKind::Prompt | EntryKind::Notification,
                _
            )
        )
    }

    pub(super) fn element_id(&self) -> ElementId {
        SharedString::from(format!("field-{self:?}")).into()
    }
}

enum FieldInput {
    Line(Entity<InputState>),
    Multi(Entity<TextareaState>),
}

impl FieldInput {
    fn is_focused(&self, window: &Window, cx: &App) -> bool {
        match self {
            FieldInput::Line(input) => input.read(cx).focus_handle(cx).is_focused(window),
            FieldInput::Multi(input) => input.read(cx).focus_handle(cx).is_focused(window),
        }
    }

    fn value(&self, cx: &App) -> String {
        match self {
            FieldInput::Line(input) => input.read(cx).value().to_string(),
            FieldInput::Multi(input) => input.read(cx).value().to_string(),
        }
    }
}

pub(super) struct Editing {
    pub(super) field: Field,
    input: FieldInput,
    /// A tag field's highlighted suggestion, moved with the arrow keys. `None` is "whatever Enter
    /// would pick by default" (see [`TagOptions`]).
    highlighted: Option<usize>,
    _events: Subscription,
}

/// What a tag field offers for what's been typed: the "create" row when the text names a tag the
/// pack hasn't got, then the tags that match. Ported from `shared-ui/TagInput.svelte`.
pub(super) struct TagOptions {
    /// `(tag, is_create)`.
    pub(super) rows: Vec<(String, bool)>,
    /// What Enter picks when the arrow keys haven't said: the create row, else the tag the text
    /// names exactly, else nothing. Never an arbitrary match, so `feet` can be created where
    /// `feet-pov` exists.
    pub(super) default: Option<usize>,
}

impl TagOptions {
    pub(super) fn new(query: &str, all: &[String], current: &[String]) -> Self {
        let query = query.trim();
        let lower = query.to_lowercase();
        let taken = |tag: &str| {
            current
                .iter()
                .any(|t| t.to_lowercase() == tag.to_lowercase())
        };
        let matches: Vec<&String> = all
            .iter()
            .filter(|tag| !taken(tag) && (lower.is_empty() || tag.to_lowercase().contains(&lower)))
            .collect();
        let can_create = !query.is_empty()
            && !matches.iter().any(|tag| tag.to_lowercase() == lower)
            && !taken(query);
        let mut rows = Vec::new();
        if can_create {
            rows.push((query.to_string(), true));
        }
        rows.extend(matches.into_iter().map(|tag| (tag.clone(), false)));
        let default = if can_create {
            Some(0)
        } else if lower.is_empty() {
            None
        } else {
            rows.iter().position(|(tag, _)| tag.to_lowercase() == lower)
        };
        Self { rows, default }
    }
}

impl ContentView {
    /// The stored value a field starts from, or `None` if what it edits has gone.
    fn stored(&self, field: &Field) -> Option<String> {
        Some(match field {
            Field::Text(kind, id) => self.data.entry(*kind, *id)?.text.clone(),
            Field::NotificationTitle(id) => self
                .data
                .entry(EntryKind::Notification, *id)?
                .summary
                .clone()
                .unwrap_or_default(),
            Field::PromptTimeout(id) => self
                .data
                .entry(EntryKind::Prompt, *id)?
                .timeout_seconds
                .map(format_seconds)
                .unwrap_or_default(),
            Field::GroupLabel(key) => self.data.group(key)?.label.clone(),
            Field::GroupDescription(key) => self
                .data
                .group(key)?
                .description
                .clone()
                .unwrap_or_default(),
            Field::EntryTag(..) | Field::GroupTag(_) => String::new(),
        })
    }

    /// The tags already on whatever a tag field edits.
    pub(super) fn current_tags(&self, field: &Field) -> Vec<String> {
        match field {
            Field::EntryTag(kind, id) => self
                .data
                .entry(*kind, *id)
                .map(|entry| entry.tags.clone())
                .unwrap_or_default(),
            Field::GroupTag(key) => self
                .data
                .group(key)
                .map(|group| group.tags.clone())
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub(super) fn start_editing(
        &mut self,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editing
            .as_ref()
            .is_some_and(|editing| editing.field == field)
        {
            return;
        }
        self.commit_editing(window, cx);
        let Some(value) = self.stored(&field) else {
            return;
        };
        let placeholder = match &field {
            Field::Text(EntryKind::WebLink, _) => "https://...",
            Field::Text(..) => "Text",
            Field::NotificationTitle(_) => "Optional",
            Field::PromptTimeout(_) => "Automatic",
            Field::GroupLabel(_) => "Label",
            Field::GroupDescription(_) => "Explain what this group contains",
            Field::EntryTag(..) | Field::GroupTag(_) => "Add tag...",
        };
        let select_all = matches!(field, Field::GroupLabel(_));
        let input = if field.multiline() {
            let input = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(2, 12)
                    .submit_on_enter(true)
                    .placeholder(placeholder)
                    .default_value(value)
            });
            input.update(cx, |input, cx| input.focus(window, cx));
            FieldInput::Multi(input)
        } else {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            });
            input.update(cx, |input, cx| {
                input.focus(window, cx);
                if select_all {
                    input.select_all(window, cx);
                }
            });
            FieldInput::Line(input)
        };
        let tag = field.is_tag();
        let on_event = move |this: &mut Self,
                             event: &InputEvent,
                             window: &mut Window,
                             cx: &mut Context<Self>| {
            match event {
                InputEvent::PressEnter { .. } if tag => this.add_highlighted_tag(window, cx),
                InputEvent::PressEnter { .. } => this.commit_editing(window, cx),
                // Leaving a tag field adds nothing: only Enter or a click on a suggestion does.
                InputEvent::Blur if tag => this.cancel_editing(window, cx),
                InputEvent::Blur => this.commit_editing(window, cx),
                InputEvent::Change if tag => {
                    if let Some(editing) = this.editing.as_mut() {
                        editing.highlighted = None;
                    }
                    cx.notify();
                }
                _ => {}
            }
        };
        let _events = match &input {
            FieldInput::Line(state) => {
                cx.subscribe_in(state, window, move |this, _, event, window, cx| {
                    on_event(this, event, window, cx)
                })
            }
            FieldInput::Multi(state) => {
                cx.subscribe_in(state, window, move |this, _, event, window, cx| {
                    on_event(this, event, window, cx)
                })
            }
        };
        self.editing = Some(Editing {
            field,
            input,
            highlighted: None,
            _events,
        });
        cx.notify();
    }

    /// Writes the field being edited, if it says something different from what's stored. Taken
    /// first, so the blur that follows an Enter finds nothing left to commit.
    /// Closes the open field. If its input had focus, the view takes it, so the editor's
    /// shortcuts (undo, save) keep working: a dropped input leaves nothing focused otherwise.
    fn end_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Editing> {
        let editing = self.editing.take()?;
        if editing.input.is_focused(window, cx) {
            self.focus.focus(window, cx);
        }
        cx.notify();
        Some(editing)
    }

    pub(super) fn commit_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.end_editing(window, cx) else {
            return;
        };
        cx.notify();
        if editing.field.is_tag() {
            return;
        }
        let value = editing.input.value(cx);
        if Some(&value) == self.stored(&editing.field).as_ref() {
            return;
        }
        let field = editing.field;
        if let Field::PromptTimeout(id) = field {
            let text = value.trim().trim_end_matches('s').trim();
            let seconds = if text.is_empty() {
                None
            } else {
                match text.parse::<f64>() {
                    Ok(seconds) => Some(seconds),
                    Err(_) => {
                        self.editor.update(cx, |editor, cx| {
                            editor.error =
                                Some("A time limit has to be a number of seconds".into());
                            cx.notify();
                        });
                        return;
                    }
                }
            };
            self.editor.update(cx, |editor, cx| {
                editor.apply(cx, |s| s.set_prompt_timeout(id, seconds))
            });
            return;
        }
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |s| match &field {
                Field::Text(kind, id) => s.set_entry_text(*kind, *id, &value),
                Field::NotificationTitle(id) => s.set_notification_summary(*id, Some(&value)),
                Field::GroupLabel(key) => s.set_content_group_label(key, &value),
                Field::GroupDescription(key) => s.set_content_group_description(key, Some(&value)),
                Field::PromptTimeout(_) | Field::EntryTag(..) | Field::GroupTag(_) => {
                    unreachable!("handled above")
                }
            })
        });
    }

    pub(super) fn cancel_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.end_editing(window, cx);
    }

    fn tag_options(&self, cx: &App) -> Option<TagOptions> {
        let editing = self.editing.as_ref()?;
        if !editing.field.is_tag() {
            return None;
        }
        let query = editing.input.value(cx);
        Some(TagOptions::new(
            &query,
            &self.data.tags,
            &self.current_tags(&editing.field),
        ))
    }

    fn move_highlight(&mut self, step: isize, cx: &mut Context<Self>) {
        let Some(options) = self.tag_options(cx) else {
            return;
        };
        let count = options.rows.len() as isize;
        let Some(editing) = self.editing.as_mut() else {
            return;
        };
        if count == 0 {
            return;
        }
        let current = editing.highlighted.or(options.default).map(|i| i as isize);
        let next = match current {
            // From nothing highlighted, down enters at the top and up at the bottom.
            None if step > 0 => 0,
            None => count - 1,
            Some(index) => (index + step).rem_euclid(count),
        };
        editing.highlighted = Some(next as usize);
        cx.notify();
    }

    fn add_highlighted_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(options) = self.tag_options(cx) else {
            return;
        };
        let highlighted = self.editing.as_ref().and_then(|e| e.highlighted);
        let Some((tag, _)) = highlighted
            .or(options.default)
            .and_then(|index| options.rows.get(index))
        else {
            return;
        };
        let tag = tag.clone();
        self.add_tag(tag, window, cx);
    }

    /// Adds `tag` to whatever the open tag field edits, and empties the field for the next.
    fn add_tag(&mut self, tag: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.editing.as_mut() else {
            return;
        };
        let field = editing.field.clone();
        editing.highlighted = None;
        if let FieldInput::Line(input) = &editing.input {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |s| match &field {
                Field::EntryTag(kind, id) => s.add_entry_tag(*kind, *id, &tag),
                Field::GroupTag(key) => s.add_content_group_tag(key, &tag),
                _ => unreachable!("only tag fields add tags"),
            })
        });
        cx.notify();
    }

    pub(super) fn remove_tag(&mut self, field: &Field, tag: &str, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |s| match field {
                Field::EntryTag(kind, id) => s.remove_entry_tag(*kind, *id, tag),
                Field::GroupTag(key) => s.remove_content_group_tag(key, tag),
                _ => unreachable!("only tag fields remove tags"),
            })
        });
    }

    // ── Rendering ───────────────────────────────────────────────────────────────────────────

    /// A field showing `value`, or its input while it's being edited.
    pub(super) fn text_field(
        &self,
        field: Field,
        value: &str,
        placeholder: &'static str,
        suffix: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let multiline = field.multiline();
        let min_height = if multiline {
            px(52.)
        } else {
            theme::CONTROL_COMPACT
        };
        let editing = self
            .editing
            .as_ref()
            .filter(|editing| editing.field == field);
        // The display matches the input's own padding (gpui-component pads a single-line input
        // 10px across, a textarea 10px across and 8px down, even without its frame), so the text
        // doesn't move when the field opens for editing.
        let shell = div()
            .id(field.element_id())
            .w_full()
            .min_h(min_height)
            .px(INPUT_PX)
            .flex()
            .gap_2()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .bg(c(theme::BG))
            .text_size(px(12.))
            .when(!multiline, |this| this.items_center())
            .when(multiline, |this| this.items_start().py(px(8.)));
        let suffix = suffix.map(ui_suffix);
        match editing {
            Some(editing) => {
                let entity = cx.entity();
                let commit = cx.entity();
                let input = match &editing.input {
                    FieldInput::Line(input) => Input::new(input)
                        .appearance(false)
                        .text_size(px(12.))
                        .flex_1()
                        .into_any_element(),
                    FieldInput::Multi(input) => Textarea::new(input)
                        .appearance(false)
                        .text_size(px(12.))
                        .flex_1()
                        .into_any_element(),
                };
                shell
                    .border_color(c(theme::FOCUS))
                    // The input brings its own padding; a suffix keeps the right edge's.
                    .when(multiline, |this| this.p_0())
                    .when(!multiline, |this| this.pl_0())
                    .capture_key_down(move |event, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            entity.update(cx, |this, cx| this.cancel_editing(window, cx));
                        }
                    })
                    .on_mouse_down_out(move |_, window, cx| {
                        commit.update(cx, |this, cx| this.commit_editing(window, cx));
                    })
                    .child(input)
                    .children(suffix)
                    .into_any_element()
            }
            None => {
                let entity = cx.entity();
                let empty = value.is_empty();
                shell
                    .border_color(c(theme::BORDER_STRONG))
                    .cursor_text()
                    .hover(|style| style.border_color(c(theme::MUTED)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            // The input's right padding, so a suffix stays put too.
                            .when(!multiline, |this| this.pr(INPUT_PX).truncate())
                            .text_color(c(if empty { theme::MUTED } else { theme::TEXT }))
                            .child(SharedString::from(if empty {
                                placeholder.to_string()
                            } else {
                                value.to_string()
                            })),
                    )
                    .children(suffix)
                    .on_click(move |_, window, cx| {
                        let field = field.clone();
                        entity.update(cx, |this, cx| this.start_editing(field, window, cx));
                    })
                    .into_any_element()
            }
        }
    }

    /// The chips of `tags` and the field that adds one, with suggestions while it's open.
    pub(super) fn tag_field(
        &self,
        field: Field,
        tags: &[String],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let chips = tags.iter().enumerate().map(|(index, tag)| {
            let entity = cx.entity();
            let field = field.clone();
            let remove_tag = tag.clone();
            div()
                .h(px(26.))
                .max_w_full()
                .pl(px(10.))
                .pr(px(3.))
                .flex()
                .items_center()
                .gap_1()
                .rounded_full()
                .border_1()
                .border_color(c(theme::BORDER))
                .bg(c(theme::SURFACE_RAISED))
                .text_size(px(12.))
                .text_color(c(theme::TEXT))
                .child(div().min_w_0().truncate().child(tag.clone()))
                .child(
                    div()
                        .id(SharedString::from(format!("{:?}-remove-{index}", field)))
                        .size(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS_SM)
                        .text_color(c(theme::MUTED))
                        .cursor_pointer()
                        .hover(|style| style.text_color(c(theme::DANGER)))
                        .child(gpui_kit::assets::IconName::Close)
                        .text_size(px(11.))
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| this.remove_tag(&field, &remove_tag, cx))
                        }),
                )
        });

        let editing = self
            .editing
            .as_ref()
            .filter(|editing| editing.field == field);
        let entry = match editing {
            Some(editing) => {
                let FieldInput::Line(input) = &editing.input else {
                    unreachable!("tag fields are single-line")
                };
                let options = self.tag_options(cx).unwrap_or(TagOptions {
                    rows: Vec::new(),
                    default: None,
                });
                let active = editing.highlighted.or(options.default);
                let keys = cx.entity();
                let close = cx.entity();
                let menu_bounds = self.tag_menu_bounds.clone();
                menu_bounds.set(None);
                let measure = menu_bounds.clone();
                let menu = (!options.rows.is_empty()).then(|| {
                    crate::ui::floating(
                        theme::CONTROL_COMPACT - px(2.),
                        false,
                        div()
                            .id("tag-suggestions")
                            .occlude()
                            .min_w(px(200.))
                            .max_w(px(320.))
                            .max_h(px(224.))
                            .overflow_y_scrollbar()
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
                            .child(
                                gpui_kit::canvas(
                                    move |bounds, _, _| measure.set(Some(bounds)),
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            )
                            .children(options.rows.into_iter().take(100).enumerate().map(
                                |(index, (tag, create))| {
                                    let entity = cx.entity();
                                    let selected = Some(index) == active;
                                    div()
                                        .id(index)
                                        .px_2()
                                        .py(px(5.))
                                        .flex()
                                        .gap_1()
                                        .rounded(theme::RADIUS_SM)
                                        .text_size(px(12.))
                                        .border_l_2()
                                        .border_color(if selected {
                                            c(theme::ACCENT_HOVER)
                                        } else {
                                            gpui_kit::transparent_black()
                                        })
                                        .when(selected, |this| this.bg(c(theme::SURFACE)))
                                        .cursor_pointer()
                                        .hover(|style| style.bg(c(theme::SURFACE)))
                                        .when(create, |this| {
                                            this.child(
                                                div().text_color(c(theme::MUTED)).child("Create"),
                                            )
                                        })
                                        .child(div().truncate().child(tag.clone()))
                                        // On mouse down, so the field is still open to add to.
                                        .on_mouse_down(
                                            gpui_kit::MouseButton::Left,
                                            move |_, window, cx| {
                                                cx.stop_propagation();
                                                let tag = tag.clone();
                                                entity.update(cx, |this, cx| {
                                                    this.add_tag(tag, window, cx)
                                                });
                                            },
                                        )
                                },
                            )),
                    )
                });
                div()
                    .id(field.element_id())
                    .relative()
                    .w(px(180.))
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(theme::FOCUS))
                    .bg(c(theme::BG))
                    .capture_key_down(
                        move |event, window, cx| match event.keystroke.key.as_str() {
                            "escape" | "tab" => {
                                cx.stop_propagation();
                                keys.update(cx, |this, cx| this.cancel_editing(window, cx));
                            }
                            "down" | "up" => {
                                cx.stop_propagation();
                                let step = if event.keystroke.key == "down" { 1 } else { -1 };
                                keys.update(cx, |this, cx| this.move_highlight(step, cx));
                            }
                            _ => {}
                        },
                    )
                    .on_mouse_down_out(move |event, window, cx| {
                        // The suggestions float outside the field; a press on one adds it.
                        if menu_bounds
                            .get()
                            .is_some_and(|bounds| bounds.contains(&event.position))
                        {
                            return;
                        }
                        close.update(cx, |this, cx| this.cancel_editing(window, cx));
                    })
                    .child(
                        Input::new(input)
                            .appearance(false)
                            .text_size(px(12.))
                            .flex_1(),
                    )
                    .children(menu)
                    .into_any_element()
            }
            None => {
                let entity = cx.entity();
                let field = field.clone();
                div()
                    .id(field.element_id())
                    .h(px(26.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(gpui_kit::transparent_black())
                    .gap_1()
                    .text_size(px(12.))
                    .text_color(c(theme::MUTED))
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .border_color(c(theme::BORDER_STRONG))
                            .text_color(c(theme::TEXT))
                    })
                    .child(crate::icons::icon(IconName::Plus).size(px(12.)))
                    .child("Add tag")
                    .on_click(move |_, window, cx| {
                        let field = field.clone();
                        entity.update(cx, |this, cx| this.start_editing(field, window, cx));
                    })
                    .into_any_element()
            }
        };

        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .min_h(px(28.))
            .children(chips)
            .child(entry)
            .into_any_element()
    }
}

/// gpui-component's horizontal padding inside an input (its default size), which the display
/// state of a field copies.
const INPUT_PX: gpui_kit::Pixels = px(10.);

/// A unit drawn inside a field's right edge, in the readout face.
fn ui_suffix(suffix: &'static str) -> impl IntoElement {
    div()
        .flex_none()
        .font_family(theme::FONT_MONO)
        .text_size(px(11.))
        .text_color(c(theme::MUTED))
        .child(suffix)
}

/// Seconds as an author would type them: `20`, not `20.0`.
pub(super) fn format_seconds(seconds: f64) -> String {
    if seconds.fract() == 0. {
        format!("{seconds:.0}")
    } else {
        format!("{seconds}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn typing_a_new_tag_offers_to_create_it_first() {
        let options = TagOptions::new("feet", &tags(&["feet-pov", "hands"]), &[]);
        assert_eq!(
            options.rows,
            vec![("feet".to_string(), true), ("feet-pov".to_string(), false)]
        );
        assert_eq!(options.default, Some(0));
    }

    #[test]
    fn typing_an_existing_tag_picks_it_whatever_the_case() {
        let options = TagOptions::new("Feet", &tags(&["feet", "feet-pov"]), &[]);
        assert!(options.rows.iter().all(|(_, create)| !create));
        assert_eq!(options.default, Some(0));
    }

    #[test]
    fn tags_already_there_are_neither_offered_nor_created() {
        let options = TagOptions::new("feet", &tags(&["feet"]), &tags(&["feet"]));
        assert!(options.rows.is_empty());
        assert_eq!(options.default, None);
    }

    #[test]
    fn an_empty_query_offers_everything_and_picks_nothing() {
        let options = TagOptions::new(" ", &tags(&["a", "b"]), &[]);
        assert_eq!(options.rows.len(), 2);
        assert_eq!(options.default, None);
    }

    #[test]
    fn seconds_read_as_typed() {
        assert_eq!(format_seconds(20.), "20");
        assert_eq!(format_seconds(2.5), "2.5");
    }
}
