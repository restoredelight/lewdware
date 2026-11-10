//! Starting sessions on their own. Ported from `config/src/lib/Scheduling.svelte`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Datelike, Local, Utc};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, Context, Entity, FocusHandle, FontWeight, SharedString, Subscription,
    Window,
};
use shared::schedule::{
    Days, Frequency, QuietHours, Range, Rule, ScheduleConfig, SessionLength, SessionOverrides,
    TimeOfDay, Trigger,
};
use shared::user_config::Mode;
use uuid::Uuid;

use crate::backend::modes::Source;
use crate::runtime;
use crate::store::Store;
use crate::theme::{self, c};
use crate::ui::{self, Button, DialogAction, SelectOption, Size, Toggle, Variant};

const DAY_LABELS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Attached to the frequency field rather than set out as a panel: it explains one number.
const RATE_EXPLANATION: &str = "Sometimes you will get fewer sessions than this - Lewdware only \
starts them while you're actually at your computer. It learns which hours you're usually around, \
so this gets less common over time.";

/// A rule that does something until edited: every day, once, somewhere in the working afternoon,
/// until stopped. Lewdware is built to take a machine over gradually, so a session far more often
/// ends with the stop shortcut than with a clock running out.
fn default_rule() -> Rule {
    Rule {
        id: Uuid::new_v4(),
        days: [true; 7],
        trigger: Trigger::Rate {
            range: Range::Between {
                from: TimeOfDay::new(9, 0),
                to: TimeOfDay::new(17, 0),
            },
            frequency: Frequency::PerDay { count: 1 },
        },
        length: SessionLength::UntilStopped,
        overrides: None,
    }
}

fn default_quiet_hours() -> QuietHours {
    QuietHours {
        days: [true; 7],
        start: TimeOfDay::new(22, 0),
        end: TimeOfDay::new(7, 0),
    }
}

fn time_text(time: TimeOfDay) -> String {
    format!("{:02}:{:02}", time.hour, time.minute)
}

/// `H:MM` or `HH:MM`, in range. Anything else is rejected rather than guessed at.
fn parse_time(text: &str) -> Option<TimeOfDay> {
    let (hour, minute) = text.trim().split_once(':')?;
    let (hour, minute): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60 && minute < 100).then(|| TimeOfDay::new(hour, minute))
}

fn format_instant(instant: DateTime<Utc>) -> String {
    let local = instant.with_timezone(&Local);
    let today = Local::now().date_naive();
    let time = local.format("%H:%M");
    if local.date_naive() == today {
        format!("today {time}")
    } else if Some(local.date_naive()) == today.succ_opt() {
        format!("tomorrow {time}")
    } else {
        format!(
            "{} {} {} {time}",
            local.format("%a"),
            local.day(),
            local.format("%b")
        )
    }
}

fn day_summary(days: &Days) -> String {
    if days.iter().all(|d| *d) {
        return "Every day".into();
    }
    if days[..5].iter().all(|d| *d) && !days[5] && !days[6] {
        return "Weekdays".into();
    }
    if !days[..5].iter().any(|d| *d) && days[5] && days[6] {
        return "Weekends".into();
    }
    let selected: Vec<&str> = DAY_LABELS
        .iter()
        .zip(days)
        .filter(|(_, on)| **on)
        .map(|(label, _)| *label)
        .collect();
    if selected.is_empty() {
        "No days selected".into()
    } else {
        selected.join(", ")
    }
}

fn frequency_words(frequency: Frequency) -> String {
    let count = frequency.count();
    let times = if count == 1 {
        "once".to_string()
    } else {
        format!("{count} times")
    };
    match frequency {
        Frequency::PerDay { .. } => format!("{times} a day"),
        Frequency::PerWeek { .. } => format!("{times} a week"),
    }
}

/// The ", for N min" suffix of a rule summary; nothing when the session runs until stopped.
fn length_suffix(length: SessionLength) -> String {
    match length {
        SessionLength::UntilStopped => String::new(),
        SessionLength::Fixed { minutes } => format!(", for {minutes} min"),
    }
}

/// The one readable sentence a rule reduces to. "About" isn't hedging: the rate is capped so it
/// can't cram a shortfall into the end of a range, so under-delivery is a real outcome.
fn rule_summary(rule: &Rule) -> String {
    let days = day_summary(&rule.days);
    match rule.trigger {
        Trigger::At { time } => {
            format!(
                "{days} at {}{}.",
                time_text(time),
                length_suffix(rule.length)
            )
        }
        Trigger::Rate { range, frequency } => {
            let when = match range {
                Range::AllDay => "any time of day".to_string(),
                Range::Between { from, to } => {
                    format!("any time between {} and {}", time_text(from), time_text(to))
                }
            };
            format!(
                "{days}, {when}, about {}{}.",
                frequency_words(frequency),
                length_suffix(rule.length)
            )
        }
    }
}

