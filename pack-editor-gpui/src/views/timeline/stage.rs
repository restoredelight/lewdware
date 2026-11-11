//! One stage's own fields: what it shows, what it fires, and how long it lasts. Ported from
//! `StageEditor.svelte` and `EventScheduleEditor.svelte`.
//!
//! Every control writes one field of this stage, addressed by its id. Values that depend on the
//! stage's current state (an event schedule's other fields, the end condition's count) are read
//! from the pack when the edit commits, not from what this render saw.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Focusable, FontWeight, SharedString, Window};
use pack_editor_core::pack::ops::timeline::{self as ops, StageAudioMode};
use shared::behaviour::editor as queries;
use shared::behaviour::{EndStrategy, EventKind, EventSchedule, Interval, MediaSlot, Stage};

use super::fields::{labelled, number_text, parse_number, Spec};
use super::widgets::{field_grid, listen_bool, note, section, toggle_row, SlotSpec};
use super::{editor_panel, Snapshot, TimelineView};
use crate::theme::{self, c};
use crate::ui::{self, Button, Select, SelectOption, Size, Toggle, Variant};

/// The event kinds a stage schedules, their names and the interval each starts at.
const EVENTS: [(EventKind, &str, f64); 5] = [
    (EventKind::Popup, "Popups", 30.0),
    (EventKind::Web, "Web links", 300.0),
    (EventKind::Notification, "Notifications", 300.0),
    (EventKind::Prompt, "Prompts", 90.0),
    (EventKind::Sound, "Sounds", 90.0),
];

fn event_name(kind: EventKind) -> &'static str {
    EVENTS
        .iter()
        .find(|(event, _, _)| *event == kind)
        .map_or("Events", |(_, name, _)| name)
}

/// `every 30s`, `every 20-40s`.
fn schedule_text(schedule: &EventSchedule) -> String {
    match schedule.interval {
        Interval::Fixed { seconds } => format!("every {}s", number_text(seconds)),
        Interval::Random {
            minimum_seconds,
            maximum_seconds,
        } => format!(
            "every {}-{}s",
            number_text(minimum_seconds),
            number_text(maximum_seconds)
        ),
    }
}

fn optional_text(value: Option<f64>) -> String {
    value.map(number_text).unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq)]
enum AudioChoice {
    Keep,
    Random,
    Specific,
}

#[derive(Clone, Copy, PartialEq)]
enum IntervalKind {
    Fixed,
    Random,
}

impl TimelineView {
    pub(super) fn stage_editor(
        &mut self,
        snapshot: &Snapshot,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let stage = snapshot.stages[index].clone();
        let previous = index
            .checked_sub(1)
            .and_then(|before| snapshot.stages.get(before))
            .cloned();
        let children = vec![
            self.stage_heading(&stage, index, snapshot.stages.len(), window, cx),
            self.content_card(snapshot, index, &stage, previous.as_ref(), window, cx),
            self.entry_card(&stage, window, cx),
            self.events_card(&stage, previous.as_ref(), window, cx),
            self.movement_card(&stage, previous.as_ref(), window, cx),
            self.mitosis_card(&stage, previous.as_ref(), window, cx),
            self.duration_card(snapshot, index, &stage, window, cx),
        ];
        editor_panel(&stage.id, children)
    }

