use dioxus::{
    desktop::{use_window, wry::dpi::PhysicalSize},
    prelude::*,
};
use dioxus_primitives::checkbox::CheckboxState;
use shared::{
    components::{checkbox::Checkbox, label::Label},
    user_config::{AppConfigStoreExt, Key},
};

use crate::Config;

#[component]
pub fn General() -> Element {
    rsx! {
        div { class: "flex-1 p-10 flex flex-col gap-8 overflow-y-auto",
            PanicKeyInput {  }
            MonitorPicker {  }
        }
    }
}

#[component]
pub fn PanicKeyInput() -> Element {
    let config = use_context::<Config>().0;

    let mut recording = use_signal(|| false);

    let display_text = match (recording(), &*config.panic_button().read()) {
        (true, _) => "Press a key…".to_string(),
        (false, kb) => kb.to_string(),
    };

    let capture_classes = if recording() {
        "bg-blue-50 border-2 border-blue-500 text-blue-700 italic"
    } else {
        "bg-gray-50 border-2 border-gray-300 text-gray-900 hover:border-gray-400"
    };

    rsx! {
        div {
            class: "inline-flex flex-col gap-1 font-sans",

            label {
                class: "text-sm font-semibold text-gray-700",
                "Panic key"
            }

            div {
                tabindex: 0,
                class: "px-4 py-2 rounded-md cursor-pointer min-w-40 text-center text-sm \
                        outline-none select-none transition-all duration-150 {capture_classes}",

                onclick: move |_| {
                    recording.set(true);
                },

                onkeydown: move |evt| {
                    if !recording() { return; }

                    let key_name = evt.key().to_string();
                    if is_modifier_key(&key_name) { return; }

                    evt.prevent_default();

                    let key_modifiers = evt.modifiers();
                    let modifiers = shared::user_config::Modifiers {
                        ctrl:  key_modifiers.ctrl(),
                        alt:   key_modifiers.alt(),
                        shift: key_modifiers.shift(),
                        meta:  key_modifiers.meta(),
                    };

                    let key = Key {
                        name: if key_name == " " { "Space".to_string() } else { key_name },
                        code: evt.code().to_string(),
                        modifiers,
                    };

                    config.panic_button().set(key);
                    recording.set(false);
                },

                onblur: move |_| {
                    recording.set(false);
                },

                "{display_text}"
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct Monitor {
    name: String,
    size: PhysicalSize<u32>,
}

#[component]
pub fn MonitorPicker() -> Element {
    let window = use_window();

    let monitors = use_memo(move || {
        window
            .available_monitors()
            .filter_map(|monitor| {
                monitor.name().map(|name| Monitor {
                    name,
                    size: monitor.size(),
                })
            })
            .collect::<Vec<_>>()
    });

    rsx! {
        div {
            class: "flex flex-col gap-1",
            Label {
                html_for: "",
                "Monitors"
            }

            div {
                class: "flex flex-col",
                for monitor in monitors.read().iter() {
                    MonitorOption { monitor: monitor.clone() }
                }
            }
        }
    }
}

#[component]
fn MonitorOption(monitor: ReadSignal<Monitor>) -> Element {
    let config = use_context::<Config>().0;

    let enabled = use_memo(move || {
        !config
            .disabled_monitors()
            .read()
            .contains(&monitor.read().name)
    });

    let set_enabled = move |val: bool| {
        println!("{}", monitor.read().name);
        if val {
            config
                .disabled_monitors()
                .retain(|name| name != &monitor.read().name);
        } else if enabled() {
            config.disabled_monitors().push(monitor.read().name.clone());
        }
    };

    rsx! {
        div {
            class: "rounded-sm hover:bg-sky-200 p-1 flex items-center gap-2",
            onclick: move |_| {
                set_enabled(!enabled());
                println!("Click");
            },
            Checkbox {
                checked: if enabled() { CheckboxState::Checked } else { CheckboxState::Unchecked },
            }
            "{monitor.read().name} ({monitor.read().size.width}x{monitor.read().size.height})"
        }
    }
}

fn is_modifier_key(key: &str) -> bool {
    matches!(
        key,
        "Control" | "Alt" | "Shift" | "Meta" | "Super" | "Hyper"
    )
}
