use std::collections::HashMap;

use dioxus::{core::Task, prelude::*};
use dioxus_heroicons::{solid::Shape, Icon};
use dioxus_primitives::{slider::SliderValue, ContentSide};
use indexmap::IndexMap;
use shared::{
    components::{
        input::Input,
        label::Label,
        select::{
            Select, SelectItemIndicator, SelectList, SelectOption, SelectTrigger, SelectValue,
        },
        separator::Separator,
        tooltip::{Tooltip, TooltipContent, TooltipTrigger},
    },
};
use shared::{
    components::{
        slider::{Slider, SliderRange, SliderThumb, SliderTrack},
        switch::{Switch, SwitchThumb},
    },
    mode::{OptionType, OptionValue},
};

use crate::{modes::Modes, Config, MediaPack, Pack};
use shared::user_config::{AppConfig, AppConfigStoreExt};

#[component]
pub fn PackModeSettings() -> Element {
    rsx! {
        div { class: "flex-1 p-10 flex flex-col gap-8 overflow-y-auto",
            h1 { class: "text-3xl font-bold", "Pack & Mode Settings" }

            Section { title: "Media Pack",
                PackPicker {}
            }

            Section { title: "Mode",
                ModeSelector { }
            }

            ModeOptions { }
        }
    }
}

#[component]
fn Section(title: String, children: Element) -> Element {
    rsx! {
        div { class: "flex flex-col gap-4",
            h2 { class: "text-xl font-semibold", {title} }
            Separator {}
            {children}
        }
    }
}

#[component]
pub fn PackPicker() -> Element {
    let config: Store<AppConfig> = use_context::<Config>().0;
    let mut pack = use_context::<Pack>().0;
    let mut task: Signal<Option<Task>> = use_signal(|| None);

    let current_path = config
        .pack_path()
        .read()
        .as_ref()
        .and_then(|p| p.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "No pack selected".to_string());

    rsx! {
        div { class: "flex flex-col gap-2",
            Label { html_for: "pack-input", "Current Pack" }
            div { class: "flex gap-2 items-center",
                div { class: "flex-1 p-2 bg-gray-100 rounded border text-sm truncate", {current_path} }
                input {
                    class: "hidden",
                    id: "pack-input",
                    r#type: "file",
                    accept: ".lwpack",
                    onchange: move |event: FormEvent| async move {
                        if let Some(file) = event.files().first() {
                            let path = file.path();
                            if let Some(task) = task.take() {
                                task.cancel();
                            }
                            let path_clone = path.clone();
                            match tokio::task::spawn_blocking(move || MediaPack::open(path_clone)).await {
                                Ok(Ok(p)) => {
                                    *config.pack_path().write() = Some(path);
                                    *pack.write() = Some(p);
                                },
                                Ok(Err(err)) => {
                                    eprintln!("{err}");
                                }
                                Err(err) => {
                                    eprintln!("{err}");
                                }
                            }
                        }
                    },
                }
                label {
                    class: "cursor-pointer bg-blue-600 text-white px-4 py-2 rounded hover:bg-blue-700 transition-colors",
                    r#for: "pack-input",
                    "Browse"
                }
            }
        }
    }
}

// #[derive(Clone, PartialEq)]
// struct ModeSelectGroup {
//     label: String,
//     items: Vec<ModeSelectItem>,
// }
//
#[derive(Clone, PartialEq)]
struct ModeSelectItem {
    // index: usize,
    value: shared::user_config::Mode,
    name: String,
}

