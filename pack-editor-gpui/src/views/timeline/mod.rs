//! The timeline (shown to players as the "Sequence" mode), its stages and the transitions between
//! them. Ported from `pack-editor/src/lib/Experience.svelte`, `TimelineEditor.svelte`,
//! `StageTabs.svelte`, `StageEditor.svelte`, `TransitionEditor.svelte`,
//! `EventScheduleEditor.svelte`, `MediaSlot.svelte` and `StageMembership.svelte`.
//!
//! | Module | What |
//! | --- | --- |
//! | this one | the view: header, the stage list and its structural edits (add, duplicate, move, remove) |
//! | `stage` | one stage's own fields |
//! | `transition` | one transition |
//! | `widgets` | cards, toggle rows, tag lists, media slots |
//! | `fields` | text and number fields that commit on Enter or blur |
//! | `membership` | "Appears in": which stages one file is in, for the library inspector |
//!
//! Everything is read from the session in `render` and redrawn when a change in the timeline's
//! scopes arrives, from wherever it came. Every write is one `Editor::apply`, one undo entry.

mod fields;
mod membership;
mod stage;
mod transition;
mod widgets;

// For the library inspector (Media library track); see `membership`.
pub use membership::{stage_membership, StageMembership};

use std::collections::HashMap;

use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, AnyView, App, Context, Entity, FocusHandle, FontWeight, SharedString,
    Subscription, Window,
};
use pack_editor_core::changes::Scope;
use pack_editor_core::pack::ops::timeline::{self as ops, StageRemovalImpact};
use pack_editor_core::session::{Applied, Session};
use shared::behaviour::editor as queries;
use shared::behaviour::{Stage, Transition};

use crate::editor::{Editor, EditorEvent};
use crate::shell::View;
use crate::theme::{self, c};
use crate::ui::{self, Button, DialogAction, Size, Toggle, Variant};

use fields::Spec;

pub fn create(view: View, editor: Entity<Editor>, window: &mut Window, cx: &mut App) -> AnyView {
    debug_assert!(matches!(view, View::Timeline));
    cx.new(|cx| TimelineView::new(editor, window, cx)).into()
}

/// The timeline as one render reads it.
struct Snapshot {
    enabled: bool,
    label: Option<String>,
    stages: Vec<Stage>,
    transitions: Vec<Transition>,
}

impl Snapshot {
    fn read(session: &Session) -> anyhow::Result<Self> {
        Ok(match session.read(queries::timeline)? {
            Some(view) => Snapshot {
                enabled: view.enabled,
                label: view.timeline.label,
                stages: view.timeline.stages,
                transitions: view.timeline.transitions,
            },
            None => Snapshot {
                enabled: false,
                label: None,
                stages: Vec::new(),
                transitions: Vec::new(),
            },
        })
    }

    fn transition_after(&self, index: usize) -> Option<&Transition> {
        let (from, to) = (self.stages.get(index)?, self.stages.get(index + 1)?);
        self.transitions
            .iter()
            .find(|transition| transition.from_stage == from.id && transition.to_stage == to.id)
    }
}

/// A stage the author asked to remove, waiting on the confirmation.
struct Removing {
    id: String,
    label: String,
    has_wallpaper: bool,
    has_audio: bool,
    impact: StageRemovalImpact,
}

/// A stage being dragged in the list.
#[derive(Clone)]
struct DraggedStage {
    id: String,
    label: SharedString,
}

impl Render for DraggedStage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(176.))
            .h(px(44.))
            .px_3()
            .flex()
            .items_center()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .bg(c(theme::SURFACE_RAISED))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(theme::TEXT))
            .opacity(0.9)
            .child(self.label.clone())
    }
}

pub struct TimelineView {
    editor: Entity<Editor>,
    focus: FocusHandle,
    dialog_focus: FocusHandle,
    /// The stage or transition being edited, by id. Falls back to the first stage.
    active: Option<String>,
    fields: HashMap<SharedString, fields::Field>,
    picking: Option<widgets::Picking>,
    removing: Option<Removing>,
    /// "Switch to a specific track" was chosen for this stage, and no track is picked yet.
    audio_picker_stage: Option<String>,
    _changes: Subscription,
}

impl TimelineView {
    fn new(editor: Entity<Editor>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _changes = cx.subscribe(&editor, |_, _, event: &EditorEvent, cx| match event {
            EditorEvent::Changed(changes) => {
                // Tags and media for the tag lists and the slots' file names.
                if changes
                    .scopes
                    .intersects(Scope::Timeline | Scope::Slots | Scope::Tags | Scope::Media)
                {
                    cx.notify();
                }
            }
        });
        Self {
            editor,
            focus: cx.focus_handle(),
            dialog_focus: cx.focus_handle(),
            active: None,
            fields: HashMap::new(),
            picking: None,
            removing: None,
            audio_picker_stage: None,
            _changes,
        }
    }

