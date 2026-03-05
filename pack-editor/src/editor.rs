use std::future::Future;

use dioxus::{core::{Runtime, Task, current_scope_id}, prelude::*};
use dioxus_heroicons::solid::Shape;
use shared::{components::menu::{Menu, MenuItem}, pack_config::Metadata};

use crate::{image_list::{MediaPage, TagsContext}, media_server::start_media_server, options::Options, pack::MediaPack, upload_files::UploadFilesContext};

#[derive(Clone)]
pub struct Port(ReadSignal<Option<u16>>);

pub fn use_port() -> Option<u16> {
    use_context::<Port>().0.read().cloned()
}

#[derive(Clone)]
pub struct Pack(pub ReadSignal<MediaPack>);

#[derive(Clone)]
pub struct MetadataStore(pub Store<Metadata>);


#[derive(Clone)]
struct EditorScopeId(ScopeId);

pub fn spawn_in_editor(fut: impl Future<Output = ()> + 'static) -> Task {
    let rt = Runtime::current();
    rt.in_scope(consume_context::<EditorScopeId>().0, || spawn(fut))
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

    let scope_id = current_scope_id();

    use_context_provider(|| Port(port.value()));
    use_context_provider(|| Pack(pack));
    use_context_provider(|| MetadataStore(metadata));
    provide_context(EditorScopeId(scope_id));

    let selected_menu_item = use_signal(|| EditorMenuItem::MediaView);

    let mut files = use_store(Vec::new);
    let mut tags = use_signal(Vec::new);

    use_context_provider(|| UploadFilesContext::new());
    use_context_provider(move || TagsContext::new(tags));

    use_resource(move || async move {
        println!("Fetching files");
        match pack.read().get_files().await {
            Ok(f) => files.set(f),
            Err(err) => {
                eprintln!("{err}");
            },
        }
    });

    use_resource(move || async move {
        match pack.read().get_all_tags().await {
            Ok(t) => tags.set(t),
            Err(err) => {
                eprintln!("{err}");
            },
        }
    });

    rsx! {
        div { class: "h-screen flex overflow-hidden",
            Menu { selected: selected_menu_item, initially_open: false }
            match selected_menu_item() {
                EditorMenuItem::Options => rsx! {
                    Options {}
                },
                EditorMenuItem::MediaView => rsx! {
                    MediaPage { files }
                },
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorMenuItem {
    Options,
    MediaView,
}

impl std::fmt::Display for EditorMenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Options => write!(f, "Options"),
            Self::MediaView => write!(f, "Media"),
        }
    }
}

impl MenuItem for EditorMenuItem {
    const VARIANTS: &'static [Self] = &[Self::Options, Self::MediaView];

    fn icon(&self) -> Shape {
        match self {
            Self::Options => Shape::Squares2x2,
            Self::MediaView => Shape::Photo,
        }
    }
}

