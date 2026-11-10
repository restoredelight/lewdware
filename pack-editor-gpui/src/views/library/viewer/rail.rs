//! The options rail beside a popup in the viewer: what the file (or the selection) does as a popup.
//! Ported from `pack-editor/src/lib/PopupOptions.svelte`. Anything left blank is the mode's
//! choice, stored as nothing (see `pack_editor_core::session::popup`).

use gpui_kit::component::input::Input;
use gpui_kit::component::slider::{Slider, SliderValue};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Div, Focusable, FontWeight, SharedString, Window};
use pack_editor_core::session::popup::{describe_region, is_capped, popup_size, REFERENCE_SCREEN};
use shared::behaviour::MonitorPreference;
use shared::encode::FileInfo;

use super::{Field, Viewer, RAIL_W};
use crate::icons::icon;
use crate::theme::{self, c};
use crate::ui::{self, Button, Select, SelectOption, Size};

#[derive(Clone, Copy, PartialEq)]
enum MonitorChoice {
    Mixed,
    Any,
    Primary,
}

impl Viewer {
    pub(super) fn rail(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(file) = self.file(cx) else {
            return div().into_any_element();
        };
        let targets = self.targets();
        let count = targets.len();
        let media = self.editor.read(cx).session().media();
        let any_video = targets.iter().any(|id| {
            media
                .get(*id)
                .is_some_and(|file| matches!(file.file_info, FileInfo::Video { .. }))
        });

        let weight = self.shared(|a| a.weight);
        let scale = self.shared(|a| a.scale);
        let region = self.shared(|a| a.region);
        let monitor = self.shared(|a| a.monitor);
        let looping = self.shared(|a| a.video_loop);
        let audio = self.shared(|a| a.video_audio);
        let level = self.shared(|a| a.video_volume);

        self.input(Field::Weight).update(cx, |input, cx| {
            input.set_placeholder(if weight.mixed { "Mixed" } else { "Equal" }, window, cx)
        });

        // The size as pixels, which an author can picture, rather than as a multiplier. Only for
        // one file whose dimensions are known: across a selection it would be a guess.
        let dims = Self::media_size(&file).map(|(w, h)| (w as f64, h as f64));
        let size_reading: SharedString = match (count, dims) {
            _ if scale.mixed => "Mixed size".into(),
            (1, Some(dims)) => {
                let (w, h) = popup_size(dims, scale.value, REFERENCE_SCREEN);
                let capped = is_capped(dims, scale.value, REFERENCE_SCREEN);
                format!("{w} x {h}{}", if capped { " (max size)" } else { "" }).into()
            }
            _ => match scale.value {
                Some(scale) => format!("{scale}x size").into(),
                None => "Automatic size".into(),
            },
        };
        let region_reading: SharedString = if region.mixed {
            "Mixed area".into()
        } else {
            describe_region(region.value).into()
        };

        let entity = cx.entity();
        let place = entity.clone();
        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Popup settings"),
            );

        let frequency = section()
            .child(rail_label("Spawn frequency (x)"))
            .child(self.number_field(Field::Weight, None, window, cx));

        let placement = section()
            .child(rail_label("Size and position"))
            .child(ui::readout(size_reading).text_color(c(theme::TEXT)))
            .child(ui::readout(region_reading).text_color(c(theme::TEXT)))
            .child(
                Button::new("open-placement", "Size & position...")
                    .size(Size::Compact)
                    .on_click(move |_, _, cx| {
                        place.update(cx, |this, cx| this.set_placing(true, cx))
                    }),
            );

        let monitor_select = {
            let selected = if monitor.mixed {
                MonitorChoice::Mixed
            } else if monitor.value == Some(MonitorPreference::Primary) {
                MonitorChoice::Primary
            } else {
                MonitorChoice::Any
            };
            let mut options = Vec::new();
            if monitor.mixed {
                options.push(SelectOption::new(MonitorChoice::Mixed, "Mixed"));
            }
            options.push(SelectOption::new(MonitorChoice::Any, "Any monitor"));
            options.push(SelectOption::new(MonitorChoice::Primary, "Primary monitor"));
            let set = entity.clone();
            section()
                .child(rail_label("Monitor"))
                .child(Select::new("popup-monitor", selected, options).on_change(
                    move |choice, _, cx| {
                        let monitor = match choice {
                            MonitorChoice::Mixed => return,
                            MonitorChoice::Any => None,
                            MonitorChoice::Primary => Some(MonitorPreference::Primary),
                        };
                        set.update(cx, |this, cx| {
                            let (targets, label) =
                                (this.targets(), this.label("Set popup monitor", cx));
                            this.apply(cx, |s| s.set_popup_monitor(&targets, monitor, &label));
                        })
                    },
                ))
        };