// #[component]
// fn ModeSelector() -> Element {
//     let pack = use_context::<Pack>().0;
//     let config = use_context::<Config>().0;
//     let modes = use_context::<Modes>();
//
//     let select_items = use_memo(move || {
//         println!("Rendering items");
//         let mut groups = Vec::new();
//         let mut index = 0;
//
//         if let Some(pack) = &*pack.read() {
//             for mode_file in modes.pack_modes.read().iter() {
//                 let mut items = Vec::new();
//                 let label = if modes.pack_modes.read().len() == 1 {
//                     pack.metadata().name.clone()
//                 } else {
//                     format!("{} ({})", mode_file.metadata.name, pack.metadata().name)
//                 };
//
//                 for (key, mode) in mode_file.metadata.modes.iter() {
//                     items.push(ModeSelectItem {
//                         index,
//                         value: shared::user_config::Mode::Pack {
//                             id: mode_file.id,
//                             mode: key.clone(),
//                         },
//                         name: mode.name.clone(),
//                     });
//
//                     index += 1;
//                 }
//
//                 groups.push(ModeSelectGroup { label, items });
//             }
//         }
//
//         for mode_file in modes.uploaded_modes.read().iter() {
//             let mut items = Vec::new();
//             let label = mode_file.metadata.name.clone();
//
//             for (key, mode) in mode_file.metadata.modes.iter() {
//                 items.push(ModeSelectItem {
//                     index,
//                     value: shared::user_config::Mode::File {
//                         path: mode_file.path.clone(),
//                         mode: key.clone(),
//                     },
//                     name: mode.name.clone(),
//                 });
//
//                 index += 1;
//             }
//
//             groups.push(ModeSelectGroup { label, items });
//         }
//
//         let mode_file = modes.default_mode.read();
//         let mut items = Vec::new();
//         let label = mode_file.name.clone();
//
//         for (key, mode) in mode_file.modes.iter() {
//             items.push(ModeSelectItem {
//                 index,
//                 value: shared::user_config::Mode::Default(key.clone()),
//                 name: mode.name.clone(),
//             });
//
//             index += 1;
//         }
//
//         groups.push(ModeSelectGroup { label, items });
//
//         groups
//     });
//
//     let select_options = select_items.iter().map(|group| {
//         let select_items = group.items.iter().map(|item| {
//             rsx! {
//                 SelectOption::<shared::user_config::Mode> {
//                     index: item.index,
//                     value:  item.value.clone(),
//                     text_value: "{item.name}",
//                     "{item.name}"
//                     SelectItemIndicator {}
//                 }
//             }
//         });
//
//         rsx! {
//             SelectGroup {
//                 SelectGroupLabel {
//                     "{group.label}"
//                 }
//                 {select_items}
//             }
//         }
//     });
//
//     rsx! {
//         div { class: "flex flex-col gap-2",
//             Label { html_for: "mode-select", "Active Mode" }
//
//             Select::<shared::user_config::Mode> {
//                 placeholder: "",
//                 id: "mode-select",
//                 value: Some(Some(config.mode().read().clone())),
//                 on_value_change: move |new_mode| {
//                     println!("Mode changed");
//                     if let Some(mode) = new_mode {
//                         *config.mode().write() = mode;
//                     }
//                 },
//                 SelectTrigger {
//                     SelectValue {  }
//                 }
//                 SelectList {
//                     {select_options}
//                 }
//             }
//         }
//     }
// }

#[component]
fn ModeSelector() -> Element {
    let mut modes = use_context::<Modes>();

    let pack_modes = modes.pack_modes.read();
    let pack_mode_options = pack_modes.iter().map(|mode_file| {
        let options = mode_file.metadata.modes.iter().map(|(key, mode)| {
            rsx! {
                ModeSelectOption {
                    mode: shared::user_config::Mode::Pack { id: mode_file.id, mode: key.clone() },
                    name: mode.name.clone(),
                }
            }
        });

        rsx! {
            if pack_modes.len() != 1 {
                p {
                    class: "text-xs font-semibold text-gray-500 mt-1 mb-0.5",
                    "{mode_file.metadata.name}"
                }
            }
            div {
                class: "flex flex-col pl-2",
                {options}
            }
        }
    });

    let default_mode = modes.default_mode.read();
    let default_mode_options = default_mode.modes.iter().map(|(key, mode)| {
        rsx! {
            ModeSelectOption {
                mode: shared::user_config::Mode::Default(key.clone()),
                name: mode.name.clone(),
            }
        }
    });

    let uploaded_modes = modes.uploaded_modes.read();
    let uploaded_mode_options = uploaded_modes.iter().map(|mode_file| {
        let file_name = mode_file.path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let options = mode_file.metadata.modes.iter().map(|(key, mode)| {
            rsx! {
                ModeSelectOption {
                    mode: shared::user_config::Mode::File { path: mode_file.path.clone(), mode: key.clone() },
                    name: mode.name.clone(),
                }
            }
        });

        rsx! {
            p {
                class: "text-xs font-semibold text-gray-500 mt-1 mb-0.5",
                "{mode_file.metadata.name} "
                span {
                    class: "font-normal text-gray-400",
                    "({file_name})"
                }
            }
            div {
                class: "flex flex-col pl-2",
                {options}
            }
        }
    });

    rsx! {
        div {
            class: "flex flex-col gap-3 max-h-96 overflow-y-auto rounded-md border border-gray-300 bg-white p-3",

            if !modes.pack_modes.read().is_empty() {
                div {
                    class: "flex flex-col gap-0.5",
                    SectionHeader { title: "From Pack" }
                    div {
                        class: "flex flex-col pl-2",
                        {pack_mode_options}
                    }
                }
                Divider {}
            }

            div {
                class: "flex flex-col gap-0.5",
                div {
                    class: "flex items-center justify-between",
                    SectionHeader { title: "Uploaded" }
                    input {
                        id: "mode-input",
                        class: "hidden",
                        type: "file",
                        accept: ".lwmode",
                        onchange: move |event: FormEvent| async move {
                            if let Some(file) = event.files().first() {
                                let path = file.path();
                                if let Err(err) = modes.upload_mode(path).await {
                                    eprintln!("{err}");
                                }
                            }
                        },
                    }
                    label {
                        for: "mode-input",
                        class: "flex items-center gap-1 rounded px-2 py-0.5 text-xs text-sky-600 hover:bg-sky-50 transition-colors",
                        Icon { icon: Shape::Plus, size: 14 }
                        "Upload"
                    }
                }
                div {
                    class: "flex flex-col pl-2",
                    {uploaded_mode_options}
                }
            }

            Divider {}

            div {
                class: "flex flex-col gap-0.5",
                SectionHeader { title: "Built-in" }
                div {
                    class: "flex flex-col pl-2",
                    {default_mode_options}
                }
            }
        }
    }
}

