mod general;

use std::fs::File;
use std::time::Duration;

use dioxus::{
    core::anyhow,
    desktop::{tao, use_window, use_wry_event_handler},
    prelude::*,
};
use dioxus_heroicons::{solid::Shape, Icon};
use shared::{
    components::menu::{Menu, MenuItem},
    pack_config::Metadata,
    read_pack::{read_pack_metadata, Header},
    user_config::{load_config, save_config_async, AppConfig},
};

use crate::general::General;

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");
const HEADER_SVG: Asset = asset!("/assets/header.svg");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

fn main() {
    let (config, pack) = init();

    dioxus::LaunchBuilder::new()
        .with_context(config)
        .with_context(pack)
        .launch(App);
}

#[derive(Clone)]
pub struct Config(Store<AppConfig>);

#[derive(Clone)]
pub struct MediaPack {
    header: Header,
    metadata: Metadata,
}

#[derive(Clone)]
pub struct Pack(Signal<Option<MediaPack>>);

fn init() -> (AppConfig, Option<MediaPack>) {
    let mut config = match load_config() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{err}");
            Default::default()
        }
    };

    let pack = if let Some(path) = &config.pack_path {
        match File::open(path)
            .map_err(|err| anyhow!(err))
            .and_then(|file| read_pack_metadata(file))
        {
            Ok((header, metadata)) => Some(MediaPack { header, metadata }),
            Err(err) => {
                eprintln!("{err}");
                config.pack_path = None;
                None
            }
        }
    } else {
        None
    };

    (config, pack)
}

#[component]
fn App() -> Element {
    let config = use_store(|| consume_context::<AppConfig>());
    let pack = use_signal(|| consume_context::<Option<MediaPack>>());

    use_context_provider(|| Config(config));
    use_context_provider(|| Pack(pack));

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
                if let Err(err) = save_config_async(&config.read()).await {
                    eprintln!("{err}");
                }

                window.close();
            });
        }
    });

    // Periodically save
    spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.tick().await;

        loop {
            interval.tick().await;
            if let Err(err) = save_config_async(&config.read()).await {
                eprintln!("{err}");
            }
        }
    });

    let selected_menu_item = use_signal(|| ConfigMenuItem::General);

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS } document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        div {
            class: "h-screen flex overflow-hidden bg-white text-black",
            Menu { selected: selected_menu_item, initially_open: true }
            match selected_menu_item() {
                ConfigMenuItem::General => rsx!{ General {  } },
                ConfigMenuItem::Advanced => rsx!{},
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfigMenuItem {
    General,
    Advanced,
}

impl std::fmt::Display for ConfigMenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::General => write!(f, "General"),
            Self::Advanced => write!(f, "Advanced"),
        }
    }
}

impl MenuItem for ConfigMenuItem {
    const VARIANTS: &'static [Self] = &[Self::General, Self::Advanced];

    fn icon(&self) -> Shape {
        match self {
            Self::General => Shape::Cog6Tooth,
            Self::Advanced => Shape::AdjustmentsHorizontal,
        }
    }
}

#[component]
pub fn Hero() -> Element {
    rsx! {
        div {
            id: "hero",
            img { src: HEADER_SVG, id: "header" }
            div { id: "links",
                a { href: "https://dioxuslabs.com/learn/0.7/", "📚 Learn Dioxus" }
                a { href: "https://dioxuslabs.com/awesome", "🚀 Awesome Dioxus" }
                a { href: "https://github.com/dioxus-community/", "📡 Community Libraries" }
                a { href: "https://github.com/DioxusLabs/sdk", "⚙️ Dioxus Development Kit" }
                a { href: "https://marketplace.visualstudio.com/items?itemName=DioxusLabs.dioxus", "💫 VSCode Extension" }
                a { href: "https://discord.gg/XgGxMSkvUM", "👋 Community Discord" }
            }
        }
    }
}