        let video = any_video.then(|| {
            let loop_on = looping.value != Some(false);
            let sound_on = audio.value != Some(false);
            let (set_loop, set_sound) = (entity.clone(), entity.clone());
            // Mixed sits the thumb at full and says so, rather than showing one file's level.
            let shown = self.dragged_volume.unwrap_or(if level.mixed {
                1.
            } else {
                level.value.unwrap_or(1.) as f32
            });
            if self.dragged_volume.is_none() {
                let at = match self.volume.read(cx).value() {
                    SliderValue::Single(value) => value,
                    _ => -1.,
                };
                if (at - shown).abs() > f32::EPSILON {
                    self.volume
                        .update(cx, |slider, cx| slider.set_value(shown, window, cx));
                }
            }
            let reading = if self.dragged_volume.is_none() && level.mixed {
                "Mixed".to_string()
            } else if shown >= 1. {
                "Full".to_string()
            } else {
                format!("{}%", (shown * 100.).round())
            };
            section()
                .child(rail_label("Video"))
                .child(check_row(
                    "popup-loop",
                    loop_on,
                    looping.mixed,
                    "Loop",
                    "Off closes the popup when the clip ends",
                    move |_, _, cx| {
                        set_loop.update(cx, |this, cx| {
                            let (targets, label) =
                                (this.targets(), this.label("Set video looping", cx));
                            let on = looping.mixed || !loop_on;
                            this.apply(cx, |s| s.set_popup_video_loop(&targets, on, &label));
                        })
                    },
                ))
                .child(check_row(
                    "popup-sound",
                    sound_on,
                    audio.mixed,
                    "Play sound",
                    "Still silent if popup sounds are muted",
                    move |_, _, cx| {
                        set_sound.update(cx, |this, cx| {
                            let (targets, label) =
                                (this.targets(), this.label("Set video sound", cx));
                            let on = audio.mixed || !sound_on;
                            this.apply(cx, |s| s.set_popup_video_audio(&targets, on, &label));
                        })
                    },
                ))
                // A level on a silenced clip couldn't do anything.
                .when(sound_on || audio.mixed, |this| {
                    this.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(9.))
                            .text_size(px(11.))
                            .text_color(c(theme::MUTED))
                            .child("Volume")
                            .child(div().flex_1().min_w_0().child(Slider::new(&self.volume)))
                            .child(
                                ui::readout(reading)
                                    .w(px(38.))
                                    .flex_none()
                                    .text_right()
                                    .text_color(c(theme::TEXT)),
                            ),
                    )
                    .child(rail_note(
                        "Levels this clip against the rest of the pack. The user's own volume \
                         still applies.",
                    ))
                })
        });

        let membership = (count == 1).then(|| self.membership.clone()).flatten();

        div()
            .id("popup-rail")
            .occlude()
            .flex_none()
            .w(px(RAIL_W))
            .h_full()
            .overflow_y_scrollbar()
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(18.))
            .border_l_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::BG))
            .text_color(c(theme::TEXT))
            // Room for the close button, which sits over the rail's top corner.
            .pt(px(56.))
            .child(header)
            .child(frequency)
            .child(placement)
            .child(monitor_select)
            .children(video)
            .children(membership)
            .into_any_element()
    }

    /// A numeric field. `suffix` is a unit drawn inside its right edge (the region's `%`);
    /// without one, a value can be cleared with the button there, back to the mode's choice.
    pub(super) fn number_field(
        &self,
        field: Field,
        suffix: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let input = self.input(field).clone();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        let filled = !input.read(cx).value().trim().is_empty();
        let clear = cx.entity();
        let clear_input = input.clone();
        div()
            .h(theme::CONTROL_COMPACT)
            .w_full()
            .flex()
            .items_center()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(if focused {
                theme::FOCUS
            } else {
                theme::BORDER_STRONG
            }))
            .bg(c(theme::BG))
            .child(
                Input::new(&input)
                    .appearance(false)
                    .text_size(px(12.))
                    .flex_1(),
            )
            .children(suffix.map(|suffix| {
                div()
                    .flex_none()
                    .pr_2()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(c(theme::MUTED))
                    .child(suffix)
            }))
            .when(suffix.is_none() && filled, |this| {
                this.child(
                    div()
                        .id(SharedString::from(format!("clear-{field:?}")))
                        .flex_none()
                        .mr_1()
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS_SM)
                        .text_color(c(theme::MUTED))
                        .cursor_pointer()
                        .hover(|style| {
                            style
                                .bg(c(theme::SURFACE_RAISED))
                                .text_color(c(theme::TEXT))
                        })
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new("Clear").build(window, cx)
                        })
                        .child(icon(gpui_kit::assets::IconName::Close).size(px(14.)))
                        .on_click(move |_, window, cx| {
                            clear_input.update(cx, |input, cx| input.set_value("", window, cx));
                            clear.update(cx, |this, cx| this.commit_fields(cx));
                        }),
                )
            })
    }
}

fn section() -> Div {
    div().flex().flex_col().gap_2()
}

fn rail_label(text: &'static str) -> Div {
    div()
        .text_size(px(11.))
        .text_color(c(theme::MUTED))
        .child(text)
}

fn rail_note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .line_height(px(15.))
        .text_color(c(theme::MUTED))
        .child(text.into())
}

/// A checkbox with a title and a note under it. A mixed value shows a dash.
fn check_row(
    id: &'static str,
    checked: bool,
    mixed: bool,
    title: &'static str,
    note: &'static str,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> impl IntoElement {
    let indicator = if mixed {
        div()
            .flex_none()
            .size(px(16.))
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(theme::ACCENT))
            .bg(c(theme::ACCENT))
            .flex()
            .items_center()
            .justify_center()
            .text_color(c(theme::TEXT))
            .child(icon(gpui_kit::assets::IconName::Minus).size(px(12.)))
    } else {
        ui::checkbox(checked)
    };
    div()
        .id(id)
        .flex()
        .items_start()
        .gap(px(9.))
        .cursor_pointer()
        .child(indicator)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(12.)).child(title))
                .child(rail_note(note)),
        )
        .on_click(on_click)
}