#[component]
fn SectionHeader(title: String) -> Element {
    rsx! {
        p {
            class: "text-sm font-semibold text-gray-700 mb-0.5",
            "{title}"
        }
    }
}

#[component]
fn Divider() -> Element {
    rsx! {
        hr { class: "border-gray-200" }
    }
}

#[component]
fn ModeSelectOption(mode: ReadSignal<shared::user_config::Mode>, name: String) -> Element {
    let config = use_context::<Config>().0;
    let mut modes = use_context::<Modes>();

    let selected = use_memo(move || &*config.mode().read() == &*mode.read());

    rsx! {
        button {
            onclick: move |_| {
                modes.set_mode(mode.read().clone());
            },
            class: "flex items-center gap-2 w-full px-2 py-1.5 rounded text-sm text-left transition-colors",
            class: if selected() {
                "bg-sky-50 text-sky-700 font-medium"
            } else {
                "text-gray-700 hover:bg-gray-100"
            },
            div {
                class: "size-4 shrink-0 text-sky-500",
                if selected() {
                    Icon { icon: Shape::Check, size: 16 }
                }
            }
            "{name}"
        }
    }
}

#[component]
fn ModeOptions() -> Element {
    let modes = use_context::<Modes>();

    rsx! {
        if let Some(mode) = modes.selected_mode() {
            if !mode.options.is_empty() {
                Section { title: format!("{} Options", mode.name),
                    div { class: "flex flex-col gap-6",
                        ModeOptionList { mode }
                    }
                }
            }
        }
    }
}

