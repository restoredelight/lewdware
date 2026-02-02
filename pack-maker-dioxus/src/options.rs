use crate::{
    components::{input::Input, label::Label, textarea::Textarea},
    MetadataStore, Pack,
};
use dioxus::prelude::*;
use shared::pack_config::{Metadata, MetadataStoreExt};

#[component]
pub fn Options() -> Element {
    let pack = use_context::<Pack>().0;
    let metadata: Store<Metadata> = use_context::<MetadataStore>().0;

    let save = move || async move {
        if let Err(err) = pack.read().set_metadata(&metadata.read()).await {
            eprintln!("{err}");
        }
        if let Err(err) = pack.read().save_metadata().await {
            eprintln!("{err}");
        }
    };

    let mark_unsaved = move || async move {
        if let Err(err) = pack.read().mark_unsaved().await  {
            eprintln!("{err}");
        }
    };

    rsx! {
        div {
            class: "flex-1 p-4 flex flex-col gap-4",
            Label {
                html_for: "name",
                "Name"
            }
            Input {
                id: "name",
                initial_value: metadata.name(),
                oninput: move |event: FormEvent| {
                    metadata.name().set(event.value());
                    mark_unsaved()
                },
                onchange: move |_| save()
            }
            Label {
                html_for: "creator",
                "Creator"
            }
            Input {
                id: "creator",
                initial_value: metadata.creator(),
                oninput: move |event: FormEvent| {
                    let value = event.value();
                    if value.is_empty() {
                        metadata.creator().set(None);
                    } else {
                        metadata.creator().set(Some(event.value()));
                    }
                    mark_unsaved()
                },
                onchange: move |_| save()
            }
            Label {
                html_for: "description",
                "Description"
            }
            Textarea {
                id: "description",
                placeholder: "Enter description",
                initial_value: metadata.description(),
                oninput: move |event: FormEvent| {
                    let value = event.value();
                    if value.is_empty() {
                        metadata.description().set(None);
                    } else {
                        metadata.description().set(Some(event.value()));
                    }
                    mark_unsaved()
                },
                onchange: move |_| save()
            }
            Label {
                html_for: "version",
                "Version"
            }
            Input {
                id: "version",
                placeholder: "Enter version (e.g. 0.0.1)",
                initial_value: metadata.version(),
                oninput: move |event: FormEvent| {
                    let value = event.value();
                    if value.is_empty() {
                        metadata.version().set(None);
                    } else {
                        metadata.version().set(Some(event.value()));
                    }
                    mark_unsaved()
                },
                onchange: move |_| save()
            }
        }
    }
}
