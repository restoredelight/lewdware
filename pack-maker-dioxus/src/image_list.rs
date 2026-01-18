use std::collections::HashSet;

use dioxus::{html::input_data::MouseButton, prelude::*};
use dioxus_desktop::tao;
use dioxus_heroicons::{solid::Shape, Icon};
use shared::encode::FileInfo;

use crate::{
    components::{
        context_menu::{ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger},
        dialog::{DialogContent, DialogRoot, DialogTitle},
    },
    use_port,
    utils::use_global_key_listener,
    Pack,
};

#[derive(Store, Clone)]
pub struct Media {
    pub id: u64,
    pub file_info: FileInfo,
    pub file_name: String,
    pub selected: bool,
}

const ITEM_WIDTH: u32 = 200;
const ITEM_HEIGHT: u32 = 270;

#[component]
pub fn ImageList() -> Element {
    let pack = use_context::<Pack>();
    let port = match use_port() {
        Some(x) => x.to_string(),
        None => "".to_string(),
    };

    let mut files = use_store::<Vec<Media>>(|| vec![]);
    let mut selected: Signal<HashSet<usize>> = use_signal(|| HashSet::new());
    let mut primary: Signal<Option<usize>> = use_signal(|| None);
    let mut opened_media: _ = use_signal(|| None);

    use_resource(move || async move {
        files.set(pack.0.read().get_files().await.unwrap());
    });

    let mut scroll_top = use_signal(|| 0.0);
    let mut width = use_signal(|| 0.0);
    let mut height = use_signal(|| 0.0);

    let cols = use_memo(move || width() as u32 / ITEM_WIDTH);
    let rows = use_memo(move || {
        if cols() == 0 {
            0
        } else {
            files.len() as u32 / cols()
        }
    });

    let row_range = use_memo(move || {
        let top_row = (((scroll_top() / ITEM_HEIGHT as f64).floor() as isize) - 5).max(0) as usize;
        let total_rows = (height() / ITEM_HEIGHT as f64).floor() as usize + 1;
        let bottom_row = (top_row + total_rows + 10).min(rows() as usize);
        top_row..bottom_row
    });

    let mut clear_selected = move || {
        for i in selected.write().drain() {
            if let Some(file) = files.get(i) {
                file.selected().set(false);
            }
        }
    };

    let mut change_primary = move |x: usize| {
        if let Some(file) = files.get(x) {
            clear_selected();
            primary.set(Some(x));
            selected.insert(x);
            file.selected().set(true);
        }
    };

    block_keybinds(primary);

    use_global_key_listener(move |key| {
        if opened_media().is_some() {
            return;
        }

        if let Some(primary) = primary() {
            match key {
                tao::keyboard::Key::ArrowLeft => {
                    if primary > 0 {
                        let new_primary = primary - 1;
                        change_primary(new_primary);
                        scroll_into_view(new_primary);
                    }
                }
                tao::keyboard::Key::ArrowRight => {
                    if primary < files.len() - 1 {
                        let new_primary = primary + 1;
                        change_primary(new_primary);
                        scroll_into_view(new_primary);
                    }
                }
                tao::keyboard::Key::ArrowUp => {
                    if primary >= cols() as usize {
                        let new_primary = primary - cols() as usize;
                        change_primary(new_primary);
                        scroll_into_view(new_primary);
                    }
                }
                tao::keyboard::Key::ArrowDown => {
                    if primary < files.len() - cols() as usize {
                        let new_primary = primary + cols() as usize;
                        change_primary(new_primary);
                        scroll_into_view(new_primary);
                    }
                }
                tao::keyboard::Key::Enter => {
                    opened_media.set(Some(primary));
                }
                _ => {}
            }
        }

        match key {
            tao::keyboard::Key::Escape => {
                clear_selected();
                primary.set(None);
            }
            _ => {}
        }
    });

    rsx! {
        MediaViewDialog { files, index: opened_media, port: port.clone() }
        div {
            // height: todo!(),
            overflow: "scroll",
            class: "h-full",
            onresize: move |x| async move {
                if let Ok(rect) = x.get_content_box_size() {
                    println!("{}", rect.width);
                    println!("{}", rect.height);
                    width.set(rect.width);
                    height.set(rect.height);
                }
            },
            onscroll: move |x| {
                println!("Scroll event");
                println!("{}", x.scroll_top());
                scroll_top.set(x.scroll_top());
            },
            onclick: move |event| {
                match event.trigger_button() {
                    Some(MouseButton::Primary) => {
                        clear_selected();
                        primary.set(None);
                    }
                    Some(MouseButton::Secondary) => todo!(),
                    _ => {}
                }
            },
            div {
                height: "{rows() * ITEM_HEIGHT}px",
                position: "relative",
                overflow: "hidden",
                for row_index in row_range() {
                    div {
                        class: "flex",
                        key: "{row_index}",
                        position: "absolute",
                        top: "{row_index as u32 * ITEM_HEIGHT}px",
                        left: 0,
                        for col_index in (0..cols()) {
                            {
                                let index = row_index * cols() as usize + col_index as usize;
                                rsx! {
                                    if let Some(file) = files.get(index) {
                                        Thumbnail {
                                            index,
                                            key: "{index}",
                                            file,
                                            port: port.clone(),
                                            onclick: move |event: MouseEvent| {
                                                match event.trigger_button() {
                                                    Some(MouseButton::Primary) => {
                                                        event.stop_propagation();
                                                        let modifiers = event.modifiers();

                                                        if modifiers.shift() {
                                                            if let Some(primary) = primary() {
                                                                let range = if primary < index {
                                                                    primary..=index
                                                                } else {
                                                                    index..=primary
                                                                };
                                                                for i in range {
                                                                    if let Some(file) = files.get(i) {
                                                                        selected.insert(i);
                                                                        file.selected().set(true);
                                                                    }
                                                                }
                                                            }
                                                        } else if !modifiers.ctrl() {
                                                            clear_selected();
                                                        }
                                                        selected.insert(index);
                                                        primary.set(Some(index));
                                                        file.selected().set(true);
                                                    }
                                                    Some(MouseButton::Secondary) => todo!(),
                                                    _ => {}
                                                }
                                            },
                                            ondoubleclick: move |_| {
                                                opened_media.set(Some(index));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Sidebar() -> Element {
    rsx!()
}

#[component]
fn Thumbnail(
    index: usize,
    file: Store<Media>,
    port: String,
    onclick: EventHandler<MouseEvent>,
    ondoubleclick: EventHandler<MouseEvent>,
) -> Element {
    if matches!(*file.file_info().read(), FileInfo::Audio { .. }) {}

    rsx! {
        ContextMenu {
            ContextMenuTrigger {
                class: "mx-[25px] my-[10px]",
                div {
                    id: "thumbnail-{index}",
                    width: "150px",
                    height: "250px",
                    display: "flex",
                    flex_direction: "column",
                    class: "group image-container",
                    "data-selected": if file.selected()() { "true" },
                    onclick,
                    ondoubleclick,
                    onkeydown: move |x| {
                        println!("Key down event");
                        if x.key() == Key::Enter {
                            x.stop_propagation();
                        }
                    },
                    div {
                        width: "150px",
                        height: "150px",
                        class: "flex place-content-center",
                        if matches!(*file.file_info().read(), FileInfo::Audio { .. }) {
                            Icon {
                                icon: Shape::MusicalNote,
                                size: 100,
                                class: "m-auto",
                            }
                        } else {
                            img {
                                loading: "lazy",
                                max_width: "150px",
                                max_height: "150px",
                                margin: "auto",
                                box_shadow: "0 0 5px",
                                class: "rounded-xs ",
                                class: if file.selected()() { "image-selected" } else { "group-hover:brightness-110" },
                                src: "http://localhost:{port}/thumbnail/{file.id()}",
                            }
                        }
                    }
                    p {
                        class: "multiline-ellipses ",
                        class: if file.selected()() { "bg-sky-400" } else { "group-hover:bg-sky-200" },
                        max_width: "100%",
                        title: file.file_name(),
                        {file.file_name()}
                    }
                }
            }
            ContextMenuContent {
                ContextMenuItem {
                    index: 0usize,
                    value: "",
                    on_select: move |_| {

                    },
                    "Select all"
                }
            }
        }
    }
}

#[component]
fn MediaViewDialog(
    files: Store<Vec<Media>>,
    index: Signal<Option<usize>>,
    port: String,
) -> Element {
    use_global_key_listener(move |key| {
        if let Some(idx) = index() {
            match key {
                tao::keyboard::Key::ArrowLeft => {
                    if idx > 0 {
                        index.set(Some(idx - 1));
                    }
                }
                tao::keyboard::Key::ArrowRight => {
                    if idx < files.len() - 1 {
                        index.set(Some(idx + 1));
                    }
                }
                _ => {}
            }
        }
    });

    rsx! {
        DialogRoot {
            open: index().is_some(),
            on_open_change: move |v: bool| {
                if !v {
                    index.set(None);
                }
            },
            DialogContent {
                button {
                    class: "dialog-close",
                    r#type: "button",
                    aria_label: "Close",
                    tabindex: if index().is_some() { "0" } else { "-1" },
                    onclick: move |_| index.set(None),
                    Icon { icon: Shape::XMark }
                }
                if let Some(index) = index() {
                    if let Some(file) = files.get(index) {
                        DialogTitle {
                            "{file.file_name()}"
                        }
                        match *file.file_info().read() {
                            FileInfo::Image { width, height, .. } => rsx! {
                                img {
                                    src: "http://localhost:{port}/file/{file.id()}",
                                    aspect_ratio: "{width} / {height}"
                                }
                            },
                            FileInfo::Video { width, height, .. } => rsx! {
                                video {
                                    src: "http://localhost:{port}/file/{file.id()}",
                                    controls: true,
                                    aspect_ratio: "{width} / {height}"
                                }
                            },
                            FileInfo::Audio { .. } => rsx! {
                                audio {
                                    controls: true,
                                    src: "http://localhost:{port}/file/{file.id()}"
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}

fn block_keybinds(primary: Signal<Option<usize>>) {
    let any_selected = use_memo(move || primary().is_some());

    let js_handle = use_signal(move || {
        document::eval(
            r#"
                const controller = new AbortController();
                let block_events = false;
                window.addEventListener('keydown', (event) => {
                    if (block_events && ['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Enter'].includes(event.key)) {
                        console.log('Blocking events');
                        event.preventDefault();
                    }
                }, { signal: controller.signal });

                while (true) {
                    const message = await dioxus.recv();

                    if (message === 'close') {
                        controller.abort();
                    } else {
                        block_events = message;
                    }
                }
            "#,
        )
    });

    use_effect(move || {
        if let Err(err) = js_handle().send(any_selected()) {
            eprintln!("{err}");
        }
    });

    use_drop(move || {
        if let Err(err) = js_handle().send("close".to_string()) {
            eprintln!("{err}");
        }
    });
}

fn scroll_into_view(index: usize) {
    document::eval(&format!(
        r#"
            const el = document.getElementById('thumbnail-{index}');
            if (el) {{
                el.scrollIntoView({{ behavior: 'smooth', block: 'nearest' }});
            }}
        "#
    ));
}