    /// One write: one undo entry. Failures show in the toolbar.
    fn apply<T>(
        &self,
        cx: &mut Context<Self>,
        write: impl FnOnce(&mut Session) -> anyhow::Result<Applied<T>>,
    ) -> Option<T> {
        self.editor.update(cx, |editor, cx| editor.apply(cx, write))
    }

    fn select(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        // A field on the stage being left commits before it goes.
        self.commit_focused(window, cx);
        self.picking = None;
        self.active = Some(id);
        cx.notify();
    }

    // ── Structural edits ────────────────────────────────────────────────────────────────────

    fn add_stage(&mut self, after: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        // Beside the stage it follows, copying its settings: the author's expectation when adding
        // a stage next to one they've configured. Tag ownership is never copied.
        let added = self.apply(cx, |session| {
            session.add_stage(after.as_deref(), after.as_deref(), None)
        });
        if let Some(id) = added {
            self.select(id, window, cx);
        }
    }

    fn duplicate_stage(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        if let Some(copy) = self.apply(cx, |session| session.duplicate_stage(&id)) {
            self.select(copy, window, cx);
        }
    }

    fn move_stage(&mut self, id: String, to: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        self.apply(cx, |session| session.move_stage(&id, to));
    }

    fn ask_remove(&mut self, stage: &Stage, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_focused(window, cx);
        let impact = self
            .editor
            .read(cx)
            .session()
            .read(|c| ops::stage_removal_impact(c, &stage.id))
            .unwrap_or_default();
        self.removing = Some(Removing {
            id: stage.id.clone(),
            label: stage.label.clone(),
            has_wallpaper: stage.content.wallpaper.is_some(),
            has_audio: stage.content.audio.is_some(),
            impact,
        });
        window.focus(&self.dialog_focus, cx);
        cx.notify();
    }

    fn confirm_remove(
        &mut self,
        also_remove_tag: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(removing) = self.removing.take() else {
            return;
        };
        // Then the stage after it (or before, for the last), as the Tauri editor does.
        let stages = self
            .editor
            .read(cx)
            .session()
            .read(queries::stages)
            .unwrap_or_default();
        let next = stages
            .iter()
            .position(|stage| stage.id == removing.id)
            .and_then(|index| {
                stages
                    .get(index + 1)
                    .or_else(|| index.checked_sub(1).and_then(|before| stages.get(before)))
            })
            .map(|stage| stage.id.clone());
        if self
            .apply(cx, |session| {
                session.remove_stage(&removing.id, also_remove_tag)
            })
            .is_some()
        {
            self.active = next;
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn cancel_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.removing = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    // ── Rendering ───────────────────────────────────────────────────────────────────────────

    fn header(
        &mut self,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let enabled = snapshot.enabled;
        let view = cx.entity();
        let bar = div()
            .flex_none()
            .h(px(44.))
            .px_4()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Timeline"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(ui::readout(if enabled { "Enabled" } else { "Disabled" }))
                    .child(Toggle::new("timeline-enabled", enabled).on_change(
                        move |on, window, cx| {
                            view.update(cx, |this, cx| {
                                this.commit_focused(window, cx);
                                this.apply(cx, |session| session.set_timeline_enabled(on));
                            })
                        },
                    )),
            );
        let lede = div()
            .flex_none()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(ui::hint(
                "Read by the built-in modes (Sandbox and Sequence). A custom mode reads none of this.",
            ));
        let mut header = div().flex_none().flex().flex_col().child(bar).child(lede);
        if enabled {
            let key: SharedString = "mode-name".into();
            let input = self.field(
                Spec::new(key.clone(), snapshot.label.clone().unwrap_or_default())
                    .placeholder("Sequence"),
                |this, text, cx| {
                    // Blank means no override: the mode keeps its own name.
                    let text = text.to_string();
                    this.apply(cx, |session| session.set_timeline_label(Some(&text)));
                },
                window,
                cx,
            );
            header = header.child(
                div()
                    .flex_none()
                    .px_4()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Mode name"),
                    )
                    .child(self.field_box(&key, &input, None, window, cx).w(px(224.)))
                    .child(
                        ui::hint("What players see this mode called. Blank keeps \"Sequence\".")
                            .text_size(px(11.)),
                    ),
            );
        }
        header
    }