/// A rule that can never fire, said out loud rather than leaving the user to wonder.
fn rule_warning(rule: &Rule) -> Option<&'static str> {
    if !rule.days.iter().any(|d| *d) {
        return Some("Select at least one day for this rule to take effect.");
    }
    if matches!(rule.trigger, Trigger::Rate { frequency, .. } if frequency.count() == 0) {
        return Some("A frequency of zero means this rule never runs.");
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Timing {
    At,
    Between,
    AllDay,
}

fn timing_of(rule: &Rule) -> Timing {
    match rule.trigger {
        Trigger::At { .. } => Timing::At,
        Trigger::Rate {
            range: Range::AllDay,
            ..
        } => Timing::AllDay,
        Trigger::Rate { .. } => Timing::Between,
    }
}

fn range_start(rule: &Rule) -> TimeOfDay {
    match rule.trigger {
        Trigger::At { time } => time,
        Trigger::Rate {
            range: Range::Between { from, .. },
            ..
        } => from,
        _ => TimeOfDay::new(9, 0),
    }
}

fn range_end(rule: &Rule) -> TimeOfDay {
    match rule.trigger {
        Trigger::Rate {
            range: Range::Between { to, .. },
            ..
        } => to,
        _ => TimeOfDay::new(17, 0),
    }
}

/// Switching timing keeps the times and counts already entered, so flipping between modes to
/// compare them doesn't discard work.
fn with_timing(rule: &Rule, timing: Timing) -> Trigger {
    let time = range_start(rule);
    let frequency = match rule.trigger {
        Trigger::Rate { frequency, .. } => frequency,
        Trigger::At { .. } => Frequency::PerDay { count: 1 },
    };
    match timing {
        Timing::At => Trigger::At { time },
        Timing::AllDay => Trigger::Rate {
            range: Range::AllDay,
            frequency,
        },
        Timing::Between => Trigger::Rate {
            range: Range::Between {
                from: time,
                to: range_end(rule),
            },
            frequency,
        },
    }
}

enum Removal {
    Rule(Uuid),
    Quiet(usize),
}

type Commit =
    Box<dyn Fn(&mut SchedulingPage, &str, &mut Context<SchedulingPage>) -> Option<String>>;

pub struct SchedulingPage {
    store: Entity<Store>,
    /// Text fields by key (`<rule id>:<field>`, `quiet<i>:<field>`, `cooldown`). Each commits on
    /// Enter or leaving the field.
    fields: HashMap<String, (Entity<InputState>, [Subscription; 3])>,
    /// The text field with focus, for its border.
    focused_input: Option<gpui_kit::EntityId>,
    /// Rules with their pack/mode overrides open. Closed by default: most rules want the defaults.
    expanded_overrides: HashSet<Uuid>,
    help_open: Option<Uuid>,
    pending_removal: Option<Removal>,
    dialog_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl SchedulingPage {
    pub fn new(store: Entity<Store>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            fields: HashMap::new(),
            focused_input: None,
            expanded_overrides: HashSet::new(),
            help_open: None,
            pending_removal: None,
            dialog_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn schedule<'a>(&self, cx: &'a Context<Self>) -> Option<&'a ScheduleConfig> {
        self.store.read(cx).config.as_ref().map(|c| &c.schedule)
    }

    fn edit(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut ScheduleConfig)) {
        self.store
            .update(cx, |store, cx| store.edit_schedule(cx, edit));
    }

    fn edit_rule(&mut self, id: Uuid, cx: &mut Context<Self>, edit: impl FnOnce(&mut Rule)) {
        self.edit(cx, |schedule| {
            if let Some(rule) = schedule.rules.iter_mut().find(|r| r.id == id) {
                edit(rule);
            }
        });
    }

    /// Fields whose values no longer match the shape they were made for (a rule's timing
    /// changed, a quiet period was removed) are dropped and rebuilt from the config.
    fn forget_fields(&mut self, prefix: &str) {
        self.fields.retain(|key, _| !key.starts_with(prefix));
    }

    /// A text field, created on first use with `initial`. `commit` turns the typed text into a
    /// config change and returns the canonical text to show, or `None` to put back `initial`.
    fn field(
        &mut self,
        key: String,
        initial: String,
        commit: Commit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some((input, _)) = self.fields.get(&key) {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial.clone()));
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let text = input.read(cx).value().to_string();
                    let shown = commit(this, &text, cx).unwrap_or_else(|| initial.clone());
                    if shown != text {
                        input.update(cx, |input, cx| input.set_value(shown, window, cx));
                    }
                }
            },
        );
        let [focus, blur] = ui::track_focused_input(
            &input,
            |this: &mut Self| &mut this.focused_input,
            window,
            cx,
        );
        self.fields
            .insert(key, (input.clone(), [subscription, focus, blur]));
        input
    }

    fn time_field(
        &mut self,
        key: String,
        current: TimeOfDay,
        apply: impl Fn(&mut Self, TimeOfDay, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let input = self.field(
            key,
            time_text(current),
            Box::new(move |this, text, cx| {
                let time = parse_time(text)?;
                apply(this, time, cx);
                Some(time_text(time))
            }),
            window,
            cx,
        );
        self.field_box(&input, None).w(px(84.)).into_any_element()
    }

    // Each argument is a distinct property of the field; a builder would add more than it saves.
    #[allow(clippy::too_many_arguments)]
    fn number_field(
        &mut self,
        key: String,
        current: u32,
        min: u32,
        max: u32,
        suffix: Option<&'static str>,
        apply: impl Fn(&mut Self, u32, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let input = self.field(
            key,
            current.to_string(),
            Box::new(move |this, text, cx| {
                // An empty or unreadable field is mid-edit, not a value: put the old one back.
                let value: u32 = text.trim().parse().ok()?;
                let value = value.clamp(min, max);
                apply(this, value, cx);
                Some(value.to_string())
            }),
            window,
            cx,
        );
        self.field_box(&input, suffix)
    }

    fn day_picker(
        &self,
        id: impl Into<SharedString>,
        days: Days,
        on_pick: impl Fn(&mut Self, usize, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id: SharedString = id.into();
        let on_pick = std::rc::Rc::new(on_pick);
        labelled(
            "Days",
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(DAY_LABELS.iter().enumerate().map(|(index, label)| {
                    let on = days[index];
                    let on_pick = on_pick.clone();
                    // Each day is its own toggle. On: raised surface inside a carmine edge, so
                    // the accent stays in the outline and never fills.
                    div()
                        .id(SharedString::from(format!("{id}-{index}")))
                        .h(theme::CONTROL_COMPACT)
                        .w(px(44.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS_SM)
                        .border_1()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .map(|this| {
                            if on {
                                this.bg(c(theme::SURFACE_RAISED))
                                    .border_color(c(theme::ACCENT_HOVER))
                                    .text_color(c(theme::TEXT))
                            } else {
                                this.bg(c(theme::BG))
                                    .border_color(c(theme::BORDER))
                                    .text_color(c(theme::MUTED))
                                    .hover(|style| {
                                        style
                                            .border_color(c(theme::BORDER_STRONG))
                                            .text_color(c(theme::TEXT))
                                    })
                            }
                        })
                        .child(*label)
                        .on_click(cx.listener(move |this, _, _, cx| on_pick(this, index, cx)))
                }))
                .into_any_element(),
        )
    }

    /// One of a few named options, as a dropdown. Sits at the start of a row so the fields it
    /// governs can follow it on the same line.
    fn choice<T: Copy + PartialEq + 'static>(
        &self,
        id: impl Into<SharedString>,
        options: &[(T, &'static str)],
        current: T,
        on_pick: impl Fn(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = cx.entity();
        div()
            .w(px(176.))
            .flex_none()
            .flex()
            .child(
                ui::Select::new(
                    id.into(),
                    current,
                    options
                        .iter()
                        .map(|&(value, label)| SelectOption::new(value, label))
                        .collect(),
                )
                .on_change(move |value, _, cx| {
                    if value != current {
                        page.update(cx, |this, cx| on_pick(this, value, cx));
                    }
                }),
            )
            .into_any_element()
    }

    fn status_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let enabled = self.schedule(cx).is_some_and(|s| s.enabled);
        let status = store.schedule_status.clone();
        let pending = store.schedule_enable_pending;
        let error = store.schedule_enable_error.clone();

        let line = match &status {
            _ if !enabled => "Disabled. No sessions will start automatically.".to_string(),
            Some(s) if s.cooldown_until.is_some() => {
                format!("Paused until {}", format_instant(s.cooldown_until.unwrap()))
            }
            Some(s) if s.next_exact_session.is_some() => {
                format!(
                    "Next session: {}",
                    format_instant(s.next_exact_session.unwrap())
                )
            }
            Some(s) if s.budget_total > 0 => {
                let mut line = format!(
                    "{} of {} left in this period",
                    s.budget_remaining, s.budget_total
                );
                if let Some(next) = s.next_opportunity {
                    line.push_str(&format!(", not before {}", format_instant(next)));
                }
                line
            }
            _ => "Enabled, but no rule can start a session yet.".to_string(),
        };
        let paused = status.as_ref().is_some_and(|s| s.cooldown_until.is_some());
        let toggle_store = self.store.clone();
        let resume_store = self.store.clone();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                ui::card()
                    .flex_row()
                    .items_center()
                    .gap_4()
                    .p_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Automatic scheduling"),
                            )
                            .child(ui::readout(line)),
                    )
                    .when(paused, |this| {
                        this.child(
                            Button::new("resume", "Resume now")
                                .size(Size::Compact)
                                .on_click(move |_, _, cx| {
                                    resume_store.update(cx, |store, cx| store.resume_schedule(cx))
                                }),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(c(if enabled { theme::TEXT } else { theme::MUTED }))
                            .child(if enabled { "Enabled" } else { "Disabled" }),
                    )
                    // Driven off the saved value, which only changes once autostart registration
                    // has worked: no optimistic flip, nothing to revert.
                    .child(
                        Toggle::new("schedule-enabled", enabled)
                            .disabled(pending)
                            .on_change(move |checked, _, cx| {
                                toggle_store
                                    .update(cx, |store, cx| store.set_schedule_enabled(checked, cx))
                            }),
                    ),
            )
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(theme::RADIUS_MD)
                        .border_1()
                        .border_color(c(theme::DANGER_BORDER))
                        .bg(c(theme::DANGER_BG))
                        .text_color(c(theme::DANGER))
                        .text_size(px(13.))
                        .child(format!("Couldn't update scheduling: {error}")),
                )
            })
            .into_any_element()
    }

    fn rule_card(
        &mut self,
        index: usize,
        rule: &Rule,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = rule.id;
        let timing = timing_of(rule);
        let warning = rule_warning(rule);

        let days = self.day_picker(
            format!("days-{id}"),
            rule.days,
            move |this, day, cx| this.edit_rule(id, cx, |rule| rule.days[day] = !rule.days[day]),
            cx,
        );

        let timing_picker = self.choice(
            format!("timing-{id}"),
            &[
                (Timing::At, "At a set time"),
                (Timing::Between, "Between times"),
                (Timing::AllDay, "Any time of day"),
            ],
            timing,
            move |this, timing, cx| {
                this.forget_fields(&id.to_string());
                this.edit_rule(id, cx, |rule| rule.trigger = with_timing(rule, timing));
            },
            cx,
        );

        let times: Option<AnyElement> = match timing {
            Timing::AllDay => None,
            Timing::At => Some(self.time_field(
                format!("{id}:at"),
                range_start(rule),
                move |this, time, cx| {
                    this.edit_rule(id, cx, |rule| rule.trigger = Trigger::At { time })
                },
                window,
                cx,
            )),
            Timing::Between => {
                let from = self.time_field(
                    format!("{id}:from"),
                    range_start(rule),
                    move |this, from, cx| {
                        this.edit_rule(id, cx, |rule| {
                            if let Trigger::Rate { range, .. } = &mut rule.trigger {
                                let to = match *range {
                                    Range::Between { to, .. } => to,
                                    Range::AllDay => TimeOfDay::new(17, 0),
                                };
                                *range = Range::Between { from, to };
                            }
                        })
                    },
                    window,
                    cx,
                );
                let to = self.time_field(
                    format!("{id}:to"),
                    range_end(rule),
                    move |this, to, cx| {
                        this.edit_rule(id, cx, |rule| {
                            if let Trigger::Rate { range, .. } = &mut rule.trigger {
                                let from = match *range {
                                    Range::Between { from, .. } => from,
                                    Range::AllDay => TimeOfDay::new(9, 0),
                                };
                                *range = Range::Between { from, to };
                            }
                        })
                    },
                    window,
                    cx,
                );
                Some(time_range(from, to))
            }
        };

        let frequency: Option<AnyElement> = match rule.trigger {
            Trigger::Rate { frequency, .. } => {
                let count = self
                    .number_field(
                        format!("{id}:count"),
                        frequency.count(),
                        1,
                        99,
                        None,
                        move |this, count, cx| {
                            this.edit_rule(id, cx, |rule| {
                                if let Trigger::Rate { frequency, .. } = &mut rule.trigger {
                                    *frequency = match *frequency {
                                        Frequency::PerDay { .. } => Frequency::PerDay { count },
                                        Frequency::PerWeek { .. } => Frequency::PerWeek { count },
                                    };
                                }
                            })
                        },
                        window,
                        cx,
                    )
                    .w(px(96.));
                let per_week = matches!(frequency, Frequency::PerWeek { .. });
                let kind_store = cx.entity();
                let kind = div().w(px(160.)).flex().child(
                    ui::Select::new(
                        SharedString::from(format!("per-{id}")),
                        per_week,
                        vec![
                            SelectOption::new(false, "times a day"),
                            SelectOption::new(true, "times a week"),
                        ],
                    )
                    .on_change(move |per_week, _, cx| {
                        kind_store.update(cx, |this, cx| {
                            this.edit_rule(id, cx, |rule| {
                                if let Trigger::Rate { frequency, .. } = &mut rule.trigger {
                                    let count = frequency.count();
                                    *frequency = if per_week {
                                        Frequency::PerWeek { count }
                                    } else {
                                        Frequency::PerDay { count }
                                    };
                                }
                            })
                        })
                    }),
                );
                let help_open = self.help_open == Some(id);
                let help = div()
                    .relative()
                    .child(
                        div()
                            .id(SharedString::from(format!("help-{id}")))
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .border_1()
                            .border_color(c(if help_open {
                                theme::FOCUS
                            } else {
                                theme::BORDER_STRONG
                            }))
                            .text_size(px(10.))
                            .text_color(c(theme::MUTED))
                            .cursor_pointer()
                            .hover(|style| style.text_color(c(theme::TEXT)))
                            .child("?")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.help_open = if this.help_open == Some(id) {
                                    None
                                } else {
                                    Some(id)
                                };
                                cx.notify();
                            })),
                    )
                    .when(help_open, |this| {
                        let close = cx.entity();
                        this.child(ui::dismiss_layer("help-backdrop", window, move |_, cx| {
                            close.update(cx, |this, cx| {
                                this.help_open = None;
                                cx.notify();
                            })
                        }))
                        .child(ui::floating(
                            px(24.),
                            false,
                            ui::popover_surface()
                                .w(px(320.))
                                .p_3()
                                .child(ui::hint(RATE_EXPLANATION)),
                        ))
                    });
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(small_label("How often"))
                                .child(help),
                        )
                        .child(div().flex().items_center().gap_2().child(count).child(kind))
                        .into_any_element(),
                )
            }
            Trigger::At { .. } => None,
        };

        let fixed = matches!(rule.length, SessionLength::Fixed { .. });
        let length_picker = self.choice(
            format!("length-{id}"),
            &[(false, "Until I stop it"), (true, "For a set time")],
            fixed,
            move |this, fixed, cx| {
                this.forget_fields(&format!("{id}:minutes"));
                this.edit_rule(id, cx, |rule| {
                    rule.length = if fixed {
                        SessionLength::Fixed { minutes: 20 }
                    } else {
                        SessionLength::UntilStopped
                    }
                })
            },
            cx,
        );
        let minutes: Option<AnyElement> = match rule.length {
            SessionLength::Fixed { minutes } => Some(
                self.number_field(
                    format!("{id}:minutes"),
                    minutes,
                    1,
                    1440,
                    Some("min"),
                    move |this, minutes, cx| {
                        this.edit_rule(id, cx, |rule| {
                            rule.length = SessionLength::Fixed { minutes }
                        })
                    },
                    window,
                    cx,
                )
                .w(px(96.))
                .into_any_element(),
            ),
            SessionLength::UntilStopped => None,
        };

        let overrides = self.overrides(rule, cx);

        ui::card()
            .gap_4()
            .p_4()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Rule {}", index + 1)),
                            )
                            .child(ui::readout(rule_summary(rule))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("remove-rule-{id}")), "Remove")
                            .variant(Variant::Destructive)
                            .size(Size::Compact)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pending_removal = Some(Removal::Rule(id));
                                window.focus(&this.dialog_focus, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(days)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(small_label("When"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(timing_picker)
                            .children(times),
                    )
                    .child(ui::hint(if timing == Timing::At {
                        "Starts at exactly this time, whether or not you're at your computer."
                    } else {
                        "Tries to choose a random time while you're at your computer."
                    })),
            )
            .children(frequency)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(small_label("Session length"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(length_picker)
                            .children(minutes),
                    )
                    .when(fixed, |this| {
                        this.child(ui::hint(
                            "Ends on its own after this long, or sooner if you stop it or quiet hours begin.",
                        ))
                    }),
            )
            .child(overrides)
            .when_some(warning, |this, warning| {
                this.child(div().text_size(px(12.)).text_color(c(theme::WARNING)).child(warning))
            })
            .into_any_element()
    }

    fn overrides(&self, rule: &Rule, cx: &mut Context<Self>) -> AnyElement {
        let id = rule.id;
        let expanded = self.expanded_overrides.contains(&id);
        let overrides = rule.overrides.clone().unwrap_or_default();
        let store = self.store.read(cx);
        // Pack-embedded modes are left out: their id is a row in that pack, so pairing one with
        // a different pack is meaningless. Built-in and uploaded modes work with any pack.
        let modes: Vec<(Mode, String)> = store
            .mode_choices()
            .into_iter()
            .filter(|choice| choice.source != Source::Pack)
            .map(|choice| (choice.mode, choice.name))
            .collect();
        let mode_label = overrides
            .mode
            .as_ref()
            .and_then(|mode| {
                modes
                    .iter()
                    .find(|(m, _)| m == mode)
                    .map(|(_, n)| n.clone())
            })
            .unwrap_or_else(|| "Default mode".into());
        let pack_label = overrides
            .pack_path
            .as_ref()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.display().to_string())
            })
            .unwrap_or_else(|| "Default pack".into());
        let summary = if overrides.mode.is_none() && overrides.pack_path.is_none() {
            "Uses the default pack and mode".to_string()
        } else {
            format!("{pack_label}, {mode_label}")
        };

        let mut options = vec![SelectOption::new(None, "Default mode")];
        options.extend(
            modes
                .into_iter()
                .map(|(mode, name)| SelectOption::new(Some(mode), name)),
        );
        let this = cx.entity();

        div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .id(SharedString::from(format!("overrides-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .self_start()
                    .cursor_pointer()
                    .text_color(c(theme::MUTED))
                    .hover(|style| style.text_color(c(theme::TEXT)))
                    .child(ui::readout(summary))
                    .child(div().text_size(px(12.)).child(if expanded {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded_overrides.remove(&id) {
                            this.expanded_overrides.insert(id);
                        }
                        cx.notify();
                    })),
            )
            .when(expanded, |this_div| {
                let pick = cx.entity();
                let reset = cx.entity();
                this_div.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_4()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(small_label("Pack"))
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            Button::new(
                                                SharedString::from(format!("rule-pack-{id}")),
                                                "Choose pack...",
                                            )
                                            .size(Size::Compact)
                                            .on_click(
                                                move |_, _, cx| {
                                                    pick.update(cx, |this, cx| {
                                                        this.pick_rule_pack(id, cx)
                                                    })
                                                },
                                            ),
                                        )
                                        .when(overrides.pack_path.is_some(), |this| {
                                            this.child(
                                                Button::new(
                                                    SharedString::from(format!(
                                                        "rule-pack-reset-{id}"
                                                    )),
                                                    "Use default",
                                                )
                                                .variant(Variant::Quiet)
                                                .size(Size::Compact)
                                                .on_click(move |_, _, cx| {
                                                    reset.update(cx, |this, cx| {
                                                        this.set_overrides(id, cx, |o| {
                                                            o.pack_path = None
                                                        })
                                                    })
                                                }),
                                            )
                                        }),
                                )
                                .child(ui::readout(pack_label).truncate()),
                        )
                        .child(labelled(
                            "Mode",
                            div()
                                .w(px(240.))
                                .flex()
                                .child(
                                    ui::Select::new(
                                        SharedString::from(format!("rule-mode-{id}")),
                                        overrides.mode.clone(),
                                        options,
                                    )
                                    .on_change(
                                        move |mode, _, cx| {
                                            this.update(cx, |this, cx| {
                                                this.set_overrides(id, cx, |o| o.mode = mode)
                                            })
                                        },
                                    ),
                                )
                                .into_any_element(),
                        )),
                )
            })
            .into_any_element()
    }

    /// Stores a rule's overrides, as nothing at all when both are back to the defaults.
    fn set_overrides(
        &mut self,
        id: Uuid,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut SessionOverrides),
    ) {
        self.edit_rule(id, cx, |rule| {
            let mut overrides = rule.overrides.clone().unwrap_or_default();
            edit(&mut overrides);
            rule.overrides =
                (overrides.mode.is_some() || overrides.pack_path.is_some()).then_some(overrides);
        });
    }

    /// Only a path: an override names a pack for the supervisor to hand the engine later. This
    /// app never loads it.
    fn pick_rule_pack(&mut self, id: Uuid, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let picked: Option<PathBuf> = runtime::run(async {
                rfd::AsyncFileDialog::new()
                    .set_title("Choose a pack for this rule")
                    .add_filter("Pack", &["lwpack"])
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            })
            .await
            .ok()
            .flatten();
            if let Some(path) = picked {
                let _ = this.update(cx, |this, cx| {
                    this.set_overrides(id, cx, |o| o.pack_path = Some(path))
                });
            }
        })
        .detach();
    }

    fn quiet_card(
        &mut self,
        index: usize,
        quiet: &QuietHours,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let prefix = format!("quiet{index}:");
        let days = self.day_picker(
            format!("quiet-days-{index}"),
            quiet.days,
            move |this, day, cx| {
                this.edit(cx, |schedule| {
                    if let Some(quiet) = schedule.quiet_hours.get_mut(index) {
                        quiet.days[day] = !quiet.days[day];
                    }
                })
            },
            cx,
        );
        let start = self.time_field(
            format!("{prefix}start"),
            quiet.start,
            move |this, start, cx| {
                this.edit(cx, |schedule| {
                    if let Some(quiet) = schedule.quiet_hours.get_mut(index) {
                        quiet.start = start;
                    }
                })
            },
            window,
            cx,
        );
        let end = self.time_field(
            format!("{prefix}end"),
            quiet.end,
            move |this, end, cx| {
                this.edit(cx, |schedule| {
                    if let Some(quiet) = schedule.quiet_hours.get_mut(index) {
                        quiet.end = end;
                    }
                })
            },
            window,
            cx,
        );
        let warning = if !quiet.days.iter().any(|d| *d) {
            Some("Select at least one day for this quiet period to take effect.")
        } else if quiet.start == quiet.end {
            Some("Start and end are the same, so this quiet period has no effect.")
        } else {
            None
        };

        ui::card()
            .gap_4()
            .p_4()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("Quiet period {}", index + 1)),
                            )
                            .child(ui::readout(format!(
                                "{}, {}–{}",
                                day_summary(&quiet.days),
                                time_text(quiet.start),
                                time_text(quiet.end)
                            ))),
                    )
                    .child(
                        Button::new(("remove-quiet", index), "Remove")
                            .variant(Variant::Destructive)
                            .size(Size::Compact)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pending_removal = Some(Removal::Quiet(index));
                                window.focus(&this.dialog_focus, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(days)
            .child(labelled("Hours", time_range(start, end)))
            .when_some(warning, |this, warning| {
                this.child(
                    div()
                        .text_size(px(12.))
                        .text_color(c(theme::WARNING))
                        .child(warning),
                )
            })
            .into_any_element()
    }

    fn section(
        title: &'static str,
        hint: &'static str,
        action: Option<AnyElement>,
        body: Vec<AnyElement>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .pt_6()
            .border_t_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(ui::section_title(title))
                            .when(!hint.is_empty(), |this| this.child(ui::hint(hint))),
                    )
                    .children(action.map(|action| div().flex_none().child(action))),
            )
            .children(body)
            .into_any_element()
    }

    fn removal_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let removal = self.pending_removal.as_ref()?;
        let (title, description) = match removal {
            Removal::Rule(_) => (
                "Remove rule?",
                "Sessions will no longer start from this rule.",
            ),
            Removal::Quiet(_) => (
                "Remove quiet period?",
                "Scheduled activity will no longer be blocked during this quiet period.",
            ),
        };
        let cancel = cx.entity();
        let confirm = cx.entity();
        let close = cx.entity();
        Some(
            ui::dialog(
                title,
                description,
                vec![
                    DialogAction::new("Cancel", Variant::Secondary, move |_, _, cx| {
                        cancel.update(cx, |this, cx| {
                            this.pending_removal = None;
                            cx.notify();
                        })
                    }),
                    DialogAction::new("Remove", Variant::Destructive, move |_, _, cx| {
                        confirm.update(cx, |this, cx| {
                            match this.pending_removal.take() {
                                Some(Removal::Rule(id)) => {
                                    this.edit(cx, |s| s.rules.retain(|r| r.id != id))
                                }
                                Some(Removal::Quiet(index)) => {
                                    // Later periods shift down a place, so their fields are
                                    // rebuilt from the config.
                                    this.forget_fields("quiet");
                                    this.edit(cx, |s| {
                                        if index < s.quiet_hours.len() {
                                            s.quiet_hours.remove(index);
                                        }
                                    })
                                }
                                None => {}
                            }
                            cx.notify();
                        })
                    }),
                ],
                move |_, cx| {
                    close.update(cx, |this, cx| {
                        this.pending_removal = None;
                        cx.notify();
                    })
                },
                &self.dialog_focus,
                window,
            )
            .into_any_element(),
        )
    }
}

