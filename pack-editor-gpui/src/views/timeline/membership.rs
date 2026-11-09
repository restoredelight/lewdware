//! "Appears in": which timeline stages one file is in, and putting it in or taking it out of each.
//! Ported from `StageMembership.svelte`. Meant for the library's inspector:
//!
//! ```ignore
//! let membership = views::timeline::stage_membership(editor.clone(), file_id, cx);
//! // in render: .child(membership.clone())
//! // when the selection changes: membership.update(cx, |m, cx| m.set_media(new_id, cx));
//! ```
//!
//! It renders nothing when the pack has no stages. It redraws itself on timeline, tag and media
//! changes, so the owner only has to keep it pointed at the right file.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, App, Context, Entity, FocusHandle, SharedString, Subscription, Window};
use pack_editor_core::changes::Scope;
use pack_editor_core::pack::ops::timeline::MembershipOutcome;
use shared::behaviour::editor as queries;

use crate::editor::{Editor, EditorEvent};
use crate::theme::{self, c};
use crate::ui::{self, DialogAction, Variant};

/// A stage membership strip for `media`. See the module docs for mounting it.
pub fn stage_membership(
    editor: Entity<Editor>,
    media: u64,
    cx: &mut App,
) -> Entity<StageMembership> {
    cx.new(|cx| StageMembership::new(editor, media, cx))
}

/// Joining would take author tags off the file; waiting on the author's say-so.
struct Pending {
    stage: String,
    label: String,
    tags: Vec<String>,
}

pub struct StageMembership {
    editor: Entity<Editor>,
    media: u64,
    label: SharedString,
    pending: Option<Pending>,
    dialog_focus: FocusHandle,
    _changes: Subscription,
}

impl StageMembership {
    fn new(editor: Entity<Editor>, media: u64, cx: &mut Context<Self>) -> Self {
        let _changes = cx.subscribe(&editor, |_, _, event: &EditorEvent, cx| match event {
            EditorEvent::Changed(changes) => {
                if changes
                    .scopes
                    .intersects(Scope::Timeline | Scope::Tags | Scope::Media)
                {
                    cx.notify();
                }
            }
        });
        Self {
            editor,
            media,
            label: "Appears in".into(),
            pending: None,
            dialog_focus: cx.focus_handle(),
            _changes,
        }
    }

    /// Points the strip at another file.
    pub fn set_media(&mut self, media: u64, cx: &mut Context<Self>) {
        if self.media != media {
            self.media = media;
            self.pending = None;
            cx.notify();
        }
    }

    fn change(
        &mut self,
        stage: String,
        label: String,
        member: bool,
        approved: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let media = self.media;
        let outcome = self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| {
                session.set_stage_membership(media, &stage, member, &approved)
            })
        });
        if let Some(MembershipOutcome::ConfirmationRequired { tags }) = outcome {
            self.pending = Some(Pending { stage, label, tags });
            window.focus(&self.dialog_focus, cx);
        }
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        cx.notify();
    }
}

impl Render for StageMembership {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.editor.read(cx).session();
        let rows = session
            .read(|c| queries::stage_membership(c, self.media))
            .unwrap_or_default();
        if rows.is_empty() {
            return div().into_any_element();
        }
        let file_name = session
            .media()
            .get(self.media)
            .map(|file| file.file_name.clone())
            .unwrap_or_else(|| "this file".to_string());
        let members = rows.iter().filter(|row| row.member).count();
        let total = rows.len();

        let chips = rows.into_iter().enumerate().map(|(index, row)| {
            let (stage, label, member) = (row.id, row.label, row.member);
            let shown = label.clone();
            div()
                .id(("membership-stage", index))
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .rounded(theme::RADIUS_SM)
                .border_1()
                .text_size(px(12.))
                .cursor_pointer()
                .map(|this| {
                    // Selected: raised surface with a carmine edge, never a fill.
                    if member {
                        this.bg(c(theme::SURFACE_RAISED))
                            .border_color(c(theme::BORDER_STRONG))
                            .border_b_2()
                            .text_color(c(theme::TEXT))
                    } else {
                        this.bg(c(theme::BG))
                            .border_color(c(theme::BORDER))
                            .text_color(c(theme::MUTED))
                            .hover(|style| style.text_color(c(theme::TEXT)))
                    }
                })
                .when(member, |this| this.border_color(c(theme::ACCENT_HOVER)))
                .child(shown)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.change(
                        stage.clone(),
                        label.clone(),
                        !member,
                        Vec::new(),
                        window,
                        cx,
                    )
                }))
        });

        let dialog = self.pending.as_ref().map(|pending| {
            let view = cx.entity();
            let cancel = view.clone();
            let close = view.clone();
            let (stage, label, tags) = (
                pending.stage.clone(),
                pending.label.clone(),
                pending.tags.clone(),
            );
            ui::dialog(
                format!("Add \"{file_name}\" to {}?", pending.label),
                format!(
                    "Tags being removed: {}",
                    pending
                        .tags
                        .iter()
                        .map(|tag| format!("\"{tag}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                vec![
                    DialogAction::new("Cancel", Variant::Secondary, move |_, _, cx| {
                        cancel.update(cx, |this, cx| this.dismiss(cx))
                    }),
                    DialogAction::new("Add anyway", Variant::Primary, move |_, window, cx| {
                        let (stage, label, tags) = (stage.clone(), label.clone(), tags.clone());
                        view.update(cx, |this, cx| {
                            this.pending = None;
                            this.change(stage, label, true, tags, window, cx)
                        })
                    }),
                ],
                move |_, cx| close.update(cx, |this, cx| this.dismiss(cx)),
                &self.dialog_focus,
                window,
            )
        });

        div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap_2()
                    .child(ui::hint(self.label.clone()).text_size(px(11.)))
                    .child(ui::readout(format!("{members} of {total}"))),
            )
            .child(div().flex().flex_wrap().gap(px(6.)).children(chips))
            .children(dialog)
            .into_any_element()
    }
}