    fn stage_heading(
        &mut self,
        stage: &Stage,
        index: usize,
        count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = stage.id.clone();
        let key: SharedString = format!("{id}-name").into();
        let rename = id.clone();
        let input = self.field(
            Spec::new(key.clone(), stage.label.clone()),
            move |this, text, cx| {
                // The tag the stage owns is renamed in the same edit.
                let text = text.to_string();
                this.apply(cx, |session| session.set_stage_label(&rename, &text));
            },
            window,
            cx,
        );
        let name_focused = input.read(cx).focus_handle(cx).is_focused(window);
        let action = |name: &'static str, label: &'static str| {
            Button::new(SharedString::from(format!("{id}-{name}")), label)
                .variant(Variant::Quiet)
                .size(Size::Compact)
        };
        let (up, down, copy) = (id.clone(), id.clone(), id.clone());
        let removing = stage.clone();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                self.field_box_sized(&key, &input, None, px(17.), window, cx)
                    .flex_1()
                    .h(px(38.))
                    .font_weight(FontWeight(650.))
                    // A title until it's being edited.
                    .when(!name_focused, |this| {
                        this.border_color(gpui_kit::transparent_black())
                            .bg(gpui_kit::transparent_black())
                            .hover(|style| style.border_color(c(theme::BORDER)).bg(c(theme::BG)))
                    }),
            )
            .child(
                action("move-up", "Move up")
                    .icon(gpui_kit::assets::IconName::ArrowUp)
                    .disabled(index == 0)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.move_stage(up.clone(), index.saturating_sub(1), window, cx)
                    })),
            )
            .child(
                action("move-down", "Move down")
                    .icon(gpui_kit::assets::IconName::ArrowDown)
                    .disabled(index + 1 == count)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.move_stage(down.clone(), index + 1, window, cx)
                    })),
            )
            .child(
                action("duplicate", "Duplicate")
                    .icon(gpui_kit::assets::IconName::Copy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.duplicate_stage(copy.clone(), window, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("{id}-remove")), "Remove")
                    .icon(gpui_kit::assets::IconName::Trash)
                    .variant(Variant::Destructive)
                    .size(Size::Compact)
                    .disabled(count == 1)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.ask_remove(&removing, window, cx)
                    })),
            )
            .into_any_element()
    }

    /// The pack's tags the author can pick from: everything but Lewdware's own.
    fn author_tags(&self, cx: &Context<Self>) -> Vec<String> {
        self.editor
            .read(cx)
            .session()
            .read(ops::author_tags)
            .unwrap_or_default()
    }

    fn content_card(
        &mut self,
        snapshot: &Snapshot,
        index: usize,
        stage: &Stage,
        previous: Option<&Stage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = stage.id.clone();
        let all_tags = self.author_tags(cx);
        let restricts = stage.content.tags.is_some();
        let restrict_id = id.clone();
        let mut card = section("Content", "", None).child(toggle_row(
            "Active tags",
            previous.map(|previous| {
                match &previous.content.tags {
                    Some(tags) => format!("Previous stage: {} selected", tags.len()),
                    None => "Previous stage: all content".to_string(),
                }
                .into()
            }),
            Toggle::new(SharedString::from(format!("{id}-restricts")), restricts).on_change(
                listen_bool(cx, move |this, on: &bool, _, cx| {
                    let on = *on;
                    this.apply(cx, |session| {
                        session.set_stage_restricts_content(&restrict_id, on)
                    });
                }),
            ),
            true,
        ));
        if let Some(tags) = &stage.content.tags {
            let tag_id = id.clone();
            card = card.child(self.tag_list(
                format!("{id}-include"),
                tags,
                stage.content.owned_tag.as_deref(),
                &all_tags,
                move |this, tag, present, cx| {
                    this.apply(cx, |session| {
                        session.set_stage_tag(&tag_id, &tag, false, present)
                    });
                },
                window,
                cx,
            ));
        } else {
            card = card.child(note("All content"));
        }

        let exclude_id = id.clone();
        card = card
            .child(toggle_row("Excluded tags", None, div(), true))
            .child(self.tag_list(
                format!("{id}-exclude"),
                &stage.content.exclude,
                stage.content.owned_exclude_tag.as_deref(),
                &all_tags,
                move |this, tag, present, cx| {
                    this.apply(cx, |session| {
                        session.set_stage_tag(&exclude_id, &tag, true, present)
                    });
                },
                window,
                cx,
            ));
        if let Some(owned) = &stage.content.owned_exclude_tag {
            card = card.child(note(format!(
                "\"{owned}\" is this stage's own exclusion tag. Every file you take out of this \
                 stage from \"Appears in\" carries it."
            )));
        }

        // What this stage's wallpaper is if it sets none: the nearest earlier stage's, or the
        // pack's. Empty isn't "none", it's "whatever is already up", so name it.
        let session = self.editor.read(cx).session();
        let inherited = snapshot.stages[..index]
            .iter()
            .rev()
            .find_map(|earlier| earlier.content.wallpaper)
            .or_else(|| {
                session
                    .read(queries::media_slots)
                    .ok()
                    .and_then(|slots| slots.wallpaper)
            })
            .and_then(|id| session.media().get(id))
            .map(|file| file.file_name.clone());
        card = card.child(
            div()
                .pt(px(10.))
                .border_t_1()
                .border_color(c(theme::BORDER))
                .child(self.media_slot(
                    SlotSpec {
                        slot: MediaSlot::StageWallpaper { stage: id.clone() },
                        title: "Wallpaper",
                        description: None,
                        empty_note: match inherited {
                            Some(name) => format!("Keeps \"{name}\".").into(),
                            None => "Keeps the current wallpaper".into(),
                        },
                    },
                    stage.content.wallpaper,
                    window,
                    cx,
                )),
        );

        // Background audio: keep, random from the stage's tags, or one track.
        let specific = stage.content.audio.is_some()
            || self.audio_picker_stage.as_deref() == Some(id.as_str());
        let choice = if specific {
            AudioChoice::Specific
        } else if stage.content.audio_random {
            AudioChoice::Random
        } else {
            AudioChoice::Keep
        };
        let audio_id = id.clone();
        let view = cx.entity();
        let mut audio = div().flex().flex_col().gap_3().child(labelled(
            "Change the background audio",
            None,
            Select::new(
                SharedString::from(format!("{id}-audio-mode")),
                choice,
                vec![
                    SelectOption::new(AudioChoice::Keep, "When the current track ends"),
                    SelectOption::new(AudioChoice::Random, "Switch to a random track"),
                    SelectOption::new(AudioChoice::Specific, "Switch to a specific track"),
                ],
            )
            .on_change(move |picked, _, cx| {
                let id = audio_id.clone();
                view.update(cx, |this, cx| {
                    this.audio_picker_stage = (picked == AudioChoice::Specific).then(|| id.clone());
                    // Whichever way, a track this stage named is let go of.
                    let mode = if picked == AudioChoice::Random {
                        StageAudioMode::Random
                    } else {
                        StageAudioMode::Keep
                    };
                    this.apply(cx, |session| session.set_stage_audio_mode(&id, mode));
                    cx.notify();
                })
            }),
        ));
        if specific {
            audio = audio.child(self.media_slot(
                SlotSpec {
                    slot: MediaSlot::StageAudio { stage: id.clone() },
                    title: "Specific track",
                    description: Some("Choose the exact track this stage starts."),
                    empty_note: "Choose a track.".into(),
                },
                stage.content.audio,
                window,
                cx,
            ));
        }
        card.child(audio).into_any_element()
    }

    fn entry_card(
        &mut self,
        stage: &Stage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = stage.id.clone();
        let splash = self.media_slot(
            SlotSpec {
                slot: MediaSlot::StageEntrySplash { stage: id.clone() },
                title: "Splash",
                description: None,
                empty_note: "No splash is shown when the stage starts.".into(),
            },
            stage.on_enter.splash,
            window,
            cx,
        );
        let sound = self.media_slot(
            SlotSpec {
                slot: MediaSlot::StageEntrySound { stage: id.clone() },
                title: "Sound",
                description: None,
                empty_note: "No sound is played when the stage starts.".into(),
            },
            stage.on_enter.sound,
            window,
            cx,
        );
        let notify_id = id.clone();
        let notification = self.text_field(
            "Notification",
            Spec::new(
                format!("{id}-entry-notification"),
                stage.on_enter.notification.clone().unwrap_or_default(),
            )
            .placeholder("No notification"),
            move |this, text, cx| {
                let text = text.trim().to_string();
                this.apply(cx, |session| {
                    session.set_stage_entry_notification(&notify_id, Some(&text))
                });
            },
            window,
            cx,
        );
        let burst = stage.on_enter.popup_burst;
        let toggle_id = id.clone();
        let mut popups = div().flex().flex_col().gap_3().child(toggle_row(
            "Spawn extra popups",
            None,
            Toggle::new(
                SharedString::from(format!("{id}-entry-burst")),
                burst.is_some(),
            )
            .on_change(listen_bool(cx, move |this, on: &bool, _, cx| {
                let count = on.then_some(burst.unwrap_or(5));
                this.apply(cx, |session| {
                    session.set_stage_entry_popup_burst(&toggle_id, count)
                });
            })),
            true,
        ));
        if let Some(count) = burst {
            let count_id = id.clone();
            popups = popups.child(self.number_field(
                "Number of popups",
                None,
                Spec::new(format!("{id}-entry-burst-count"), count.to_string()),
                None,
                move |this, value, cx| {
                    if let Some(value) = value {
                        let count = value.floor().max(1.0) as u32;
                        this.apply(cx, |session| {
                            session.set_stage_entry_popup_burst(&count_id, Some(count))
                        });
                    }
                },
                window,
                cx,
            ));
        }
        section(
            "On stage start",
            "Fire these effects once, after the transition into this stage finishes.",
            None,
        )
        .child(splash)
        .child(sound)
        .child(notification)
        .child(popups)
        .into_any_element()
    }

    /// Reads the stage's current schedule for `kind`, changes it, and writes it back.
    fn edit_schedule(
        &mut self,
        id: &str,
        kind: EventKind,
        change: impl FnOnce(&mut EventSchedule),
        cx: &mut Context<Self>,
    ) {
        let current = self
            .editor
            .read(cx)
            .session()
            .read(queries::stages)
            .ok()
            .and_then(|stages| stages.into_iter().find(|stage| stage.id == id))
            .and_then(|stage| stage.events.get(kind).cloned());
        let Some(mut schedule) = current else {
            return;
        };
        change(&mut schedule);
        self.apply(cx, |session| {
            session.set_stage_event(id, kind, Some(schedule))
        });
    }

    /// A small number field inline in a row of controls.
    #[allow(clippy::too_many_arguments)]
    fn inline_number(
        &mut self,
        key: String,
        current: Option<f64>,
        placeholder: &'static str,
        apply: impl Fn(&mut Self, Option<f64>, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let key: SharedString = key.into();
        let input = self.field(
            Spec::new(key.clone(), optional_text(current)).placeholder(placeholder),
            move |this, text, cx| {
                if let Ok(value) = parse_number(text) {
                    apply(this, value, cx);
                }
            },
            window,
            cx,
        );
        self.field_box(&key, &input, None, window, cx).w(px(84.))
    }

    fn schedule_editor(
        &mut self,
        stage: &Stage,
        previous: Option<&Stage>,
        (kind, name, default): (EventKind, &'static str, f64),
        seam: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let id = stage.id.clone();
        let schedule = stage.events.get(kind).cloned();
        let previous_note = previous
            .and_then(|previous| previous.events.get(kind))
            .map(|schedule| format!("Previous stage: {}", schedule_text(schedule)).into());
        let toggle_id = id.clone();
        let mut row = div().flex().flex_col().gap(px(9.)).child(toggle_row(
            name,
            previous_note,
            Toggle::new(
                SharedString::from(format!("{id}-event-{kind:?}")),
                schedule.is_some(),
            )
            .on_change(listen_bool(cx, move |this, on: &bool, _, cx| {
                let schedule = on.then_some(EventSchedule {
                    interval: Interval::Fixed { seconds: default },
                    initial_delay_seconds: None,
                    max_concurrent: None,
                });
                this.apply(cx, |session| {
                    session.set_stage_event(&toggle_id, kind, schedule)
                });
            })),
            seam,
        ));
        let Some(schedule) = schedule else {
            return row;
        };

        let label = |text: &'static str| ui::hint(text).text_size(px(11.));
        let interval_kind = match schedule.interval {
            Interval::Fixed { .. } => IntervalKind::Fixed,
            Interval::Random { .. } => IntervalKind::Random,
        };
        let select_id = id.clone();
        let view = cx.entity();
        let mut controls = div()
            .pl_4()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(
                div().w(px(160.)).child(
                    Select::new(
                        SharedString::from(format!("{id}-event-{kind:?}-interval")),
                        interval_kind,
                        vec![
                            SelectOption::new(IntervalKind::Fixed, "Fixed interval"),
                            SelectOption::new(IntervalKind::Random, "Random range"),
                        ],
                    )
                    .on_change(move |picked, _, cx| {
                        let id = select_id.clone();
                        view.update(cx, |this, cx| {
                            this.edit_schedule(
                                &id,
                                kind,
                                |schedule| {
                                    schedule.interval = match picked {
                                        IntervalKind::Fixed => Interval::Fixed { seconds: default },
                                        IntervalKind::Random => Interval::Random {
                                            minimum_seconds: default * 0.75,
                                            maximum_seconds: default * 1.25,
                                        },
                                    }
                                },
                                cx,
                            )
                        })
                    }),
                ),
            );
        let prefix = format!("{id}-event-{kind:?}");
        match schedule.interval {
            Interval::Fixed { seconds } => {
                let edit_id = id.clone();
                let every = self.inline_number(
                    format!("{prefix}-every"),
                    Some(seconds),
                    "",
                    move |this, value, cx| {
                        if let Some(seconds) = value {
                            this.edit_schedule(
                                &edit_id,
                                kind,
                                |schedule| schedule.interval = Interval::Fixed { seconds },
                                cx,
                            );
                        }
                    },
                    window,
                    cx,
                );
                controls = controls
                    .child(label("Every"))
                    .child(every)
                    .child(label("seconds"));
            }
            Interval::Random {
                minimum_seconds,
                maximum_seconds,
            } => {
                let (min_id, max_id) = (id.clone(), id.clone());
                let minimum = self.inline_number(
                    format!("{prefix}-min"),
                    Some(minimum_seconds),
                    "",
                    move |this, value, cx| {
                        if let Some(value) = value {
                            this.edit_schedule(
                                &min_id,
                                kind,
                                |schedule| {
                                    if let Interval::Random {
                                        minimum_seconds, ..
                                    } = &mut schedule.interval
                                    {
                                        *minimum_seconds = value;
                                    }
                                },
                                cx,
                            );
                        }
                    },
                    window,
                    cx,
                );
                let maximum = self.inline_number(
                    format!("{prefix}-max"),
                    Some(maximum_seconds),
                    "",
                    move |this, value, cx| {
                        if let Some(value) = value {
                            this.edit_schedule(
                                &max_id,
                                kind,
                                |schedule| {
                                    if let Interval::Random {
                                        maximum_seconds, ..
                                    } = &mut schedule.interval
                                    {
                                        *maximum_seconds = value;
                                    }
                                },
                                cx,
                            );
                        }
                    },
                    window,
                    cx,
                );
                controls = controls
                    .child(label("Between"))
                    .child(minimum)
                    .child(label("and"))
                    .child(maximum)
                    .child(label("seconds"));
            }
        }
        let (delay_id, active_id) = (id.clone(), id.clone());
        let delay = self.inline_number(
            format!("{prefix}-delay"),
            schedule.initial_delay_seconds,
            "None",
            move |this, value, cx| {
                this.edit_schedule(
                    &delay_id,
                    kind,
                    |schedule| schedule.initial_delay_seconds = value,
                    cx,
                );
            },
            window,
            cx,
        );
        let active = self.inline_number(
            format!("{prefix}-active"),
            schedule.max_concurrent.map(f64::from),
            "No limit",
            move |this, value, cx| {
                let value = value.map(|value| value.round().max(1.0) as u32);
                this.edit_schedule(
                    &active_id,
                    kind,
                    |schedule| schedule.max_concurrent = value,
                    cx,
                );
            },
            window,
            cx,
        );
        controls = controls
            .child(label("Initial delay"))
            .child(delay)
            .child(label("Maximum active"))
            .child(active);
        row = row.child(controls);
        row
    }

    fn events_card(
        &mut self,
        stage: &Stage,
        previous: Option<&Stage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut card = section("Events", "", None);
        for (index, event) in EVENTS.into_iter().enumerate() {
            let mut row = self.schedule_editor(stage, previous, event, true, window, cx);
            if event.0 == EventKind::Prompt && stage.events.prompt.is_some() {
                row = row.child(self.prompt_settings(stage, window, cx));
            }
            let _ = index;
            card = card.child(row);
        }
        card.into_any_element()
    }

    fn prompt_settings(
        &mut self,
        stage: &Stage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = stage.id.clone();
        let prompt = &stage.prompt;
        let deadline_id = id.clone();
        let mut settings = div()
            .pl_4()
            .pt_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(toggle_row(
                "Enforce prompt deadlines",
                None,
                Toggle::new(
                    SharedString::from(format!("{id}-prompt-deadlines")),
                    prompt.timeouts_enabled,
                )
                .on_change(listen_bool(cx, move |this, on: &bool, _, cx| {
                    let on = *on;
                    this.apply(cx, |session| {
                        session.set_stage_prompt_timeouts_enabled(&deadline_id, on)
                    });
                })),
                false,
            ));
        if prompt.timeouts_enabled {
            let multiplier_id = id.clone();
            settings = settings.child(
                self.number_field(
                    "Deadline multiplier",
                    None,
                    Spec::new(
                        format!("{id}-prompt-multiplier"),
                        number_text(prompt.timeout_multiplier),
                    ),
                    Some("x"),
                    move |this, value, cx| {
                        if let Some(value) = value {
                            let value = value.max(0.1);
                            this.apply(cx, |session| {
                                session.set_stage_prompt_timeout_multiplier(&multiplier_id, value)
                            });
                        }
                    },
                    window,
                    cx,
                )
                .w(px(240.)),
            );
        }
        let burst = prompt.popup_burst;
        let burst_id = id.clone();
        settings = settings
            .child(ui::section_title("On wrong answer or expired prompt").text_size(px(13.)));
        settings = settings.child(toggle_row(
            "Spawn extra popups",
            None,
            Toggle::new(
                SharedString::from(format!("{id}-prompt-burst")),
                burst.is_some(),
            )
            .on_change(listen_bool(cx, move |this, on: &bool, _, cx| {
                let count = on.then_some(burst.unwrap_or(5));
                this.apply(cx, |session| {
                    session.set_stage_prompt_popup_burst(&burst_id, count)
                });
            })),
            true,
        ));
        if let Some(count) = burst {
            let count_id = id.clone();
            settings = settings.child(
                self.number_field(
                    "Number of popups",
                    None,
                    Spec::new(format!("{id}-prompt-burst-count"), count.to_string()),
                    None,
                    move |this, value, cx| {
                        if let Some(value) = value {
                            let count = value.floor().max(1.0) as u32;
                            this.apply(cx, |session| {
                                session.set_stage_prompt_popup_burst(&count_id, Some(count))
                            });
                        }
                    },
                    window,
                    cx,
                )
                .w(px(240.)),
            );
        }
        let sound = self.media_slot(
            SlotSpec {
                slot: MediaSlot::StagePromptSound { stage: id.clone() },
                title: "Sound",
                description: None,
                empty_note: "No sound played.".into(),
            },
            prompt.sound,
            window,
            cx,
        );
        settings
            .child(
                div()
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .child(sound),
            )
            .into_any_element()
    }

    fn movement_card(
        &mut self,
        stage: &Stage,
        previous: Option<&Stage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = stage.id.clone();
        let toggle_id = id.clone();
        let toggle = Toggle::new(
            SharedString::from(format!("{id}-movement")),
            stage.movement.is_some(),
        )
        .on_change(listen_bool(cx, move |this, on: &bool, _, cx| {
            let on = *on;
            this.apply(cx, |session| session.set_stage_movement(&toggle_id, on));
        }));
        let mut card = section("Moving windows", "", Some(toggle.into_any_element()));
        if let Some(movement) = &stage.movement {
            let previous_speed = |pick: fn(&shared::behaviour::Movement) -> Option<f64>| {
                let text = previous
                    .and_then(|previous| previous.movement.as_ref())
                    .and_then(pick)
                    .map_or("off".to_string(), number_text);
                Some(format!("Previous: {text}").into())
            };
            let (min_id, max_id) = (id.clone(), id.clone());
            let minimum = self.number_field(
                "Minimum speed",
                previous_speed(|movement| movement.minimum_speed),
                Spec::new(
                    format!("{id}-speed-min"),
                    optional_text(movement.minimum_speed),
                ),
                None,
                move |this, value, cx| {
                    if value.is_some() {
                        this.apply(cx, |session| {
                            session.set_stage_movement_speed(&min_id, value, None)
                        });
                    }
                },
                window,
                cx,
            );
            let maximum = self.number_field(
                "Maximum speed",
                previous_speed(|movement| movement.maximum_speed),
                Spec::new(
                    format!("{id}-speed-max"),
                    optional_text(movement.maximum_speed),
                ),
                None,
                move |this, value, cx| {
                    if value.is_some() {
                        this.apply(cx, |session| {
                            session.set_stage_movement_speed(&max_id, None, value)
                        });
                    }
                },
                window,
                cx,
            );
            card = card.child(field_grid(vec![
                minimum.into_any_element(),
                maximum.into_any_element(),
            ]));
        }
        card.into_any_element()
    }

    fn mitosis_card(
        &mut self,
        stage: &Stage,
        previous: Option<&Stage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = stage.id.clone();
        let toggle_id = id.clone();
        let toggle = Toggle::new(
            SharedString::from(format!("{id}-mitosis")),
            stage.mitosis.is_some(),
        )
        .on_change(listen_bool(cx, move |this, on: &bool, _, cx| {
            let on = *on;
            this.apply(cx, |session| session.set_stage_mitosis(&toggle_id, on));
        }));
        let mut card = section(
            "Mitosis",
            "Closing windows will have a chance of spawning more.",
            Some(toggle.into_any_element()),
        );
        if let Some(mitosis) = &stage.mitosis {
            let previous_mitosis = previous.and_then(|previous| previous.mitosis.as_ref());
            let previous_chance = previous_mitosis
                .and_then(|mitosis| mitosis.chance)
                .map_or("off".to_string(), number_text);
            let previous_count = previous_mitosis
                .and_then(|mitosis| mitosis.count)
                .map_or("off".to_string(), |count| count.to_string());
            let (chance_id, count_id) = (id.clone(), id.clone());
            let chance = self.number_field(
                "Chance (0 to 1)",
                Some(format!("Previous: {previous_chance}").into()),
                Spec::new(
                    format!("{id}-mitosis-chance"),
                    optional_text(mitosis.chance),
                ),
                None,
                move |this, value, cx| {
                    if let Some(value) = value {
                        let value = value.clamp(0.0, 1.0);
                        this.apply(cx, |session| {
                            session.set_stage_mitosis_values(&chance_id, Some(value), None)
                        });
                    }
                },
                window,
                cx,
            );
            let copies = self.number_field(
                "Copies",
                Some(format!("Previous: {previous_count}").into()),
                Spec::new(
                    format!("{id}-mitosis-count"),
                    mitosis
                        .count
                        .map(|count| count.to_string())
                        .unwrap_or_default(),
                ),
                None,
                move |this, value, cx| {
                    if let Some(value) = value {
                        let count = value.round().max(1.0) as u32;
                        this.apply(cx, |session| {
                            session.set_stage_mitosis_values(&count_id, None, Some(count))
                        });
                    }
                },
                window,
                cx,
            );
            card = card.child(field_grid(vec![
                chance.into_any_element(),
                copies.into_any_element(),
            ]));
        }
        card.into_any_element()
    }

    fn duration_card(
        &mut self,
        snapshot: &Snapshot,
        index: usize,
        stage: &Stage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_last = index + 1 == snapshot.stages.len();
        let mut card = section(
            "Stage duration",
            if is_last {
                "The final stage continues until the session ends."
            } else {
                "Choose how long these settings stay active."
            },
            None,
        );
        let Some(end) = &stage.end else {
            return card.into_any_element();
        };
        let id = stage.id.clone();
        let has_event = end.event_count.is_some();

        let minutes_id = id.clone();
        let minutes = self.number_field(
            "Keep these settings for (minutes)",
            (end.duration_seconds == Some(0.0))
                .then(|| "The transition begins as soon as this stage is reached.".into()),
            Spec::new(
                format!("{id}-duration"),
                optional_text(end.duration_seconds.map(|seconds| seconds / 60.0)),
            )
            .placeholder("None"),
            Some("min"),
            move |this, value, cx| {
                // Blank ends the stage on its event condition alone, so it needs one.
                if value.is_none() && !has_event {
                    return;
                }
                let seconds = value.map(|minutes| minutes.max(0.0) * 60.0);
                this.apply(cx, |session| {
                    session.set_stage_end_duration(&minutes_id, seconds)
                });
            },
            window,
            cx,
        );

        let event_id = id.clone();
        let view = cx.entity();
        let mut options = vec![SelectOption::new(None, "No event condition")];
        options.extend(
            EVENTS
                .iter()
                .map(|(kind, name, _)| SelectOption::new(Some(*kind), format!("{name} spawned"))),
        );
        let duration_set = end.duration_seconds.is_some();
        let condition = labelled(
            "Additional condition",
            None,
            Select::new(
                SharedString::from(format!("{id}-end-event")),
                end.event_count.as_ref().map(|condition| condition.event),
                options,
            )
            .on_change(move |picked: Option<EventKind>, _, cx| {
                let id = event_id.clone();
                view.update(cx, |this, cx| {
                    // Dropping the event condition from a stage that has no duration would leave
                    // it with no way to end: give it the default back.
                    if picked.is_none() && !duration_set {
                        this.apply(cx, |session| {
                            session.set_stage_end_duration(&id, Some(ops::DEFAULT_STAGE_SECONDS))
                        });
                    }
                    this.apply(cx, |session| session.set_stage_end_event(&id, picked));
                })
            }),
        );
        let mut fields = vec![minutes.into_any_element(), condition.into_any_element()];
        if let Some(event_count) = &end.event_count {
            let count_id = id.clone();
            fields.push(
                self.number_field(
                    "Event count",
                    Some(
                        format!("{} spawned in this stage.", event_name(event_count.event)).into(),
                    ),
                    Spec::new(format!("{id}-end-count"), event_count.count.to_string()),
                    None,
                    move |this, value, cx| {
                        if let Some(value) = value {
                            let count = value.round().max(1.0) as u32;
                            this.apply(cx, |session| {
                                session.set_stage_end_event_count(&count_id, count)
                            });
                        }
                    },
                    window,
                    cx,
                )
                .into_any_element(),
            );
            let strategy_id = id.clone();
            let view = cx.entity();
            fields.push(
                labelled(
                    "Advance when",
                    None,
                    Select::new(
                        SharedString::from(format!("{id}-end-strategy")),
                        end.strategy,
                        vec![
                            SelectOption::new(EndStrategy::Any, "Either condition is reached"),
                            SelectOption::new(EndStrategy::All, "Both conditions are reached"),
                        ],
                    )
                    .on_change(move |picked, _, cx| {
                        let id = strategy_id.clone();
                        view.update(cx, |this, cx| {
                            this.apply(cx, |session| session.set_stage_end_strategy(&id, picked));
                        })
                    }),
                )
                .into_any_element(),
            );
        }
        card = card.child(field_grid(fields));

        // "Then change to <next>", and the way to the transition between them.
        if let (Some(next), Some(outgoing)) = (
            snapshot.stages.get(index + 1),
            snapshot.transition_after(index),
        ) {
            let summary = if outgoing.duration_seconds == 0.0 {
                "Immediately".to_string()
            } else {
                let minutes = outgoing.duration_seconds / 60.0;
                if minutes.fract() == 0.0 {
                    format!(
                        "Gradually over {minutes} minute{}",
                        if minutes == 1.0 { "" } else { "s" }
                    )
                } else {
                    format!(
                        "Gradually over {} seconds",
                        number_text(outgoing.duration_seconds)
                    )
                }
            };
            let (next_id, transition_id) = (next.id.clone(), outgoing.id.clone());
            card = card.child(
                div()
                    .pt_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_size(px(12.))
                            .text_color(c(theme::MUTED))
                            .child("Then change to")
                            .child(
                                div()
                                    .id(SharedString::from(format!("{id}-next")))
                                    .text_color(c(theme::TEXT))
                                    .underline()
                                    .cursor_pointer()
                                    .child(next.label.clone())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select(next_id.clone(), window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{id}-transition")))
                            .h(px(30.))
                            .px(px(9.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(theme::RADIUS_SM)
                            .bg(c(theme::BG))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                            .child(summary)
                            .child(
                                div()
                                    .text_color(c(theme::MUTED))
                                    .child(gpui_kit::assets::IconName::ArrowRight),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select(transition_id.clone(), window, cx)
                            })),
                    ),
            );
        }
        card.into_any_element()
    }
}