#[component]
fn ModeOptionList(mode: ReadSignal<shared::mode::Mode>) -> Element {
    let config: Store<AppConfig> = use_context::<Config>().0;
    let mode_option_store: Option<Store<HashMap<String, OptionValue>, _>> =
        config.mode_options().get(config.mode()());

    rsx! {
        if let Some(options_signal) = mode_option_store {
            for (id, option) in mode.read().options.iter() {
                if let Some(mut value) = options_signal.clone().get(id.clone()) {
                    div {
                        class: "flex items-center gap-5",
                        if let Some(description) = &option.description {
                            Tooltip {
                                TooltipTrigger {
                                    div {
                                        class: "flex items-center gap-2",
                                        h3 {
                                            {option.label.clone()}
                                        }
                                        Icon {
                                            class: "shrink-0",
                                            icon: Shape::InformationCircle,
                                            size: 20,
                                        }
                                    }
                                }
                                TooltipContent {
                                    side: ContentSide::Right,
                                    p {
                                        width: "max-content",
                                        max_width: "100%",
                                        "{description}"
                                    }
                                }
                            }
                        } else {
                            h3 {
                                {option.label.clone()}
                            }
                        }
                        match option.option_type.clone() {
                            OptionType::Integer { default: _, min, max, step, clamp, slider } => rsx! {
                                if let OptionValue::Integer(v) = value() {
                                    IntegerOption {
                                        value: v,
                                        on_value_change: move |v| {
                                            value.set(OptionValue::Integer(v));
                                        },
                                        min,
                                        max,
                                        step,
                                        clamp,
                                        slider,
                                    }
                                }
                            },
                            OptionType::Number { default: _, min, max, step, clamp, slider } => rsx! {
                                if let OptionValue::Number(v) = value() {
                                    NumberOption {
                                        value: v,
                                        on_value_change: move |v| {
                                            value.set(OptionValue::Number(v));
                                        },
                                        min,
                                        max,
                                        step,
                                        clamp,
                                        slider,
                                    }
                                }
                            },
                            OptionType::String { default: _ } => rsx! {
                                if let OptionValue::String(v) = value() {
                                    StringOption {
                                        value: v,
                                        on_value_change: move |v| {
                                            value.set(OptionValue::String(v));
                                        },
                                    }
                                }
                            },
                            OptionType::Boolean { default: _ } => rsx! {
                                if let OptionValue::Boolean(v) = value() {
                                    BooleanOption {
                                        value: v,
                                        on_value_change: move |v| {
                                            value.set(OptionValue::Boolean(v));
                                        },
                                    }
                                }
                            },
                            OptionType::Enum { default: _, values } => rsx! {
                                if let OptionValue::Enum(v) = value() {
                                    EnumOption {
                                        value: v,
                                        on_value_change: move |v| {
                                            value.set(OptionValue::Enum(v));
                                        },
                                        options: values,
                                    }
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn BooleanOption(value: ReadSignal<bool>, on_value_change: Callback<bool>) -> Element {
    rsx! {
        Switch {
            checked: value(),
            on_checked_change: on_value_change,
            SwitchThumb {}
        }
    }
}

#[component]
fn IntegerOption(
    value: ReadSignal<i64>,
    on_value_change: Callback<i64>,
    min: ReadSignal<Option<i64>>,
    max: ReadSignal<Option<i64>>,
    step: ReadSignal<Option<i64>>,
    clamp: ReadSignal<bool>,
    slider: ReadSignal<bool>,
) -> Element {
    rsx! {
        NumberOption {
            value: value() as f64,
            on_value_change: move |value| {
                on_value_change(value as i64);
            },
            min: min().map(|x| x as f64),
            max: max().map(|x| x as f64),
            step: step().map(|x| x as f64),
            clamp: clamp(),
            slider: slider(),
        }
    }
}

fn round_to_step(value: f64, step: f64) -> f64 {
    if step <= 0.0 {
        return value;
    }

    let snapped = (value / step).round() * step;
    let decimals = (-step.log10().floor()).max(0.0) as i32;
    let factor = 10f64.powi(decimals);
    (snapped * factor).round() / factor
}

#[component]
fn NumberOption(
    value: ReadSignal<f64>,
    on_value_change: Callback<f64>,
    min: ReadSignal<Option<f64>>,
    max: ReadSignal<Option<f64>>,
    step: ReadSignal<Option<f64>>,
    clamp: ReadSignal<bool>,
    slider: ReadSignal<bool>,
) -> Element {
    let on_change = move |mut value: f64| {
        if let Some(step) = step() {
            value = round_to_step(value, step);
        }

        if clamp() {
            if let Some(min) = min() {
                if value < min {
                    value = min;
                }
            }

            if let Some(max) = max() {
                if value > max {
                    value = max;
                }
            }
        }

        on_value_change(value);
    };

    let input = rsx! {
        Input {
            r#type: "number",
            value: value(),
            min: min(),
            max: max(),
            step: step(),
            oninput: move |event: FormEvent| {
                if let Ok(n) = event.value().parse::<f64>() {
                    on_change(n);
                }
            },
        }
    };

    rsx! {
        if slider() {
            div {
                class: "flex items-center gap-5",
                Slider {
                    min: min().unwrap_or(0.0),
                    max: max().unwrap_or(100.0),
                    step: step().unwrap_or(1.0),
                    value: SliderValue::Single(value()),
                    horizontal: true,
                    on_value_change: move |SliderValue::Single(v): SliderValue| {
                        on_change(v);
                    },
                    SliderTrack {
                        SliderRange {}
                        SliderThumb {}
                    }
                }
                div {
                    class: "flex justify-center",
                    {input}
                }
            }
        } else {
            {input}
        }
    }
}

#[component]
fn StringOption(value: ReadSignal<String>, on_value_change: Callback<String>) -> Element {
    rsx! {
        Input {
            value,
            oninput: move |event: FormEvent| {
                on_value_change(event.value());
            },
        }
    }
}

#[component]
fn EnumOption(
    value: ReadSignal<String>,
    on_value_change: Callback<String>,
    options: ReadSignal<IndexMap<String, String>>,
) -> Element {
    let options = options.read();
    let select_options = options.iter().enumerate().map(|(i, (key, value))| {
        rsx! {
            SelectOption::<String> {
                index: i,
                value: key.clone(),
                text_value: "{value}",
                "{value}"
                SelectItemIndicator {}
            }
        }
    });

    rsx! {
        Select {
            value: Some(Some(value())),
            on_value_change: move |value| {
                if let Some(value) = value {
                    on_value_change(value);
                }
            },
            SelectTrigger {
                SelectValue {  }
            }
            SelectList {
                {select_options}
            }
        }
    }
}
