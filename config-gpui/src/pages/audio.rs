//! Where sound comes out, and how loudly media plays. Ported from `config/src/lib/Audio.svelte`.

use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, Entity, Subscription, Window};
use shared::user_config::{AudioDeviceChoice, Volume};

use crate::backend::hardware;
use crate::runtime;
use crate::store::{FeedbackKind, Store};
use crate::theme::{self, c};
use crate::ui::{self, Button, SelectOption, Size};

/// Label, description, and how to write and read the channel in `Volume`.
type VolumeSpec = (
    &'static str,
    &'static str,
    fn(&mut Volume) -> &mut f32,
    fn(&Volume) -> f32,
);

struct VolumeSlider {
    label: &'static str,
    description: &'static str,
    read: fn(&Volume) -> f32,
    state: Entity<SliderState>,
}

pub struct AudioPage {
    store: Entity<Store>,
    sliders: Vec<VolumeSlider>,
    testing: bool,
    _subscriptions: Vec<Subscription>,
}

impl AudioPage {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Only fetched once this page is shown -- see `Store::audio_devices`.
        store.update(cx, |store, cx| {
            if store.audio_devices.is_none() && store.audio_devices_error.is_none() {
                store.load_audio_devices(cx);
            }
        });

        let volume = store
            .read(cx)
            .config
            .as_ref()
            .map(|c| c.volume.clone())
            .unwrap_or_default();

        let specs: [VolumeSpec; 2] = [
            (
                "Video volume",
                "Master volume for a video popup's embedded audio track.",
                |v| &mut v.video,
                |v| v.video,
            ),
            (
                "Audio volume",
                "Master volume for standalone audio the pack or mode plays.",
                |v| &mut v.audio,
                |v| v.audio,
            ),
        ];

        let mut subscriptions = Vec::new();
        let sliders = specs
            .into_iter()
            .map(|(label, description, channel, read)| {
                let state = cx.new(|_| {
                    SliderState::new()
                        .min(0.)
                        .max(1.)
                        .step(0.01)
                        .default_value(read(&volume))
                });
                let store = store.clone();
                // Dragging only updates the value on screen; the release saves it.
                subscriptions.push(cx.subscribe_in(&state, window, move |_, _, event, _, cx| {
                    let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
                    let SliderValue::Single(value) = *value else {
                        return;
                    };
                    let release = matches!(event, SliderEvent::Release(_));
                    store.update(cx, |store, cx| {
                        store.preview_volume(channel, value, cx);
                        if release {
                            store.save(cx);
                        }
                    });
                }));
                VolumeSlider {
                    label,
                    description,
                    read,
                    state,
                }
            })
            .collect();

