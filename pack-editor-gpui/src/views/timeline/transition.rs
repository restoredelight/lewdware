//! One transition between two stages: how long it takes, its easing, and which values change
//! gradually. Ported from `TransitionEditor.svelte`.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, FontWeight, SharedString, Window};
use shared::behaviour::{Easing, Transition, TransitionCategory};

use super::fields::{labelled, number_text, Spec};
use super::widgets::{field_grid, note, section};
use super::{editor_panel, Snapshot, TimelineView};
use crate::theme::{self, c};
use crate::ui::{self, Select, SelectOption};

/// The gradual changes, grouped as the author sees them, with the broad category early editor
/// builds stored for the whole group.
/// A group's heading, its legacy broad category, and its values with their labels.
type Group = (
    &'static str,
    Option<TransitionCategory>,
    &'static [(TransitionCategory, &'static str)],
);

const GROUPS: [Group; 4] = [
    (
        "Event intervals",
        Some(TransitionCategory::Events),
        &[
            (TransitionCategory::PopupInterval, "Popups"),
            (TransitionCategory::WebInterval, "Web links"),
            (TransitionCategory::NotificationInterval, "Notifications"),
            (TransitionCategory::PromptInterval, "Prompts"),
            (TransitionCategory::SoundInterval, "Sounds"),
        ],
    ),
    (
        "Movement",
        Some(TransitionCategory::Movement),
        &[
            (TransitionCategory::MovementMinimumSpeed, "Minimum speed"),
            (TransitionCategory::MovementMaximumSpeed, "Maximum speed"),
        ],
    ),
    (
        "Mitosis",
        Some(TransitionCategory::Mitosis),
        &[
            (TransitionCategory::MitosisChance, "Chance"),
            (TransitionCategory::MitosisCount, "Number of copies"),
        ],
    ),
    (
        "Audio",
        None,
        &[(TransitionCategory::Crossfade, "Crossfade background audio")],
    ),
];

impl TimelineView {
    pub(super) fn transition_editor(
        &mut self,
        snapshot: &Snapshot,
        transition: &Transition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let label_of = |id: &str| {
            snapshot
                .stages
                .iter()
                .find(|stage| stage.id == id)
                .map(|stage| stage.label.clone())
                .unwrap_or_default()
        };
        let (from, to) = (
            label_of(&transition.from_stage),
            label_of(&transition.to_stage),
        );
        let id = transition.id.clone();
        let instant = transition.duration_seconds == 0.0;

        let stage_link = |key: &str, label: &str, stage: &str, cx: &mut Context<Self>| {
            let stage = stage.to_string();
            div()
                .id(SharedString::from(format!("{id}-{key}")))
                .cursor_pointer()
                .underline()
                .child(label.to_string())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.select(stage.clone(), window, cx)),
                )
        };
        let heading = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(ui::readout("Transition"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(17.))
                    .font_weight(FontWeight(650.))
                    .child(stage_link("from", &from, &transition.from_stage, cx))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(c(theme::MUTED))
                            .child("to"),
                    )
                    .child(stage_link("to", &to, &transition.to_stage, cx)),
            )
            .child(ui::hint(format!(
                "Choose how values change after {from} has finished."
            )));

        let duration_id = id.clone();
        let duration = self.number_field(
            "Duration (seconds)",
            None,
            Spec::new(
                format!("{id}-duration"),
                number_text(transition.duration_seconds),
            ),
            Some("s"),
            move |this, value, cx| {
                // Blank is mid-edit, not a request for an instant transition.
                if let Some(seconds) = value {
                    let seconds = seconds.max(0.0);
                    this.apply(cx, |session| {
                        session.set_transition_duration(&duration_id, seconds)
                    });
                }
            },
            window,
            cx,
        );
        let easing_id = id.clone();
        let view = cx.entity();
        let easing = labelled(
            "Easing",
            None,
            Select::new(
                SharedString::from(format!("{id}-easing")),
                transition.easing,
                vec![
                    SelectOption::new(Easing::Linear, "Linear"),
                    SelectOption::new(Easing::EaseIn, "Ease in"),
                    SelectOption::new(Easing::EaseOut, "Ease out"),
                    SelectOption::new(Easing::EaseInOut, "Ease in and out"),
                ],
            )
            .disabled(instant)
            .on_change(move |easing, _, cx| {
                let id = easing_id.clone();
                view.update(cx, |this, cx| {
                    this.apply(cx, |session| session.set_transition_easing(&id, easing));
                })
            }),
        );
        let timing = section(
            "Timing",
            "A zero-second transition changes everything immediately.",
            None,
        )
        .child(field_grid(vec![
            duration.into_any_element(),
            easing.into_any_element(),
        ]));

        let mut gradual = section(
            "Gradual changes",
            "Choose exactly which numeric values interpolate. All other changes take effect at \
             the end.",
            None,
        )
        .when(instant, |this| this.opacity(0.6));
        for (group_index, (name, legacy, values)) in GROUPS.iter().enumerate() {
            let boxes = values.iter().map(|(category, label)| {
                // A legacy broad entry covers every member of its group.
                let checked = transition.affected.contains(category)
                    || legacy.is_some_and(|legacy| transition.affected.contains(&legacy));
                let (category, toggle_id) = (*category, id.clone());
                div()
                    .id(SharedString::from(format!("{id}-gradual-{category:?}")))
                    .min_w(px(180.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(13.))
                    .child(ui::checkbox(checked))
                    .child(*label)
                    .when(!instant, |this| {
                        this.cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.apply(cx, |session| {
                                    session.set_transition_category(&toggle_id, category, !checked)
                                });
                            }))
                    })
            });
            gradual = gradual.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(group_index > 0, |this| {
                        this.pt(px(10.)).border_t_1().border_color(c(theme::BORDER))
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(*name),
                    )
                    .child(div().flex().flex_wrap().gap_x_4().children(boxes)),
            );
        }

        editor_panel(
            &id,
            vec![
                heading.into_any_element(),
                timing.into_any_element(),
                gradual.into_any_element(),
                note(format!(
                    "Content selections and enabled or disabled features switch to {to}'s values \
                     when this transition ends."
                ))
                .into_any_element(),
            ],
        )
    }
}
