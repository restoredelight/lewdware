//! Which pack, which mode, and that mode's options. Ported from `config/src/lib/PackMode.svelte`.
//!
//! The option tree is resolved from the store synchronously on every render, so what's shown is
//! always the selected mode's -- the webview version had to save the new mode before it could ask
//! for its options, and showed the previous mode's in between.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderScale, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, Context, Entity, FocusHandle, FontWeight, SharedString, Subscription,
    Window,
};
use indexmap::IndexMap;
use shared::mode::{
    ConditionValue, ModeEntry, ModeGroup, ModeOption, OptionType, OptionValue, Permission,
    ShowWhen, StoredValue,
};
use shared::user_config::Mode;

use crate::backend::modes::{same_mode, Source};
use crate::store::{Store, Tab};
use crate::theme::{self, c};
use crate::ui::{self, Button, DialogAction, SelectOption, Size, Toggle, Variant};

enum Removal {
    Pack,
    Mode { path: PathBuf, name: String },
}

/// The widgets an option keeps between renders: a text field, and for a slider option the slider.
struct Controls {
    input: Option<Entity<InputState>>,
    slider: Option<Entity<SliderState>>,
    _subscriptions: Vec<Subscription>,
}

/// What a numeric option allows, read off its type.
#[derive(Clone, Copy)]
struct NumberSpec {
    integer: bool,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    clamp: bool,
    slider: bool,
}

impl NumberSpec {
    fn of(option_type: &OptionType) -> Option<Self> {
        match *option_type {
            OptionType::Integer {
                min,
                max,
                step,
                clamp,
                slider,
                ..
            } => Some(Self {
                integer: true,
                min: min.map(|v| v as f64),
                max: max.map(|v| v as f64),
                step: step.map(|v| v as f64),
                clamp,
                slider,
            }),
            OptionType::Number {
                min,
                max,
                step,
                clamp,
                slider,
                ..
            } => Some(Self {
                integer: false,
                min,
                max,
                step,
                clamp,
                slider,
            }),
            _ => None,
        }
    }

    /// How finely the controls move. A declared step wins; otherwise the largest power of ten
    /// that cuts the range into 100-odd positions, never coarser than 1. Without this a 0-1
    /// option's slider could only reach its endpoints.
    ///
    /// Only governs the controls: typed values are held to a step the mode actually declared.
    fn ui_step(&self) -> f64 {
        if let Some(step) = self.step {
            return step;
        }
        if self.integer {
            return 1.;
        }
        let range = self.max.unwrap_or(100.) - self.min.unwrap_or(0.);
        if range <= 0. {
            return 1.;
        }
        10f64.powf((range / 100.).log10().floor()).min(1.)
    }

    fn clamp(&self, value: f64) -> f64 {
        if !self.clamp {
            return value;
        }
        let value = self.min.map_or(value, |min| value.max(min));
        self.max.map_or(value, |max| value.min(max))
    }

    /// The value as it's stored: stepped, clamped, and as the option's own number type.
    fn stored(&self, value: f64, step: Option<f64>) -> StoredValue {
        let stepped = step.map_or(value, |step| round_to_step(value, step));
        let clamped = self.clamp(stepped);
        if self.integer {
            StoredValue::Int(clamped.round() as i64)
        } else {
            StoredValue::Float(clamped)
        }
    }
}

/// Rounds to a step and to the step's decimal places, which keeps float noise
/// (0.30000000000000004) out of the stored config.
fn round_to_step(value: f64, step: f64) -> f64 {
    if step <= 0. {
        return value;
    }
    let snapped = (value / step).round() * step;
    let decimals = (-step.log10().floor()).max(0.) as i32;
    let factor = 10f64.powi(decimals);
    (snapped * factor).round() / factor
}

fn number_of(value: &OptionValue) -> Option<f64> {
    match *value {
        OptionValue::Integer(i) => Some(i as f64),
        OptionValue::Number(n) => Some(n),
        _ => None,
    }
}

fn format_number(value: f64) -> String {
    format!("{value}")
}

/// The value an optional option is switched back on with when nothing earlier is remembered.
fn initial_value(option: &ModeOption) -> StoredValue {
    match &option.option_type {
        OptionType::Integer { default, .. } => StoredValue::Int(*default),
        OptionType::Number { default, .. } => StoredValue::Float(*default),
        OptionType::String { default } => StoredValue::Str(default.clone()),
        OptionType::Boolean { default } => StoredValue::Bool(*default),
        OptionType::Enum { default, .. } => StoredValue::Str(default.clone()),
    }
}