        Self {
            store,
            sliders,
            testing: false,
            _subscriptions: subscriptions,
        }
    }

    fn test_audio(&mut self, cx: &mut Context<Self>) {
        let device = self
            .store
            .read(cx)
            .config
            .as_ref()
            .and_then(|c| c.audio_device.as_ref())
            .map(|d| d.id.clone());
        self.testing = true;
        self.store
            .update(cx, |store, cx| store.dismiss_feedback("test-audio", cx));
        cx.notify();

        let store = self.store.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime::run(hardware::test_audio(device))
                .await
                .and_then(|r| r);
            store.update(cx, |store, cx| {
                match result {
                    // Hearing it is the confirmation; success is silent.
                    Ok(result) if !result.fell_back => {}
                    Ok(_) => store.set_feedback(
                        "test-audio",
                        FeedbackKind::Warning,
                        "That output isn't available -- the sound played on the system default"
                            .into(),
                    ),
                    Err(error) => store.set_feedback(
                        "test-audio",
                        FeedbackKind::Error,
                        format!("Test sound failed: {error}"),
                    ),
                }
                cx.notify();
            });
            let _ = this.update(cx, |this, cx| {
                this.testing = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn output_device(&self, cx: &mut Context<Self>) -> AnyElement {
        let store = self.store.read(cx);
        let entity = self.store.clone();

        if let Some(error) = &store.audio_devices_error {
            return ui::card()
                .items_center()
                .gap_2()
                .p_4()
                .child(
                    ui::label("Audio outputs couldn't be detected")
                        .text_size(px(12.))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD),
                )
                .child(ui::hint(error.clone()).max_w(px(448.)))
                .child(
                    Button::new("retry-audio", "Try again")
                        .size(Size::Compact)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |store, cx| store.load_audio_devices(cx))
                        }),
                )
                .into_any_element();
        }

        let devices = store.audio_devices.clone().unwrap_or_default();
        let loading = store.audio_devices_loading;
        let chosen = store.config.as_ref().and_then(|c| c.audio_device.clone());
        let selected = chosen.as_ref().map(|d| d.id.clone());
        // A device saved on another machine, or unplugged since. The engine falls back to the
        // default for that session and keeps the setting, so say so rather than show a blank.
        let missing = chosen
            .as_ref()
            .filter(|d| !loading && !devices.is_empty() && !devices.iter().any(|x| x.id == d.id));

        let mut options = vec![SelectOption::new(None, "System default")];
        // Connected devices are labelled from the live list, so a renamed device shows its
        // current name.
        options.extend(
            devices
                .iter()
                .map(|d| SelectOption::new(Some(d.id.clone()), d.name.clone())),
        );
        if let Some(missing) = missing {
            options.push(
                SelectOption::new(
                    Some(missing.id.clone()),
                    format!("{} (not connected)", missing.name),
                )
                .disabled(true),
            );
        }

        let status: AnyElement = if loading {
            ui::readout("Detecting outputs...").into_any_element()
        } else if let Some(missing) = missing {
            ui::readout(format!(
                "{} isn't connected -- sound falls back to the system default",
                missing.name
            ))
            .text_color(c(theme::WARNING))
            .into_any_element()
        } else if let (None, Some(default)) = (&selected, devices.iter().find(|d| d.is_default)) {
            ui::readout(format!("Currently {}", default.name)).into_any_element()
        } else if !devices.is_empty() {
            let n = devices.len();
            ui::readout(format!(
                "{n} {} available",
                if n == 1 { "output" } else { "outputs" }
            ))
            .into_any_element()
        } else {
            div().into_any_element()
        };

        let on_select = self.store.clone();
        let rescan = self.store.clone();
        ui::card()
            .gap_3()
            .p_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        ui::Select::new("output-device", selected, options)
                            .disabled(loading)
                            .on_change(move |value: Option<String>, _, cx| {
                                on_select.update(cx, |store, cx| {
                                    let name = store
                                        .audio_devices
                                        .iter()
                                        .flatten()
                                        .find(|d| Some(&d.id) == value.as_ref())
                                        .map(|d| d.name.clone());
                                    let choice = value.map(|id| AudioDeviceChoice {
                                        name: name.unwrap_or_else(|| id.clone()),
                                        id,
                                    });
                                    store.set_audio_device(choice, cx)
                                })
                            }),
                    )
                    .child(
                        Button::new("rescan-audio", "Rescan")
                            .disabled(loading)
                            .on_click(move |_, _, cx| {
                                rescan.update(cx, |store, cx| store.load_audio_devices(cx))
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(status)
                    .child(
                        Button::new("test-audio", "Test sound")
                            .size(Size::Compact)
                            .loading(self.testing)
                            .on_click(cx.listener(|this, _, _, cx| this.test_audio(cx))),
                    ),
            )
            .into_any_element()
    }

    fn volume(&self, cx: &mut Context<Self>) -> AnyElement {
        let volume = self
            .store
            .read(cx)
            .config
            .as_ref()
            .map(|c| c.volume.clone())
            .unwrap_or_default();

        div()
            .flex()
            .gap_3()
            .children(self.sliders.iter().map(|slider| {
                let percent = ((slider.read)(&volume) * 100.).round() as i32;
                ui::card()
                    .flex_1()
                    .gap_3()
                    .p_4()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(ui::label(slider.label))
                            .child(
                                ui::readout(format!("{percent}%"))
                                    .px_2()
                                    .py_1()
                                    .rounded(theme::RADIUS_SM)
                                    .bg(c(theme::BG))
                                    .text_color(c(theme::TEXT))
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD),
                            ),
                    )
                    .child(ui::hint(slider.description))
                    .child(Slider::new(&slider.state).accessibility_label(slider.label))
            }))
            .into_any_element()
    }
}

impl Render for AudioPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let section = |title: &'static str, hint: &'static str, body: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(ui::section_title(title))
                .child(ui::hint(hint))
                .child(body)
                .into_any_element()
        };
        let output = self.output_device(cx);
        let volume = self.volume(cx);
        ui::page(
            "audio",
            "Audio",
            "Choose where sound comes out, and how loudly media plays.",
            [
                section(
                    "Output device",
                    "Which output popups and sounds play through. Applies to sounds started from now on; a session already running keeps the device it opened with.",
                    output,
                ),
                section(
                    "Volume",
                    "Master volume, applied on top of whatever volume the pack or mode requests for a track.",
                    volume,
                ),
            ],
        )
    }
}
