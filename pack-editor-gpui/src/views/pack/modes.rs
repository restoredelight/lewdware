//! The Modes view: the mode the config app preselects for this pack, and the custom modes the
//! pack bundles (adding a built `.lwmode`, removing one). Ported from
//! `pack-editor/src/lib/Modes.svelte`.

use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, Context, Entity, FocusHandle, FontWeight, SharedString, Subscription, Window,
};
use pack_editor_core::changes::Scope;
use pack_editor_core::pack::ops::lifecycle::{EmbeddedMode, MetadataField};
use shared::behaviour::editor as behaviour_editor;
use shared::pack::RecommendedMode;

use crate::editor::{Editor, EditorEvent};
use crate::runtime;
use crate::theme::{self, c};
use crate::ui::{self, Button, DialogAction, Select, SelectOption, Size, Variant};

pub struct ModesView {
    editor: Entity<Editor>,
    /// Read when the `Modes` scope changes rather than on every render: it reads every mode file.
    modes: Result<Vec<EmbeddedMode>, SharedString>,
    /// The file picker is open, or the picked file is being read.
    adding: bool,
    removing: Option<EmbeddedMode>,
    dialog_focus: FocusHandle,
    _changes: Subscription,
}

impl ModesView {
    pub fn new(editor: Entity<Editor>, cx: &mut Context<Self>) -> Self {
        let _changes = cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| match event {
            EditorEvent::Changed(changes) => {
                if changes.scopes.contains(Scope::Modes) {
                    this.reload(cx);
                }
                if changes
                    .scopes
                    .intersects(Scope::Modes | Scope::Metadata | Scope::Timeline)
                {
                    cx.notify();
                }
            }
        });
        let mut this = Self {
            editor,
            modes: Ok(Vec::new()),
            adding: false,
            removing: None,
            dialog_focus: cx.focus_handle(),
            _changes,
        };
        this.reload(cx);
        this
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.modes = self
            .editor
            .read(cx)
            .session()
            .modes()
            .map_err(|error| format!("{error:#}").into());
    }

    fn add_mode(&mut self, cx: &mut Context<Self>) {
        if self.adding {
            return;
        }
        self.adding = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let picked = runtime::run(async {
                rfd::AsyncFileDialog::new()
                    .set_title("Add custom mode")
                    .add_filter("Lewdware mode", &["lwmode"])
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            })
            .await;
            let bytes = match picked {
                Ok(Some(path)) => Some(runtime::blocking(move || std::fs::read(path)).await),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            };
            let _ = this.update(cx, |this, cx| {
                this.adding = false;
                let editor = this.editor.clone();
                match bytes {
                    Some(Ok(Ok(bytes))) => {
                        editor.update(cx, |editor, cx| {
                            editor.apply(cx, |session| session.add_mode(bytes));
                        });
                    }
                    Some(Ok(Err(error))) => editor.update(cx, |editor, cx| {
                        editor.report(format!("Couldn't read that mode: {error}"), cx)
                    }),
                    Some(Err(error)) => editor.update(cx, |editor, cx| {
                        editor.report(format!("Couldn't add a mode: {error:#}"), cx)
                    }),
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm_remove(&mut self, mode: EmbeddedMode, window: &mut Window, cx: &mut Context<Self>) {
        self.removing = Some(mode);
        window.focus(&self.dialog_focus, cx);
        cx.notify();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        let Some(mode) = self.removing.take() else {
            return;
        };
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| session.remove_mode(mode.id));
        });
        cx.notify();
    }

    fn set_recommendation(&mut self, value: Option<RecommendedMode>, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| {
                session.set_metadata(MetadataField::RecommendedMode(value))
            });
        });
    }

    fn recommendation(
        &self,
        modes: &[EmbeddedMode],
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let session = self.editor.read(cx).session();
        let current = session.metadata().recommended_mode;
        // The timeline mode ("Sequence") shows under the pack's own label for it when there is
        // one, as the config app shows it.
        let timeline = session.read(behaviour_editor::timeline).ok().flatten();
        let timeline_name = timeline
            .as_ref()
            .and_then(|view| view.timeline.label.clone())
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| "Sequence".to_string());
        let timeline_on = timeline.as_ref().is_some_and(|view| view.enabled);
        let automatic = format!(
            "Automatic ({})",
            if timeline_on {
                timeline_name.as_str()
            } else {
                "Sandbox"
            }
        );
        let mut options = vec![
            SelectOption::new(None, automatic),
            SelectOption::new(Some(RecommendedMode::Sandbox), "Sandbox"),
            SelectOption::new(Some(RecommendedMode::Experience), timeline_name),
        ];
        options.extend(modes.iter().map(|mode| {
            SelectOption::new(
                Some(RecommendedMode::Pack { id: mode.id }),
                mode.name.clone(),
            )
        }));
        let view = cx.entity();
        ui::card()
            .child(
                div()
                    .p(px(14.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_6()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(ui::label("Recommended mode"))
                            .child(ui::hint(
                                "The config app preselects this mode when someone opens the pack. They can still choose another mode.",
                            )),
                    )
                    .child(
                        div().w(px(240.)).flex_none().flex().child(
                            Select::new("recommended-mode", current, options).on_change(
                                move |value, _, cx| {
                                    view.update(cx, |this, cx| this.set_recommendation(value, cx))
                                },
                            ),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn mode_list(
        &self,
        modes: &[EmbeddedMode],
        recommended: Option<u64>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let rows = modes.iter().enumerate().map(|(index, mode)| {
            let details = [
                mode.author.clone(),
                mode.version
                    .as_ref()
                    .map(|version| format!("Version {version}")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            let details = if details.is_empty() {
                "No author or version provided".to_string()
            } else {
                details
            };
            let view = cx.entity();
            let target = mode.clone();
            div()
                .min_h(px(64.))
                .px(px(14.))
                .py_3()
                .flex()
                .items_center()
                .gap_3()
                .when(index > 0, |this| {
                    this.border_t_1().border_color(c(theme::BORDER))
                })
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
                                .text_size(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(mode.name.clone()),
                        )
                        .child(ui::hint(details).text_size(px(11.)).truncate())
                        .child(ui::readout(format!(
                            "{} · {}",
                            plural(mode.option_count, "option", "options"),
                            format_size(mode.size)
                        ))),
                )
                .when(recommended == Some(mode.id), |this| {
                    this.child(
                        div()
                            .flex_none()
                            .px(px(7.))
                            .py(px(3.))
                            .rounded_full()
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .bg(c(theme::SURFACE_RAISED))
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Recommended"),
                    )
                })
                .child(
                    Button::new(("remove-mode", index), "Remove")
                        .variant(Variant::Quiet)
                        .size(Size::Compact)
                        .on_click(move |_, window, cx| {
                            let mode = target.clone();
                            view.update(cx, |this, cx| this.confirm_remove(mode, window, cx))
                        }),
                )
        });
        ui::card().children(rows).into_any_element()
    }

    fn empty_state(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let view = cx.entity();
        ui::card()
            .child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_3()
                    .child(ui::section_title("No custom modes"))
                    .child(
                        ui::hint(
                            "Add a built .lwmode file to distribute its Lua scripts and option specification with this pack.",
                        )
                        .max_w(px(420.))
                        .text_center(),
                    )
                    .child(
                        Button::new("add-mode-empty", "Add mode...")
                            .variant(Variant::Primary)
                            .size(Size::Compact)
                            .loading(self.adding)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| this.add_mode(cx))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn remove_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let mode = self.removing.as_ref()?;
        let (cancel, remove, close) = (cx.entity(), cx.entity(), cx.entity());
        Some(ui::dialog(
            format!("Remove \"{}\"?", mode.name),
            "The mode's Lua scripts and option specification will be removed from this pack. You can undo this.",
            vec![
                DialogAction::new("Cancel", Variant::Secondary, move |_, _, cx| {
                    cancel.update(cx, |this, cx| {
                        this.removing = None;
                        cx.notify();
                    })
                }),
                DialogAction::new("Remove mode", Variant::Destructive, move |_, _, cx| {
                    remove.update(cx, |this, cx| this.remove(cx))
                }),
            ],
            move |_, cx| {
                close.update(cx, |this, cx| {
                    this.removing = None;
                    cx.notify();
                })
            },
            &self.dialog_focus,
            window,
        ))
    }
}

impl Render for ModesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let modes = match &self.modes {
            Ok(modes) => modes.clone(),
            Err(_) => Vec::new(),
        };
        let recommended = match self.editor.read(cx).session().metadata().recommended_mode {
            Some(RecommendedMode::Pack { id }) => Some(id),
            _ => None,
        };
        let mut sections = vec![self.recommendation(&modes, cx)];
        sections.push(match &self.modes {
            Err(error) => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(ui::section_title("Couldn't read the modes"))
                .child(ui::hint(error.clone()).text_color(c(theme::DANGER)))
                .into_any_element(),
            Ok(modes) if modes.is_empty() => self.empty_state(cx),
            Ok(modes) => {
                let view = cx.entity();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(ui::section_title("Custom modes"))
                            .child(
                                Button::new("add-mode", "Add mode...")
                                    .variant(Variant::Secondary)
                                    .size(Size::Compact)
                                    .loading(self.adding)
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |this, cx| this.add_mode(cx))
                                    }),
                            ),
                    )
                    .child(self.mode_list(modes, recommended, cx))
                    .into_any_element()
            }
        });
        let dialog = self.remove_dialog(window, cx);
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(ui::page(
                "modes-page",
                "Modes",
                "Bundle a Lua mode with this pack, and choose which mode the pack starts in.",
                sections,
            ))
            .children(dialog)
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// `format.ts`'s `formatFileSize`: B, KB, MB, GB with one decimal past bytes.
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit < UNITS.len() - 1 {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::format_size;

    #[test]
    fn sizes_read_like_the_tauri_editor() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(5 * 1024 * 1024 + 1024 * 512), "5.5 MB");
    }
}