fn small_label(text: &'static str) -> gpui_kit::Div {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(theme::TEXT))
        .child(text)
}

fn labelled(label: &'static str, control: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(small_label(label))
        .child(control)
        .into_any_element()
}

/// Two times on one line: "09:00 to 17:00".
fn time_range(from: AnyElement, to: AnyElement) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(from)
        .child(
            div()
                .text_size(px(13.))
                .text_color(c(theme::MUTED))
                .child("to"),
        )
        .child(to)
        .into_any_element()
}

/// Matches `ui::Select`'s height and outline, so a field can sit on the same line as one.
impl SchedulingPage {
    fn field_box(&self, input: &Entity<InputState>, suffix: Option<&'static str>) -> gpui_kit::Div {
        div()
            .h(theme::CONTROL_NORMAL)
            // The input pads itself by 10px and scrolls its text to keep 10px clear of the caret, so
            // extra padding here starves it: "09:00" scrolled out of view on arrow keys.
            .pl(px(2.))
            .pr(px(if suffix.is_some() { 10. } else { 2. }))
            .flex()
            .items_center()
            .gap_1()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(ui::field_border(
                self.focused_input == Some(input.entity_id()),
            ))
            .bg(c(theme::BG))
            .child(
                Input::new(input)
                    .appearance(false)
                    .flex_1()
                    .text_size(px(13.))
                    .font_family(theme::FONT_MONO),
            )
            .when_some(suffix, |this, suffix| this.child(ui::readout(suffix)))
    }
}

