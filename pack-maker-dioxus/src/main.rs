mod components;
mod image_list;
mod media_server;
mod pack;
mod thumbnail;
mod utils;
mod encode;

use dioxus::prelude::*;
use dioxus_heroicons::{solid::Shape, Icon};

use crate::{
    components::{
        button::Button,
        dialog::{DialogContent, DialogRoot, DialogTitle},
        input::Input,
        label::Label,
    },
    image_list::ImageList,
    media_server::start_media_server,
    pack::MediaPack,
};

#[derive(Debug, Clone, Routable, PartialEq)]
#[rustfmt::skip]
enum Route {
    #[layout(Navbar)]
    #[route("/")]
    // Home {},
    #[route("/blog/:id")]
    Blog { id: i32 },
}

const FAVICON: Asset = asset!("/assets/favicon.ico");
const MAIN_CSS: Asset = asset!("/assets/main.css");
const HEADER_SVG: Asset = asset!("/assets/header.svg");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");
const DX_COMPONENTS_CSS: Asset = asset!("/assets/dx-components-theme.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let pack = use_store(|| None);

    let page = match pack.transpose() {
        Some(pack) => {
            rsx! {
                Main { pack }
            }
        }
        None => rsx! {
            Start { pack }
        },
    };

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Link { rel: "stylesheet", href: DX_COMPONENTS_CSS }
        {page}
    }
}

#[component]
pub fn Start(pack: Store<Option<MediaPack>>) -> Element {
    rsx! {
        div { id: "hero",
            img { src: HEADER_SVG, id: "header" }
            CreatePack { pack }
            OpenPack { pack }
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

#[derive(Clone)]
pub struct Port(ReadSignal<Option<u16>>);

pub fn use_port() -> Option<u16> {
    use_context::<Port>().0.read().cloned()
}

#[derive(Clone)]
pub struct Pack(ReadSignal<MediaPack>);

#[component]
pub fn Main(pack: ReadSignal<MediaPack>) -> Element {
    let port = use_resource(move || async move {
        start_media_server(pack.read().get_view().unwrap()).await.unwrap()
    });

    use_context_provider::<Port>(|| Port(port.value()));
    use_context_provider::<Pack>(|| Pack(pack));

    rsx! {
        div { class: "h-screen", ImageList {} }
    }
}

#[component]
fn CreatePack(pack: Store<Option<MediaPack>>) -> Element {
    let mut open = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut loading = use_signal(|| false);

    rsx! {
        Button { onclick: move |_| open.set(true),
            Icon { icon: Shape::Plus, size: 30, class: "m-auto" }
            "Kill yourself"
        }
        DialogRoot { open: open(), on_open_change: move |v| open.set(v),
            DialogContent {
                button {
                    class: "dialog-close",
                    r#type: "button",
                    aria_label: "Close",
                    tabindex: if open() { "0" } else { "-1" },
                    onclick: move |_| open.set(false),
                    Icon { icon: Shape::XMark }
                }
                DialogTitle { "Create a pack" }
                Label { html_for: "name", "Name" }
                Input {
                    id: "name",
                    oninput: move |e: FormEvent| name.set(e.value()),
                    placeholder: "Enter the pack name",
                    value: name,
                }
                Button {
                    class: "button m-auto",
                    onclick: move |_| {
                        loading.set(true);

                        async move {
                            if let Some(file) = rfd::AsyncFileDialog::new()
                                .set_title("Save media pack")
                                .set_file_name(format!("{}.md", name))
                                .add_filter("Lewdware Pack", &["md"])
                                .save_file()

                                .await
                            {
                                let name = name.read();
                                match MediaPack::new(file.path().to_path_buf(), &name).await {
                                    Ok(new_pack) => {
                                        *pack.write() = Some(new_pack);
                                    }
                                    Err(err) => eprintln!("{err}"),
                                }
                            }
                        }
                    },
                    disabled: loading(),
                    "Create"
                }
            }
        }
    }
}

#[component]
fn OpenPack(pack: Store<Option<MediaPack>>) -> Element {
    rsx! {
        Button {
            onclick: move |_| async move {
                if let Some(file) = rfd::AsyncFileDialog::new()
                    .set_title("Select media pack")
                    .add_filter("Lewdware Pack", &["md"])
                    .pick_file()
                    .await
                {
                    match MediaPack::open(file.path().to_path_buf()).await {
                        Ok(opened_pack) => {
                            *pack.write() = Some(opened_pack);
                        }
                        Err(err) => {
                            eprintln!("{err}");
                        }
                    }
                }
            },
            Icon { icon: Shape::Plus, size: 30, class: "m-auto" }
            "Kill yourself"
        }
    }
}

/// /// Home page
/// #[component]
/// fn Home() -> Element {
///     rsx! {
///         Start {}
///
///     }
/// }

/// Blog page
#[component]
pub fn Blog(id: i32) -> Element {
    rsx! {
        div { id: "blog",

            // Content
            h1 { "This is blog #{id}!" }
            p {
                "In blog #{id}, we show how the Dioxus router works and how URL parameters can be passed as props to our route components."
            }

            // Navigation links
            Link { to: Route::Blog { id: id - 1 }, "Previous" }
            span { " <---> " }
            Link { to: Route::Blog { id: id + 1 }, "Next" }
        }
    }
}

/// Shared navbar component.
#[component]
fn Navbar() -> Element {
    rsx! {
        div { id: "navbar",
            // Link {
            //     // to: Route::Home {},
            //     "Home"
            // }
            Link { to: Route::Blog { id: 1 }, "Blog" }
        }

        Outlet::<Route> {}
    }
}