fn stored_from(value: &OptionValue) -> StoredValue {
    match value {
        OptionValue::Integer(i) => StoredValue::Int(*i),
        OptionValue::Number(n) => StoredValue::Float(*n),
        OptionValue::String(s) | OptionValue::Enum(s) => StoredValue::Str(s.clone()),
        OptionValue::Boolean(b) => StoredValue::Bool(*b),
        OptionValue::Null => StoredValue::Null,
    }
}

fn permission_label(permission: Permission) -> &'static str {
    match permission {
        Permission::SetWallpaper => "Change wallpaper",
        Permission::OpenLinks => "Open links",
        Permission::SendNotifications => "Show notifications",
    }
}

fn permission_sentence(permissions: &[Permission]) -> String {
    let names: Vec<String> = permissions
        .iter()
        .map(|p| format!("\"{}\"", permission_label(*p)))
        .collect();
    match names.as_slice() {
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => String::new(),
    }
}

/// Everything a render needs to decide what's visible and what's allowed.
struct View {
    values: HashMap<String, OptionValue>,
    granted: Box<dyn Fn(Permission) -> bool>,
}

impl View {
    fn visible(&self, show_when: &Option<ShowWhen>) -> bool {
        show_when.as_ref().is_none_or(|conditions| {
            conditions
                .iter()
                .all(|(key, expected): (&String, &ConditionValue)| {
                    self.values
                        .get(key)
                        .is_some_and(|actual| expected.matches(actual))
                })
        })
    }

    /// A group is no more visible than its contents: a heading over nothing to configure would
    /// otherwise appear whenever `show_when` sits on the options rather than the group.
    fn has_visible_content(&self, entry: &ModeEntry) -> bool {
        match entry {
            ModeEntry::Option(_) => true,
            ModeEntry::Group(group) => group
                .entries
                .values()
                .any(|child| self.visible(show_when_of(child)) && self.has_visible_content(child)),
        }
    }

    fn unmet(&self, permissions: &[Permission]) -> Vec<Permission> {
        permissions
            .iter()
            .copied()
            .filter(|p| !(self.granted)(*p))
            .collect()
    }
}

fn show_when_of(entry: &ModeEntry) -> &Option<ShowWhen> {
    match entry {
        ModeEntry::Option(option) => &option.show_when,
        ModeEntry::Group(group) => &group.show_when,
    }
}

pub struct PackModePage {
    store: Entity<Store>,
    pending_removal: Option<Removal>,
    dialog_focus: FocusHandle,
    /// Groups the user has collapsed. Groups start open.
    collapsed: HashSet<String>,
    controls: HashMap<String, Controls>,
    /// The mode `controls` were built for. Keys can repeat across modes, so a mode change
    /// discards them.
    controls_mode: Option<Mode>,
    /// The last value an optional option had before it was switched off, to restore it with.
    last_values: HashMap<String, StoredValue>,
    _subscriptions: Vec<Subscription>,
}

