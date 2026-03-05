mod general;
mod pack_mode;
mod modes;
mod pack;

use std::{fmt::write, path::Path};
use std::time::Duration;

use dioxus::{
    desktop::{tao, use_window, use_wry_event_handler},
    prelude::*,
};
use dioxus_heroicons::solid::Shape;
use shared::{
    components::menu::{Menu, MenuItem},
    mode::read_mode_metadata,
    user_config::{load_config, save_config_async, AppConfig},
};

use crate::modes::UploadedMode;
use crate::pack_mode::PackModeSettings;
use crate::{general::General, modes::Modes, pack::MediaPack};

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");
const HEADER_SVG: Asset = asset!("/assets/header.svg");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");
const DX_COMPONENTS_CSS: Asset = asset!("/assets/dx-components-theme.css");

fn main() -> anyhow::Result<()> {
    let (config, pack, uploaded_modes) = init();

    let metadata = {
        let data = include_bytes!("../../default-modes/build/Default Modes.lwmode");

        let mut cursor = std::io::Cursor::new(data);

        read_mode_metadata(&mut cursor)?.1
    };

    dioxus::LaunchBuilder::new()
        .with_context(config)
        .with_context(pack)
        .with_context(metadata)
        .with_context(uploaded_modes)
        .launch(App);

    Ok(())
}

#[derive(Clone)]
pub struct Config(Store<AppConfig>);

#[derive(Clone)]
pub struct Pack(Signal<Option<MediaPack>>);

fn init() -> (AppConfig, Option<MediaPack>, Vec<UploadedMode>) {
    let mut config = match load_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}");
            Default::default()
        }
    };

    let pack = if let Some(path) = &config.pack_path {
        match MediaPack::open(path.clone()) {
            Ok(pack) => Some(pack),
            Err(err) => {
                eprintln!("{err}");
                config.pack_path = None;
                None
            }
        }
    } else {
        None
    };

    let mut retained_modes = Vec::new();
    let mut modes = Vec::new();

    for mode_path in config.uploaded_modes.drain(..) {
        match read_mode(&mode_path) {
            Ok(metadata) => {
                modes.push(UploadedMode {
                    path: mode_path.clone(),
                    metadata,
                });
                retained_modes.push(mode_path);
            }
            Err(err) => {
                eprintln!("{err}");
            }
        }
    }

    config.uploaded_modes = retained_modes;

    (config, pack, modes)
}

fn read_mode(path: &Path) -> Result<shared::mode::Metadata> {
    let mut file = std::fs::File::open(path)?;

    Ok(read_mode_metadata(&mut file)?.1)
}

#[component]
fn App() -> Element {
    let config = use_store(|| consume_context::<AppConfig>());
    let pack = use_signal(|| consume_context::<Option<MediaPack>>());

    use_context_provider(|| Config(config));
    use_context_provider(|| Pack(pack));
    let mut modes = use_context_provider(|| {
        Modes::new(
            config,
            consume_context::<shared::mode::Metadata>(),
            consume_context::<Vec<UploadedMode>>(),
        )
    });

    let window = use_window();

    // Save before closing the window
    use_wry_event_handler(move |event, _| {
        if let tao::event::Event::WindowEvent {
            event: tao::event::WindowEvent::CloseRequested,
            ..
        } = event
        {
            let window = window.clone();
            spawn(async move {
                let config_data: AppConfig = config.peek().clone();

                if let Err(err) = save_config_async(config_data).await {
                    eprintln!("Error saving config: {err}");
                }

                window.close();
            });
        }
    });

    // Periodically save
    use_future(move || async move {
        println!("Running future");
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.tick().await;

        loop {
            interval.tick().await;

            // Make sure we don't hold the config read lock across await points
            let config_data: AppConfig = config.peek().clone();

            if let Err(err) = save_config_async(config_data).await {
                eprintln!("Error saving pack: {err}");
            } else {
                println!("Successfully saved");
            }
        }
    });

    use_future(move || async move {
        if let Err(err) = modes.update_pack(&*pack.read()).await {
            eprintln!("Error updating pack: {err}");
        }
    });

    let selected_menu_item = use_signal(|| ConfigMenuItem::General);

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Link { rel: "stylesheet", href: DX_COMPONENTS_CSS }
        div { class: "h-screen flex overflow-hidden bg-white text-black",
            Menu { selected: selected_menu_item, initially_open: true }
            match selected_menu_item() {
                ConfigMenuItem::General => rsx! {
                    General {}
                },
                ConfigMenuItem::PackMode => rsx! {
                    PackModeSettings {}
                },
                ConfigMenuItem::Advanced => rsx! {},
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigMenuItem {
    General,
    PackMode,
    Advanced,
}

impl std::fmt::Display for ConfigMenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::General => write!(f, "General"),
            Self::PackMode => write!(f, "Pack & Mode"),
            Self::Advanced => write!(f, "Advanced"),
        }
    }
}

impl MenuItem for ConfigMenuItem {
    const VARIANTS: &'static [Self] = &[Self::General, Self::PackMode, Self::Advanced];

    fn icon(&self) -> Shape {
        match self {
            Self::General => Shape::Cog6Tooth,
            Self::PackMode => Shape::ArchiveBox,
            Self::Advanced => Shape::AdjustmentsHorizontal,
        }
    }
}

#[component]
pub fn Hero() -> Element {
    rsx! {
        div { id: "hero",
            img { src: HEADER_SVG, id: "header" }
            div { id: "links",
                a { href: "https://dioxuslabs.com/learn/0.7/", "📚 Learn Dioxus" }
                a { href: "https://dioxuslabs.com/awesome", "🚀 Awesome Dioxus" }
                a { href: "https://github.com/dioxus-community/", "📡 Community Libraries" }
                a { href: "https://github.com/DioxusLabs/sdk", "⚙️ Dioxus Development Kit" }
                a { href: "https://marketplace.visualstudio.com/items?itemName=DioxusLabs.dioxus",
                    "💫 VSCode Extension"
                }
                a { href: "https://discord.gg/XgGxMSkvUM", "👋 Community Discord" }
            }
        }
    }
}