impl Render for SchedulingPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(schedule) = self.schedule(cx).cloned() else {
            return div().into_any_element();
        };
        let mut sections = vec![self.status_card(cx)];

        if schedule.enabled {
            let mut rules: Vec<AnyElement> = Vec::new();
            for (index, rule) in schedule.rules.iter().enumerate() {
                rules.push(self.rule_card(index, rule, window, cx));
            }
            let add_rule =
                |id: &'static str, cx: &mut Context<Self>| {
                    Button::new(id, "Add rule")
                        .size(Size::Compact)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.edit(cx, |s| s.rules.push(default_rule()))
                        }))
                        .into_any_element()
                };
            if rules.is_empty() {
                rules.push(
                    empty_state(
                        "No rules",
                        "Scheduling is enabled, but no sessions can start until you add a rule.",
                    )
                    .child(add_rule("add-rule-empty", cx))
                    .into_any_element(),
                );
            }
            sections.push(Self::section(
                "Rules",
                "",
                Some(add_rule("add-rule", cx)),
                rules,
            ));

            let mut quiet: Vec<AnyElement> = Vec::new();
            for (index, period) in schedule.quiet_hours.iter().enumerate() {
                quiet.push(self.quiet_card(index, period, window, cx));
            }
            if quiet.is_empty() {
                quiet.push(
                    empty_state(
                        "",
                        "No quiet hours. Scheduled sessions may run whenever a rule allows.",
                    )
                    .into_any_element(),
                );
            }
            sections.push(Self::section(
                "Quiet hours",
                "Never start during these times, and stop a scheduled session that is already running. Sessions you launch yourself are unaffected.",
                Some(
                    Button::new("add-quiet", "Add quiet hours")
                        .size(Size::Compact)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.edit(cx, |s| s.quiet_hours.push(default_quiet_hours()))
                        }))
                        .into_any_element(),
                ),
                quiet,
            ));

            let cooldown = self
                .number_field(
                    "cooldown".into(),
                    schedule.cooldown_minutes,
                    1,
                    1440,
                    Some("min"),
                    |this, minutes, cx| this.edit(cx, |s| s.cooldown_minutes = minutes),
                    window,
                    cx,
                )
                .w(px(112.));

            let grace = Toggle::new("grace", schedule.grace_notification).on_change({
                let this = cx.entity();
                move |checked, _, cx| {
                    this.update(cx, |this, cx| {
                        this.edit(cx, |s| s.grace_notification = checked)
                    })
                }
            });
            sections.push(Self::section(
                "Timing",
                "",
                None,
                vec![
                    row_card(
                        "Gap before the next session",
                        "Cancelling a grace notification starts the same gap.",
                        cooldown.into_any_element(),
                    ),
                    row_card(
                        "Warn before starting",
                        "Cancelling skips that session and starts the gap above.",
                        grace.into_any_element(),
                    ),
                ],
            ));
        } else {
            sections.push(
                empty_state(
                    "Scheduling is turned off",
                    "Enable automatic scheduling to have Lewdware start at specific times.",
                )
                .into_any_element(),
            );
        }

        let dialog = self.removal_dialog(window, cx);
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(ui::page("scheduling", "Scheduling", "", sections))
            .children(dialog)
            .into_any_element()
    }
}

fn empty_state(title: &'static str, text: &'static str) -> gpui_kit::Div {
    div()
        .p_6()
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .rounded(theme::RADIUS_MD)
        .border_1()
        .border_dashed()
        .border_color(c(theme::BORDER_STRONG))
        .when(!title.is_empty(), |this| {
            this.child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
        })
        .child(ui::hint(text).max_w(px(448.)))
}

fn row_card(label: &'static str, description: &'static str, control: AnyElement) -> AnyElement {
    ui::card()
        .flex_row()
        .items_center()
        .gap_4()
        .p_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(ui::label(label))
                .child(ui::hint(description)),
        )
        .child(control)
        .into_any_element()
}