impl PackModePage {
    pub fn new(store: Entity<Store>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&store, |_, _, cx| cx.notify())];
        Self {
            store,
            pending_removal: None,
            dialog_focus: cx.focus_handle(),
            collapsed: HashSet::new(),
            controls: HashMap::new(),
            controls_mode: None,
            last_values: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    fn set_option(&mut self, key: String, value: StoredValue, cx: &mut Context<Self>) {
        self.store
            .update(cx, |store, cx| store.set_mode_option(key, value, cx));
    }

    /// Makes sure every text and number option has its widgets, for the mode now selected.
    fn ensure_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.store.read(cx).config.as_ref().map(|c| c.mode.clone());
        if self.controls_mode != mode {
            self.controls.clear();
            self.controls_mode = mode;
        }

        let mut options = Vec::new();
        fn collect(entries: &IndexMap<String, ModeEntry>, out: &mut Vec<(String, ModeOption)>) {
            for (key, entry) in entries {
                match entry {
                    ModeEntry::Option(option) => out.push((key.clone(), option.clone())),
                    ModeEntry::Group(group) => collect(&group.entries, out),
                }
            }
        }
        collect(&self.store.read(cx).schema.entries, &mut options);
        let values = self.store.read(cx).option_values();

        for (key, option) in options {
            if self.controls.contains_key(&key) {
                continue;
            }
            let value = values.get(&key).cloned().unwrap_or(OptionValue::Null);
            let controls = match &option.option_type {
                OptionType::String { .. } => {
                    let text = match &value {
                        OptionValue::String(s) => s.clone(),
                        _ => String::new(),
                    };
                    let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
                    let option_key = key.clone();
                    let subscription =
                        cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                            if matches!(event, InputEvent::Change) {
                                let text = input.read(cx).value().to_string();
                                this.set_option(option_key.clone(), StoredValue::Str(text), cx);
                            }
                        });
                    Controls {
                        input: Some(input),
                        slider: None,
                        _subscriptions: vec![subscription],
                    }
                }
                option_type => {
                    let Some(spec) = NumberSpec::of(option_type) else {
                        continue;
                    };
                    let current = number_of(&value).or_else(|| match initial_value(&option) {
                        StoredValue::Int(i) => Some(i as f64),
                        StoredValue::Float(f) => Some(f),
                        _ => None,
                    });
                    let input = cx.new(|cx| {
                        InputState::new(window, cx)
                            .default_value(current.map(format_number).unwrap_or_default())
                    });
                    let mut subscriptions = Vec::new();
                    let option_key = key.clone();
                    // Typed values are committed on Enter or leaving the field, held to the
                    // mode's declared step only.
                    subscriptions.push(cx.subscribe_in(
                        &input,
                        window,
                        move |this, input, event: &InputEvent, window, cx| {
                            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                                let text = input.read(cx).value().trim().to_string();
                                if let Ok(number) = text.parse::<f64>() {
                                    let stored = spec.stored(number, spec.step);
                                    this.set_option(option_key.clone(), stored, cx);
                                }
                                this.sync_number(&option_key, window, cx);
                            }
                        },
                    ));

                    let slider = spec.slider.then(|| {
                        let min = spec.min.unwrap_or(0.);
                        let max = spec.max.unwrap_or(100.);
                        let scale = if option.logarithmic && min > 0. {
                            SliderScale::Logarithmic
                        } else {
                            SliderScale::Linear
                        };
                        let state = cx.new(|_| {
                            SliderState::new()
                                .min(min as f32)
                                .max(max as f32)
                                .step(spec.ui_step() as f32)
                                .scale(scale)
                                .default_value(current.unwrap_or(min) as f32)
                        });
                        let option_key = key.clone();
                        let field = input.clone();
                        // Dragging previews in the field; the release saves, stepped to the
                        // slider's own step.
                        subscriptions.push(cx.subscribe_in(
                            &state,
                            window,
                            move |this, _, event: &SliderEvent, window, cx| {
                                let (SliderEvent::Change(value) | SliderEvent::Release(value)) =
                                    event;
                                let SliderValue::Single(value) = *value else {
                                    return;
                                };
                                let step = spec.ui_step();
                                let preview = spec.clamp(round_to_step(value as f64, step));
                                field.update(cx, |field, cx| {
                                    field.set_value(format_number(preview), window, cx)
                                });
                                if matches!(event, SliderEvent::Release(_)) {
                                    let stored = spec.stored(value as f64, Some(step));
                                    this.set_option(option_key.clone(), stored, cx);
                                }
                            },
                        ));
                        state
                    });

                    Controls {
                        input: Some(input),
                        slider,
                        _subscriptions: subscriptions,
                    }
                }
            };
            self.controls.insert(key, controls);
        }
    }

    /// Puts a number option's settled value back into its field and slider, so a value that was
    /// stepped or clamped shows what was actually kept.
    fn sync_number(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(value) = self
            .store
            .read(cx)
            .option_values()
            .get(key)
            .and_then(number_of)
        else {
            return;
        };
        if let Some(controls) = self.controls.get(key) {
            if let Some(input) = &controls.input {
                input.update(cx, |input, cx| {
                    input.set_value(format_number(value), window, cx)
                });
            }
            if let Some(slider) = &controls.slider {
                slider.update(cx, |slider, cx| slider.set_value(value as f32, window, cx));
            }
        }
    }

    fn set_optional(
        &mut self,
        key: String,
        option: &ModeOption,
        enabled: bool,
        current: &OptionValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if enabled {
            let restored = self
                .last_values
                .get(&key)
                .cloned()
                .unwrap_or_else(|| initial_value(option));
            self.set_option(key.clone(), restored, cx);
            self.sync_number(&key, window, cx);
        } else {
            if *current != OptionValue::Null {
                self.last_values.insert(key.clone(), stored_from(current));
            }
            self.set_option(key, StoredValue::Null, cx);
        }
    }

    fn permission_hint(&self, permissions: &[Permission], cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.clone();
        // Deliberately quiet: it states a fact and offers a route, it doesn't sound an alarm.
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .text_size(px(11.))
            .text_color(c(theme::MUTED))
            .child(format!(
                "Requires the {} {} ·",
                permission_sentence(permissions),
                if permissions.len() > 1 {
                    "permissions"
                } else {
                    "permission"
                }
            ))
            .child(
                div()
                    .id(SharedString::from(format!("to-safety-{}", cx.entity_id())))
                    .underline()
                    .cursor_pointer()
                    .hover(|style| style.text_color(c(theme::TEXT)))
                    .child("Safety")
                    .on_click(move |_, _, cx| {
                        store.update(cx, |store, cx| store.set_tab(Tab::Safety, cx))
                    }),
            )
            .into_any_element()
    }

    fn control(
        &self,
        key: &str,
        option: &ModeOption,
        value: &OptionValue,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let this = cx.entity();
        match &option.option_type {
            OptionType::Boolean { .. } => {
                let key = key.to_string();
                Toggle::new(
                    SharedString::from(format!("bool-{key}")),
                    *value == OptionValue::Boolean(true),
                )
                .on_change(move |checked, _, cx| {
                    this.update(cx, |this, cx| {
                        this.set_option(key.clone(), StoredValue::Bool(checked), cx)
                    })
                })
                .into_any_element()
            }
            OptionType::String { .. } => match self.controls.get(key).and_then(|c| c.input.clone())
            {
                Some(input) => field_box(Input::new(&input).appearance(false).text_size(px(13.)))
                    .w(px(224.))
                    .into_any_element(),
                None => div().into_any_element(),
            },
            OptionType::Enum { values, .. } => {
                let key = key.to_string();
                let selected = match value {
                    OptionValue::Enum(s) => s.clone(),
                    _ => String::new(),
                };
                let options = values
                    .iter()
                    .map(|(id, member)| SelectOption::new(id.clone(), member.label.clone()))
                    .collect();
                div()
                    .w(px(240.))
                    .flex()
                    .child(
                        ui::Select::new(
                            SharedString::from(format!("enum-{key}")),
                            selected,
                            options,
                        )
                        .on_change(move |choice: String, _, cx| {
                            this.update(cx, |this, cx| {
                                this.set_option(key.clone(), StoredValue::Str(choice), cx)
                            })
                        }),
                    )
                    .into_any_element()
            }
            _ => {
                let Some(controls) = self.controls.get(key) else {
                    return div().into_any_element();
                };
                let field = controls.input.clone().map(|input| {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            field_box(Input::new(&input).appearance(false).text_size(px(13.)))
                                .w(px(96.)),
                        )
                        .when_some(option.suffix.clone(), |this, suffix| {
                            this.child(ui::readout(suffix).text_size(px(12.)))
                        })
                });
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when_some(controls.slider.clone(), |this, slider| {
                        this.child(
                            div().w(px(240.)).child(
                                Slider::new(&slider).accessibility_label(option.label.clone()),
                            ),
                        )
                    })
                    .children(field)
                    .into_any_element()
            }
        }
    }

    fn option_row(
        &self,
        key: &str,
        option: &ModeOption,
        view: &View,
        inherited_disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = view.values.get(key).cloned().unwrap_or(OptionValue::Null);
        let optional_off = option.optional && value == OptionValue::Null;
        let own = view.unmet(&option.needs_permissions);
        let disabled = inherited_disabled || !own.is_empty();
        let is_bool = matches!(option.option_type, OptionType::Boolean { .. });

        let controls = if is_bool && !option.optional {
            self.control(key, option, &value, cx)
        } else {
            let this = cx.entity();
            let toggle_key = key.to_string();
            let toggle_option = option.clone();
            let toggle_value = value.clone();
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                // A switched-off optional shows no control: the toggle alone says "off".
                .when(!optional_off, |this| {
                    this.child(self.control(key, option, &value, cx))
                })
                .when(option.optional, |row| {
                    row.child(
                        Toggle::new(SharedString::from(format!("optional-{key}")), !optional_off)
                            .on_change(move |enabled, window, cx| {
                                this.update(cx, |this, cx| {
                                    this.set_optional(
                                        toggle_key.clone(),
                                        &toggle_option,
                                        enabled,
                                        &toggle_value,
                                        window,
                                        cx,
                                    )
                                })
                            }),
                    )
                })
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px_4()
            .py_3()
            // The hint is for the option's own requirement; a group's is shown once on the group.
            .when(!own.is_empty(), |this| {
                this.child(self.permission_hint(&own, cx))
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_x_6()
                    .gap_y_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(208.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(ui::label(option.label.clone()))
                            .when_some(option.description.clone(), |this, description| {
                                this.child(ui::hint(description))
                            }),
                    )
                    .child(controls)
                    .when(disabled, |this| {
                        // Greyed out and inert: an option that can't take effect until a
                        // permission is granted shouldn't look settable.
                        this.opacity(0.5)
                            .child(div().absolute().inset_0().occlude())
                    }),
            )
            .into_any_element()
    }

    fn group(
        &self,
        key: &str,
        group: &ModeGroup,
        view: &View,
        inherited_disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.collapsed.contains(key);
        let unmet = view.unmet(&group.needs_permissions);
        let children_disabled = inherited_disabled || !unmet.is_empty();
        let toggle_key = key.to_string();

        ui::card()
            .child(
                div()
                    .id(SharedString::from(format!("group-{key}")))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .rounded(theme::RADIUS_MD)
                    .cursor_pointer()
                    .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.collapsed.remove(&toggle_key) {
                            this.collapsed.insert(toggle_key.clone());
                        }
                        cx.notify();
                    }))
                    .child(div().text_size(px(14.)).text_color(c(theme::MUTED)).child(
                        if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        },
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(group.label.clone()),
                            )
                            .when_some(group.description.clone(), |this, description| {
                                this.child(ui::hint(description))
                            }),
                    ),
            )
            // Shown even while collapsed, so the state is legible without expanding.
            .when(!unmet.is_empty(), |this| {
                this.child(div().px_4().pb_2().child(self.permission_hint(&unmet, cx)))
            })
            .when(!collapsed, |this| {
                this.child(
                    div()
                        .border_t_1()
                        .border_color(c(theme::BORDER))
                        .children(self.entries(&group.entries, view, true, children_disabled, cx)),
                )
            })
            .into_any_element()
    }

    /// Consecutive options share one card of rows; each group gets its own. Inside a group
    /// (`bare`) the rows sit directly in the group's card.
    fn entries(
        &self,
        entries: &IndexMap<String, ModeEntry>,
        view: &View,
        bare: bool,
        inherited_disabled: bool,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        let mut rows: Vec<AnyElement> = Vec::new();
        let flush = |rows: &mut Vec<AnyElement>, out: &mut Vec<AnyElement>| {
            if rows.is_empty() {
                return;
            }
            let mut container = if bare {
                div().flex().flex_col()
            } else {
                ui::card()
            };
            for (index, row) in rows.drain(..).enumerate() {
                if index > 0 {
                    container = container.child(ui::seam());
                }
                container = container.child(row);
            }
            out.push(container.into_any_element());
        };

        let visible = entries.iter().filter(|(_, entry)| {
            view.visible(show_when_of(entry)) && view.has_visible_content(entry)
        });
        for (index, (key, entry)) in visible.enumerate() {
            match entry {
                ModeEntry::Option(option) => {
                    rows.push(self.option_row(key, option, view, inherited_disabled, cx))
                }
                ModeEntry::Group(group) => {
                    flush(&mut rows, &mut out);
                    let card = self.group(key, group, view, inherited_disabled, cx);
                    out.push(if bare {
                        div()
                            .p_3()
                            .when(index > 0, |this| {
                                this.border_t_1().border_color(c(theme::BORDER))
                            })
                            .child(card)
                            .into_any_element()
                    } else {
                        card
                    });
                }
            }
        }
        flush(&mut rows, &mut out);
        out
    }

    fn pack_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let busy = store.pack_busy;
        let pack_path = store.config.as_ref().and_then(|c| c.pack_path.clone());
        let entity = self.store.clone();

        let body = match pack_path {
            Some(path) => {
                let metadata = store.pack.as_ref().map(|p| &p.metadata);
                let name = metadata
                    .map(|m| m.name.clone())
                    .filter(|n| !n.is_empty())
                    .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_default();
                let byline = [
                    metadata
                        .and_then(|m| m.creator.clone())
                        .map(|c| format!("By {c}")),
                    metadata
                        .and_then(|m| m.version.clone())
                        .map(|v| format!("Version {v}")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                let description = metadata.and_then(|m| m.description.clone());
                let picker = entity.clone();

                ui::card()
                    .flex_row()
                    .items_start()
                    .gap_4()
                    .p_4()
                    .child(
                        div()
                            .size(px(40.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(theme::RADIUS_MD)
                            .bg(c(theme::SURFACE_RAISED))
                            .text_size(px(20.))
                            .text_color(c(theme::MUTED))
                            .child(IconName::FolderOpen),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .when_some(description, |this, d| this.child(ui::hint(d)))
                            .when(!byline.is_empty(), |this| this.child(ui::readout(byline)))
                            .child(ui::readout(path.display().to_string()).truncate()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .self_center()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("remove-pack", "Remove")
                                    .variant(Variant::Destructive)
                                    .size(Size::Compact)
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.pending_removal = Some(Removal::Pack);
                                        window.focus(&this.dialog_focus, cx);
                                        cx.notify();
                                    })),
                            )
                            // Not primary: Launch holds the carmine fill while a pack is chosen.
                            .child(
                                Button::new("change-pack", "Change pack...")
                                    .size(Size::Compact)
                                    .disabled(busy)
                                    .on_click(move |_, _, cx| {
                                        picker.update(cx, |store, cx| store.pick_pack(cx))
                                    }),
                            ),
                    )
                    .into_any_element()
            }
            None => div()
                .p_6()
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .rounded(theme::RADIUS_MD)
                .border_1()
                .border_dashed()
                .border_color(c(theme::BORDER_STRONG))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("No pack selected"),
                )
                .child(ui::hint("Choose a .lwpack file before launching Lewdware.").mb_3())
                // Launch is disabled without a pack, so this is the live next step.
                .child(
                    Button::new("choose-pack", "Choose pack...")
                        .variant(Variant::Primary)
                        .size(Size::Compact)
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |store, cx| store.pick_pack(cx))
                        }),
                )
                .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(ui::section_title("Pack"))
                    .child(ui::hint("Contains media, captions, prompts, etc.")),
            )
            .child(body)
            .into_any_element()
    }

    fn mode_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let current = store.config.as_ref().map(|c| c.mode.clone());
        let busy = store.mode_busy;
        let choices = store.mode_choices();
        let uploader = self.store.clone();

        let rows = choices.into_iter().enumerate().map(|(index, choice)| {
            let selected = current.as_ref().is_some_and(|m| same_mode(m, &choice.mode));
            let (badge_border, badge_bg, badge_text) = match choice.source {
                Source::Pack => (theme::BORDER_STRONG, theme::SURFACE_RAISED, theme::MUTED),
                Source::Uploaded => (theme::WARNING_BORDER, theme::WARNING_BG, theme::WARNING),
                Source::Builtin => (theme::BORDER, theme::BG, theme::MUTED),
            };
            let entity = self.store.clone();
            let mode = choice.mode.clone();
            let removable = match &choice.mode {
                Mode::File { path } => Some((path.clone(), choice.name.clone())),
                _ => None,
            };

            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .id(("mode", index))
                        .flex_1()
                        .min_w_0()
                        .min_h(px(40.))
                        .flex()
                        .items_start()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded(theme::RADIUS_MD)
                        .cursor_pointer()
                        .map(|this| {
                            // Selected: raised surface plus a carmine edge, text stays white.
                            if selected {
                                this.bg(c(theme::SURFACE_RAISED))
                                    .border_l_2()
                                    .border_color(c(theme::ACCENT_HOVER))
                            } else {
                                this.hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                            }
                        })
                        .when(!selected, |this| {
                            this.on_click(move |_, _, cx| {
                                entity.update(cx, |store, cx| store.set_mode(mode.clone(), cx))
                            })
                        })
                        .child(
                            div()
                                .mt(px(2.))
                                .size(px(16.))
                                .flex_none()
                                .rounded_full()
                                .border_1()
                                .border_color(c(if selected {
                                    theme::ACCENT
                                } else {
                                    theme::BORDER_STRONG
                                }))
                                .bg(c(if selected {
                                    theme::ACCENT
                                } else {
                                    theme::SURFACE
                                }))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(10.))
                                .text_color(c(theme::TEXT))
                                .when(selected, |this| this.child(IconName::Check)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(13.))
                                        .when(selected, |this| this.font_weight(FontWeight::MEDIUM))
                                        .child(choice.name.clone()),
                                )
                                .when_some(choice.description.clone(), |this, d| {
                                    this.child(ui::hint(d).line_clamp(2))
                                }),
                        )
                        .child(
                            div()
                                .flex_none()
                                .px_2()
                                .py(px(1.))
                                .rounded_full()
                                .border_1()
                                .border_color(c(badge_border))
                                .bg(c(badge_bg))
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(c(badge_text))
                                .child(choice.source.label()),
                        ),
                )
                .when_some(removable, |this, (path, name)| {
                    this.child(
                        div()
                            .id(("remove-mode", index))
                            .size(px(32.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(theme::RADIUS_MD)
                            .text_size(px(14.))
                            .text_color(c(theme::DANGER))
                            .cursor_pointer()
                            .hover(|style| style.bg(c(theme::DANGER_BG)))
                            .child(IconName::Close)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.pending_removal = Some(Removal::Mode {
                                    path: path.clone(),
                                    name: name.clone(),
                                });
                                window.focus(&this.dialog_focus, cx);
                                cx.notify();
                            })),
                    )
                })
        });

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
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(ui::section_title("Mode"))
                            .child(ui::hint("Changes the behaviour of Lewdware.")),
                    )
                    .child(
                        Button::new("upload-mode", "Upload mode")
                            .size(Size::Compact)
                            .disabled(busy)
                            .on_click(move |_, _, cx| {
                                uploader.update(cx, |store, cx| store.upload_mode(cx))
                            }),
                    ),
            )
            .child(
                ui::card().child(
                    div()
                        .id("mode-list")
                        .max_h(px(384.))
                        .overflow_y_scroll()
                        .p_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(rows),
                ),
            )
            .into_any_element()
    }

    fn options_section(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let store = self.store.read(cx);
        if store.schema.entries.is_empty() {
            return None;
        }
        let permissions = store.config.as_ref().map(|c| c.permissions.clone());
        let view = View {
            values: store.option_values(),
            granted: Box::new(move |permission| {
                permissions.as_ref().is_some_and(|p| p.allows(permission))
            }),
        };
        let entries = store.schema.entries.clone();
        // Permissions the mode uses however it's configured belong to no one option.
        let mode_unmet = view.unmet(&store.schema.needs_permissions);

        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_6()
                .border_t_1()
                .border_color(c(theme::BORDER))
                .child(ui::section_title("Mode options"))
                .when(!mode_unmet.is_empty(), |this| {
                    this.child(self.permission_hint(&mode_unmet, cx))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .children(self.entries(&entries, &view, false, false, cx)),
                )
                .into_any_element(),
        )
    }

    fn removal_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let removal = self.pending_removal.as_ref()?;
        let (title, description) = match removal {
            Removal::Pack => (
                "Remove pack?".to_string(),
                "Lewdware cannot launch until another pack is selected. Your pack file will not be deleted.",
            ),
            Removal::Mode { name, .. } => (
                format!("Remove \"{name}\"?"),
                "This removes the uploaded mode from Lewdware. If it is selected, Lewdware will switch to a built-in mode.",
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
                            let removal = this.pending_removal.take();
                            let store = this.store.clone();
                            match removal {
                                Some(Removal::Pack) => {
                                    store.update(cx, |store, cx| store.remove_pack(cx))
                                }
                                Some(Removal::Mode { path, .. }) => store
                                    .update(cx, |store, cx| store.remove_uploaded_mode(path, cx)),
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

/// A text field's frame: the app's input look around an unstyled gpui-component input.
fn field_box(input: Input) -> gpui_kit::Div {
    div()
        .h(theme::CONTROL_COMPACT)
        .px_2()
        .flex()
        .items_center()
        .rounded(theme::RADIUS_SM)
        .border_1()
        .border_color(c(theme::BORDER))
        .bg(c(theme::BG))
        .child(input.flex_1())
}

impl Render for PackModePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_controls(window, cx);
        let mut sections = vec![self.pack_section(cx), self.mode_section(cx)];
        sections.extend(self.options_section(cx));
        let dialog = self.removal_dialog(window, cx);
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(ui::page(
                "pack-mode",
                "Pack & mode",
                "Choose the pack Lewdware uses, then select and configure how it behaves.",
                sections,
            ))
            .children(dialog)
    }
}
