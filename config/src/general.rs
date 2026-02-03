use dioxus::{core::{Task, spawn_forever}, prelude::*};
use shared::read_pack::read_pack_metadata_async;

use crate::{Config, MediaPack, Pack};
use shared::user_config::{AppConfig, AppConfigStoreExt};

#[component]
pub fn General() -> Element {
    rsx! {
        div {
            class: "flex-1 p-5 flex flex-col gap-4"
        }
    }
}

#[component]
pub fn PackPicker() -> Element {
    let config: Store<AppConfig> = use_context::<Config>().0;
    let mut pack = use_context::<Pack>().0;
    let mut task: Signal<Option<Task>> = use_signal(|| None);

    rsx! {
        input {
            class: "rounded-md hover:bg-gray-200",
            type: "file",
            accept: ".md",
            onchange: move |event: FormEvent| {
                if let Some(file) = event.files().first() {
                    let path = file.path();

                    if let Some(task) = task.take() {
                        task.cancel();
                    }

                    *task.write() = Some(spawn_forever(async move {
                        let file = match tokio::fs::File::open(&path).await {
                            Ok(file) => file,
                            Err(err) => {
                                eprintln!("{err}");
                                return;
                            },
                        };

                        match read_pack_metadata_async(file).await {
                            Ok((header, metadata)) => {
                                *config.pack_path().write() = Some(path);
                                *pack.write() = Some(MediaPack { header, metadata });
                            },
                            Err(err) => {
                                eprintln!("{err}");
                                return;
                            },
                        }
                    }));
                }
            },
            if let Some(pack) = pack.read() {
            } else {
                ""
            }
        }
    }
}
