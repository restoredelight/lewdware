mod components;
mod encode;
mod image_list;
mod media_server;
mod menu;
mod pack;
mod thumbnail;
mod utils;
mod options;

use dioxus::prelude::*;
use dioxus_desktop::{tao, use_muda_event_handler, use_window, use_wry_event_handler, Config};
use dioxus_heroicons::{solid::Shape, Icon};
use dioxus_primitives::toast::{use_toast, ToastOptions};
use shared::pack_config::Metadata;

use crate::{
    components::{
        button::Button,
        dialog::{DialogContent, DialogRoot, DialogTitle},
        input::Input,
        label::Label,
        toast::ToastProvider,
    }, image_list::ImageList, media_server::start_media_server, menu::{MenuAction, create_menu}, options::Options, pack::MediaPack
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

fn main() -> anyhow::Result<()> {
    dioxus::LaunchBuilder::new()
        .with_cfg(Config::new().with_menu(Some(create_menu()?)))
        .launch(App);

    Ok(())
}

#[component]
fn App() -> Element {
    use_context_provider(|| SaveProgress::new());

    rsx! {
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Link { rel: "stylesheet", href: DX_COMPONENTS_CSS }
        ToastProvider {
            SavePopup {}
            Main {}
        }
    }
}

#[derive(Clone, Copy)]
pub struct SaveProgress {
    saving: SyncSignal<usize>,
    saved: SyncSignal<usize>,
}

impl SaveProgress {
    pub fn new() -> Self {
        Self {
            saving: SyncSignal::new_maybe_sync(0),
            saved: SyncSignal::new_maybe_sync(0),
        }
    }

    pub fn start_saving(&mut self, saving: usize) {
        self.saving.set(saving);
    }

    pub fn increment_saved(&mut self) {
        self.saved += 1;
    }

    pub fn reset(&mut self) {
        self.saving.set(0);
        self.saved.set(0);
    }

    pub fn is_saving(&self) -> bool {
        self.saving() > 0
    }

    pub fn saving(&self) -> usize {
        (self.saving)()
    }

    pub fn saved(&self) -> usize {
        (self.saved)()
    }
}

#[component]
fn SavePopup() -> Element {
    let progress = use_context::<SaveProgress>();

    let saving = progress.saving();
    let saved = progress.saved();

    let percent = if saving == 0 {
        0.0
    } else {
        (saved as f32 / saving as f32) * 100.0
    };

    rsx! {
        if progress.is_saving() {
            div { class: "fixed inset-0 z-2000 flex items-center justify-center bg-black/40",

                div { class: "bg-neutral-900 rounded-xl shadow-xl px-8 py-6 min-w-[320px] text-center space-y-3",

                    h3 { class: "text-lg font-semibold", "Saving file…" }

                    p { class: "text-sm text-neutral-400", "{saved} / {saving} files" }

                    progress {
                        class: "w-full h-2",
                        max: "{saving}",
                        value: "{saved}",
                    }

                    p { class: "text-sm text-neutral-300", "{percent:.0}%" }
                }
            }
        }
    }
}

#[component]
fn Main() -> Element {
    let mut pack = use_store::<Option<MediaPack>>(|| None);
    let metadata = use_store(|| Metadata::default());

    let window = use_window();

    let mut before_close_dialog_open = use_signal(|| false);
    let mut on_close_callback = use_callback(|_| {});
    let close_and_then = move |mut f: Box<dyn FnMut(())>| {
        spawn(async move {
            let unsaved = match pack.as_ref() {
                Some(pack) => !pack.is_saved().await,
                None => false,
            };

            if unsaved {
                before_close_dialog_open.set(true);
                on_close_callback.replace(Box::new(f));
            } else {
                f(());
            }
        })
    };

    use_wry_event_handler(move |event, _| {
        if let tao::event::Event::WindowEvent {
            event: tao::event::WindowEvent::CloseRequested,
            ..
        } = event
        {
            let window = window.clone();
            close_and_then(Box::new(move |_| {
                window.close();
            }));
        }
    });

    let mut new_pack_dialog_open = use_signal(|| false);

    use_muda_event_handler(move |event| {
        let action: MenuAction = match event.id().0.parse() {
            Ok(x) => x,
            Err(err) => {
                eprintln!("{err}");
                return;
            }
        };

        match action {
            MenuAction::New => {
                new_pack_dialog_open.set(true);
            }
            MenuAction::Open => {
                close_and_then(Box::new(move |_| {
                    spawn(async move {
                        match open_pack().await {
                            Ok(Some(new_pack)) => {
                                *pack.write() = Some(new_pack);
                            }
                            Ok(None) => {}
                            Err(err) => {
                                eprintln!("{err}");
                            }
                        }
                    });
                }));
            }
            MenuAction::Save => {
                spawn(async move {
                    if let Some(pack) = pack.as_ref() {
                        if let Err(err) = save(&pack, metadata).await {
                            eprintln!("{err}");
                        }
                    }
                });
            }
            MenuAction::SaveAs => {
                spawn(async move {
                    if let Some(file) = rfd::AsyncFileDialog::new()
                        .set_title("Save as...")
                        .add_filter("Lewdware Pack", &["md"])
                        .save_file()
                        .await
                    {
                        let new_pack = if let Some(current_pack) = pack.as_ref() {
                            current_pack.set_metadata(&metadata.read());
                            match current_pack.save_as(file.path()).await {
                                Ok(pack) => pack,
                                Err(err) => {
                                    eprintln!("{err}");
                                    return;
                                }
                            }
                        } else {
                            None
                        };

                        if let Some(new_pack) = new_pack {
                            *pack.write() = Some(new_pack);
                        }
                    }
                });
            }
        }
    });

    rsx! {
        NewPackDialog { pack, open: new_pack_dialog_open }
        BeforeCloseDialog {
            open: before_close_dialog_open,
            on_close: on_close_callback,
            pack,
            metadata,
        }
        match pack.transpose() {
            Some(pack) => {
                rsx! {
                    Editor { pack, metadata }
                }
            }
            None => rsx! {
                Start { pack, new_pack_dialog_open }
            },
        }
    }
}

#[component]
pub fn BeforeCloseDialog(
    open: Signal<bool>,
    on_close: EventHandler<()>,
    pack: Store<Option<MediaPack>>,
    metadata: Store<Metadata>,
) -> Element {
    let mut loading = use_signal(|| false);
    let toast_api = use_toast();

    rsx! {
        DialogRoot { open: open(), on_open_change: move |v| open.set(v),
            DialogContent {
                class: "max-w-128 rounded-md p-8",
                button {
                    class: "dialog-close",
                    r#type: "button",
                    aria_label: "Close",
                    tabindex: if open() { "0" } else { "-1" },
                    onclick: move |_| open.set(false),
                    Icon { icon: Shape::XMark }
                }
                DialogTitle { "Save pack?" }
                Button {
                    onclick: move |_| async move {
                        loading.set(true);
                        if let Some(pack) = pack.as_ref() {
                            match save(&pack, metadata).await {
                                Ok(()) => {
                                    open.set(false);
                                    on_close(())
                                }
                                Err(err) => {
                                    eprintln!("{err}");
                                }
                            }
                        }
                        loading.set(false);
                    },
                    disabled: loading(),
                    "Save"
                }
                Button {
                    onclick: move |_| async move {
                        loading.set(true);
                        if let Some(pack) = pack.as_ref() {
                            match discard_changes(&pack, metadata).await {
                                Ok(()) => {
                                    open.set(false);
                                    on_close(())
                                }
                                Err(err) => {
                                    eprintln!("{err}");
                                }
                            }
                        }
                        loading.set(false);
                    },
                    disabled: loading(),
                    "Discard changes"
                }
                Button { onclick: move |_| open.set(false), disabled: loading(), "Cancel" }
            }
        }
    }
}

#[component]
pub fn Start(pack: Store<Option<MediaPack>>, new_pack_dialog_open: Signal<bool>) -> Element {
    rsx! {
        div { id: "hero",
            img { src: HEADER_SVG, id: "header" }
            Button { onclick: move |_| new_pack_dialog_open.set(true),
                Icon { icon: Shape::Plus, size: 30, class: "m-auto" }
                "New pack"
            }
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

#[component]
pub fn NewPackDialog(pack: Store<Option<MediaPack>>, open: Signal<bool>) -> Element {
    let mut name = use_signal(String::new);
    let mut loading = use_signal(|| false);

    rsx! {
        DialogRoot { open: open(), on_open_change: move |v| open.set(v),
            DialogContent {
                class: "max-w-128 rounded-md p-8",
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
                                        println!("Created pack successfully");
                                        *pack.write() = Some(new_pack);
                                        open.set(false);
                                    }
                                    Err(err) => eprintln!("Creating pack failed: {err}"),
                                }
                            }

                            loading.set(false);
                        }
                    },
                    disabled: loading(),
                    "Create"
                }
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

#[derive(Clone)]
pub struct MetadataStore(Store<Metadata>);

pub async fn save(pack: &MediaPack, metadata: Store<Metadata>) -> anyhow::Result<()> {
    pack.set_metadata(&metadata.read());
    pack.save().await
}

pub async fn discard_changes(
    pack: &MediaPack,
    mut metadata: Store<Metadata>,
) -> anyhow::Result<()> {
    pack.discard_changes().await?;
    *metadata.write() = pack.metadata();

    Ok(())
}

#[component]
pub fn Editor(pack: ReadSignal<MediaPack>, metadata: Store<Metadata>) -> Element {
    let port = use_resource(move || async move {
        start_media_server(pack.read().get_view().unwrap())
            .await
            .unwrap()
    });

    use_effect(move || {
        metadata.set(pack.read().metadata());
    });

    use_context_provider(|| Port(port.value()));
    use_context_provider(|| Pack(pack));
    use_context_provider(|| MetadataStore(metadata));

    let selected_menu_item = use_signal(|| MenuItem::MediaView);

    rsx! {
        div {
            class: "h-screen flex overflow-hidden",
            Menu { selected: selected_menu_item }
            match selected_menu_item() {
                MenuItem::Options => rsx!{ Options {} },
                MenuItem::MediaView => rsx! {ImageList {}}
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuItem {
    Options,
    MediaView,
}

impl std::fmt::Display for MenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Options => write!(f, "Options"),
            Self::MediaView => write!(f, "Media"),
        }
    }
}

impl MenuItem {
    fn icon(&self) -> Shape {
        match self {
            Self::Options => Shape::Squares2x2,
            Self::MediaView => Shape::Photo,
        }
    }
}

#[component]
fn Menu(selected: Signal<MenuItem>) -> Element {
    let mut open = use_signal(|| false);

    let options = 
        [MenuItem::Options, MenuItem::MediaView].iter().map(|&option| {
            rsx! {
                button {
                    class: "flex items-center p-1 rounded-sm w-full nowrap transition-[width]",
                    class: if selected() == option { "bg-sky-400 text-white" } else { "hover:bg-sky-200" },
                    onclick: move |_| {
                        selected.set(option);
                    },
                    Icon {
                        class: "mx-1",
                        icon: option.icon(),
                        size: 20,
                    }
                    "{option}"
                }
            }
        });

    rsx! {
        div {
            class: "p-[5px] transition-[width] border-r border-gray-300 bg-gray-50 overflow-hidden",
            width: if open() {"16rem"} else {"40px"},
            div {
                class: "w-full h-[35px] border-b border-gray-300",
                button {
                    class: "float-right rounded-sm hover:text-gray-800 hover:bg-gray-200 size-[30px] flex justify-center items-center",
                    onclick: move |_| {
                        open.toggle();
                    },
                    Icon {
                        icon: Shape::Bars3,
                        size: 25,
                    }
                }
            }
            if open() {
                div {
                    class: "flex flex-col gap-1 pt-2",
                    {options}
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
                match open_pack().await {
                    Ok(Some(opened_pack)) => {
                        *pack.write() = Some(opened_pack);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        eprintln!("{err}");
                    }
                }
            },
            Icon { icon: Shape::Plus, size: 30, class: "m-auto" }
            "Kill yourself"
        }
    }
}

async fn open_pack() -> Result<Option<MediaPack>> {
    match rfd::AsyncFileDialog::new()
        .set_title("Select media pack")
        .add_filter("Lewdware Pack", &["md"])
        .pick_file()
        .await
    {
        Some(file) => Ok(Some(MediaPack::open(file.path().to_path_buf()).await?)),
        None => Ok(None),
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
