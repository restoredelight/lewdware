//! The cards a pool is listed in (`ContentList.svelte`): each headed by its position and a
//! Remove, with the entry's fields under it. And the empty state where the cards would be.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, FontWeight, SharedString, Window};
use pack_editor_core::pack::ops::content::{automatic_prompt_timeout, Entry, EntryKind};
use shared::behaviour::ContentGroup;

use super::fields::{format_seconds, Field};
use super::{ContentView, Removal, Tab};
use crate::theme::{self, c};
use crate::ui::{self, Button, Size, Variant};

fn capitalised(noun: &str) -> String {
    let mut chars = noun.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A field's label: 12px, semibold.
fn field_label(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(theme::TEXT))
        .child(text.into())
}

fn labelled(label: &'static str, field: AnyElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(5.))
        .child(field_label(label))
        .child(field)
}

impl ContentView {
    fn card(
        &self,
        heading: String,
        remove_id: SharedString,
        removal: Removal,
        fields: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        ui::card()
            .p_3()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family(theme::FONT_MONO)
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(theme::MUTED))
                            .child(heading),
                    )
                    .child(
                        Button::new(remove_id, "Remove")
                            .icon(gpui_kit::assets::IconName::Trash)
                            .variant(Variant::Destructive)
                            .size(Size::Compact)
                            .on_click(move |_, window, cx| {
                                let removal = removal.clone();
                                entity.update(cx, |this, cx| {
                                    this.request_removal(removal, window, cx)
                                })
                            }),
                    ),
            )
            .children(fields)
            .into_any_element()
    }

    pub(super) fn entry_card(
        &self,
        kind: EntryKind,
        index: usize,
        entry: &Entry,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = entry.id;
        let mut fields = Vec::new();
        match kind {
            EntryKind::Notification => {
                fields.push(
                    labelled(
                        "Title",
                        self.text_field(
                            Field::NotificationTitle(id),
                            entry.summary.as_deref().unwrap_or(""),
                            "Optional",
                            None,
                            cx,
                        ),
                    )
                    .into_any_element(),
                );
                fields.push(
                    labelled(
                        "Message",
                        self.text_field(Field::Text(kind, id), &entry.text, "", None, cx),
                    )
                    .into_any_element(),
                );
            }
            EntryKind::WebLink => fields.push(
                labelled(
                    "URL",
                    self.text_field(Field::Text(kind, id), &entry.text, "https://...", None, cx),
                )
                .into_any_element(),
            ),
            EntryKind::Caption | EntryKind::Prompt => {
                fields.push(self.text_field(Field::Text(kind, id), &entry.text, "Text", None, cx))
            }
        }
        fields.push(self.tag_field(Field::EntryTag(kind, id), &entry.tags, cx));
        if kind == EntryKind::Prompt {
            let description = match entry.timeout_seconds {
                None => format!(
                    "Automatic: {} seconds based on this prompt's length.",
                    automatic_prompt_timeout(&entry.text)
                ),
                Some(_) => {
                    "Clear this value to use the automatic limit based on prompt length.".into()
                }
            };
            let value = entry
                .timeout_seconds
                .map(format_seconds)
                .unwrap_or_default();
            fields.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(field_label("Time limit"))
                    .child(div().w(px(160.)).child(self.text_field(
                        Field::PromptTimeout(id),
                        &value,
                        "Automatic",
                        Some("s"),
                        cx,
                    )))
                    .child(ui::hint(description).text_size(px(11.)))
                    .into_any_element(),
            );
        }
        self.card(
            format!("{} {}", capitalised(kind.noun()), index + 1),
            format!("remove-{}-{id}", kind.noun()).into(),
            Removal::Entry(kind, id),
            fields,
            cx,
        )
    }

    pub(super) fn group_card(
        &self,
        index: usize,
        group: &ContentGroup,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = group.id.clone();
        let enabled = group.enabled_by_default;
        let toggle = cx.entity();
        let toggle_key = key.clone();
        let fields = vec![
            labelled(
                "Group name",
                self.text_field(
                    Field::GroupLabel(key.clone()),
                    &group.label,
                    "Label",
                    None,
                    cx,
                ),
            )
            .into_any_element(),
            labelled(
                "Description (optional)",
                self.text_field(
                    Field::GroupDescription(key.clone()),
                    group.description.as_deref().unwrap_or(""),
                    "Explain what this group contains",
                    None,
                    cx,
                ),
            )
            .into_any_element(),
            self.tag_field(Field::GroupTag(key.clone()), &group.tags, cx),
            div()
                .id(SharedString::from(format!("group-enabled-{key}")))
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .child(ui::checkbox(enabled))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(c(theme::TEXT))
                        .child("Enabled by default"),
                )
                .on_click(move |_, _, cx| {
                    let key = toggle_key.clone();
                    toggle.update(cx, |this, cx| {
                        this.editor.update(cx, |editor, cx| {
                            editor.apply(cx, |s| {
                                s.set_content_group_enabled_by_default(&key, !enabled)
                            })
                        });
                    })
                })
                .into_any_element(),
        ];
        self.card(
            format!("Group {}", index + 1),
            format!("remove-group-{key}").into(),
            Removal::Group(key),
            fields,
            cx,
        )
    }

    /// Where the cards would be: the spawned-window motif (an echo frame behind a panel), with
    /// the way to add the first one.
    pub(super) fn empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let (title, description, action): (String, &str, Option<String>) = match self.tab {
            Tab::Entries(kind) => {
                let plural = self.tab.title().to_lowercase();
                (
                    format!("No {plural} yet"),
                    match kind {
                        EntryKind::WebLink => {
                            "Add a link if this pack should be able to open a page in the user's browser."
                        }
                        EntryKind::Caption => "Add the first caption to make it available to this pack.",
                        EntryKind::Prompt => "Add the first prompt to make it available to this pack.",
                        EntryKind::Notification => {
                            "Add the first notification to make it available to this pack."
                        }
                    },
                    Some(format!("Add {}", kind.noun())),
                )
            }
            _ => (
                "No content groups yet".into(),
                "Create a group when you want people to opt in or out of related tagged content.",
                None,
            ),
        };
        let tab = self.tab;
        div()
            .pt_6()
            .flex()
            .justify_center()
            .child(
                div()
                    .relative()
                    .w(px(360.))
                    .child(
                        div()
                            .absolute()
                            .top(px(-8.))
                            .left(px(-8.))
                            .right(px(8.))
                            .bottom(px(8.))
                            .rounded(theme::RADIUS_MD)
                            .border_1()
                            .border_color(c(theme::BORDER)),
                    )
                    .child(
                        div()
                            .relative()
                            .p_6()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .rounded(theme::RADIUS_MD)
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .bg(c(theme::SURFACE))
                            .child(ui::section_title(title))
                            .child(
                                ui::hint(description.to_string())
                                    .text_center()
                                    .text_size(px(12.)),
                            )
                            .children(action.map(|label| {
                                div().pt_2().child(
                                    Button::new("content-empty-add", label)
                                        .variant(Variant::Primary)
                                        .size(Size::Compact)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Tab::Entries(kind) = tab {
                                                this.add_entry(kind, window, cx)
                                            }
                                        })),
                                )
                            })),
                    ),
            )
            .into_any_element()
    }
}