    fn switched_off(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .p_8()
            .child(
                div()
                    .max_w(px(448.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(ui::section_title("Timeline is off"))
                    .child(
                        ui::hint(
                            "A timeline allows you to create a more interactive experience for \
                             your pack, without writing your own mode.",
                        )
                        .text_size(px(13.))
                        .text_center(),
                    )
                    .child(
                        div().mt_2().child(
                            Button::new("timeline-turn-on", "Turn on timeline")
                                .size(Size::Compact)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.apply(cx, |session| session.set_timeline_enabled(true));
                                })),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn stage_list(&self, snapshot: &Snapshot, active: &str, cx: &mut Context<Self>) -> AnyElement {
        let count = snapshot.stages.len();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, stage) in snapshot.stages.iter().enumerate() {
            rows.push(self.stage_row(index, count, stage, active == stage.id, cx));
            if let Some(transition) = snapshot.transition_after(index) {
                rows.push(self.transition_row(index, transition, active == transition.id, cx));
            }
        }
        // With a transition selected, a new stage goes after its first stage; otherwise after the
        // selected stage.
        let after = snapshot
            .stages
            .iter()
            .find(|stage| stage.id == active)
            .or_else(|| {
                let transition = snapshot.transitions.iter().find(|item| item.id == active)?;
                snapshot
                    .stages
                    .iter()
                    .find(|stage| stage.id == transition.from_stage)
            })
            .or(snapshot.stages.last())
            .map(|stage| stage.id.clone());

        div()
            .flex_none()
            .w(px(200.))
            .h_full()
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .border_r_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::SURFACE))
            .child(
                div()
                    .id("stage-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(div().flex().flex_col().gap(px(2.)).children(rows)),
            )
            .child(
                Button::new("add-stage", "Add stage")
                    .size(Size::Compact)
                    .full_width()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.add_stage(after.clone(), window, cx)
                    })),
            )
            .into_any_element()
    }

    fn stage_row(
        &self,
        index: usize,
        count: usize,
        stage: &Stage,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let duration = match (&stage.end, index + 1 == count) {
            (Some(end), false) => match end.duration_seconds {
                Some(seconds) => format_seconds(seconds),
                None => "on events".to_string(),
            },
            _ => "until end".to_string(),
        };
        let label: SharedString = if stage.label.is_empty() {
            format!("Stage {}", index + 1).into()
        } else {
            stage.label.clone().into()
        };
        let dragged = DraggedStage {
            id: stage.id.clone(),
            label: label.clone(),
        };
        let select_id = stage.id.clone();
        div()
            .id(SharedString::from(format!("stage-row-{}", stage.id)))
            .relative()
            .flex_none()
            .h(px(44.))
            .pl_2()
            .pr_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(if active {
                theme::BORDER_STRONG
            } else {
                theme::BORDER
            }))
            .bg(c(if active {
                theme::SURFACE_RAISED
            } else {
                theme::BG
            }))
            .cursor_pointer()
            .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
            // Selection is an edge, not a fill.
            .when(active, |this| {
                this.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.))
                        .bg(c(theme::ACCENT_HOVER)),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.))
                    .text_color(c(theme::MUTED))
                    .child(gpui_kit::assets::IconName::Menu),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .when(active, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .child(label),
            )
            .child(ui::readout(duration).flex_none())
            .on_click(
                cx.listener(move |this, _, window, cx| this.select(select_id.clone(), window, cx)),
            )
            .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            .drag_over::<DraggedStage>(|style, _, _, _| style.border_color(c(theme::FOCUS)))
            .on_drop(
                cx.listener(move |this, dragged: &DraggedStage, window, cx| {
                    this.move_stage(dragged.id.clone(), index, window, cx)
                }),
            )
            .into_any_element()
    }

    fn transition_row(
        &self,
        index: usize,
        transition: &Transition,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text = if transition.duration_seconds == 0.0 {
            "immediate".to_string()
        } else {
            format_seconds(transition.duration_seconds)
        };
        let id = transition.id.clone();
        div()
            .id(("transition-row", index))
            .flex_none()
            .h(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .h(px(22.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(if active {
                        theme::ACCENT_HOVER
                    } else {
                        theme::BORDER
                    }))
                    .bg(c(if active {
                        theme::SURFACE_RAISED
                    } else {
                        theme::SURFACE
                    }))
                    .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                    .child(ui::readout(text).text_color(c(if active {
                        theme::TEXT
                    } else {
                        theme::MUTED
                    }))),
            )
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| this.select(id.clone(), window, cx)))
            .into_any_element()
    }

    fn removal_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let removing = self.removing.as_ref()?;
        let mut description = String::from(
            "This stage and its settings will be removed. Transitions to its neighbours will be \
             reset.",
        );
        if removing.has_wallpaper {
            description.push_str(" Its wallpaper is cleared with it.");
        }
        if removing.has_audio {
            description.push_str(" Its audio slot is cleared with it.");
        }
        let impact = &removing.impact;
        if let Some(tag) = &impact.tag {
            if impact.claimed() {
                let mut parts = Vec::new();
                if impact.media > 0 {
                    parts.push(format!(
                        "{} file{}",
                        impact.media,
                        if impact.media == 1 { "" } else { "s" }
                    ));
                }
                match impact.stages.as_slice() {
                    [] => {}
                    [one] => parts.push(format!("\"{one}\"")),
                    many => parts.push(format!("{} other stages", many.len())),
                }
                if impact.content > 0 {
                    parts.push(format!(
                        "{} content {}",
                        impact.content,
                        if impact.content == 1 {
                            "entry"
                        } else {
                            "entries"
                        }
                    ));
                }
                description.push_str(&format!(
                    " Its tag \"{tag}\" is on {}, and is kept unless you remove it too.",
                    parts.join(", ")
                ));
            } else {
                description.push_str(&format!(" Its tag \"{tag}\" is unused and goes with it."));
            }
        }

        let view = cx.entity();
        let mut actions = vec![DialogAction::new("Cancel", Variant::Secondary, {
            let view = view.clone();
            move |_, window, cx| view.update(cx, |this, cx| this.cancel_remove(window, cx))
        })];
        // Offered, never the default: losing a classification to a restructuring is a worse trade
        // than an unused tag left behind.
        if let Some(tag) = impact.tag.as_ref().filter(|_| impact.claimed()) {
            let view = view.clone();
            actions.push(DialogAction::new(
                format!("Remove stage and \"{tag}\""),
                Variant::Destructive,
                move |_, window, cx| {
                    view.update(cx, |this, cx| this.confirm_remove(true, window, cx))
                },
            ));
        }
        actions.push(DialogAction::new("Remove stage", Variant::Destructive, {
            let view = view.clone();
            move |_, window, cx| view.update(cx, |this, cx| this.confirm_remove(false, window, cx))
        }));
        Some(
            ui::dialog(
                format!("Remove \"{}\"?", removing.label),
                description,
                actions,
                move |window, cx| view.update(cx, |this, cx| this.cancel_remove(window, cx)),
                &self.dialog_focus,
                window,
            )
            .into_any_element(),
        )
    }
}

impl Render for TimelineView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.begin_fields();
        let snapshot = match Snapshot::read(self.editor.read(cx).session()) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.end_fields();
                return div()
                    .size_full()
                    .p_8()
                    .child(ui::hint(format!("Couldn't read the timeline: {error:#}")))
                    .into_any_element();
            }
        };

        // Nothing selected, or a selection the timeline no longer has: the first stage.
        let known = |id: &String| {
            snapshot.stages.iter().any(|stage| &stage.id == id)
                || snapshot.transitions.iter().any(|item| &item.id == id)
        };
        if !self.active.as_ref().is_some_and(known) {
            self.active = snapshot.stages.first().map(|stage| stage.id.clone());
        }
        let active = self.active.clone().unwrap_or_default();

        let header = self.header(&snapshot, window, cx);
        let body = if !snapshot.enabled {
            self.switched_off(cx)
        } else {
            let editor =
                if let Some(index) = snapshot.stages.iter().position(|stage| stage.id == active) {
                    self.stage_editor(&snapshot, index, window, cx)
                } else if let Some(transition) =
                    snapshot.transitions.iter().find(|item| item.id == active)
                {
                    self.transition_editor(&snapshot, transition, window, cx)
                } else {
                    div()
                        .flex_1()
                        .p_8()
                        .child(ui::hint("Add a stage to start the timeline."))
                        .into_any_element()
                };
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(self.stage_list(&snapshot, &active, cx))
                .child(editor)
                .into_any_element()
        };
        let dialog = self.removal_dialog(window, cx);
        self.end_fields();

        div()
            .id("timeline-view")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            .child(header)
            .child(body)
            .children(dialog)
            .into_any_element()
    }
}

/// `45 s`, `5 min`, `2.5 min`.
fn format_seconds(seconds: f64) -> String {
    if seconds < 60.0 {
        return format!("{} s", fields::number_text(seconds));
    }
    let minutes = (seconds / 60.0 * 10.0).round() / 10.0;
    format!("{} min", fields::number_text(minutes))
}

/// The panel a stage or transition is edited in: scrolls, capped at 800px. Keyed by what it shows,
/// so another stage starts at the top.
fn editor_panel(id: &str, children: Vec<AnyElement>) -> AnyElement {
    div()
        .id(SharedString::from(format!("timeline-editor-{id}")))
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .child(
            div()
                .mx_auto()
                .w_full()
                .max_w(px(800.))
                .p_6()
                .flex()
                .flex_col()
                .gap(px(14.))
                .children(children),
        )
        .into_any_element()
}
