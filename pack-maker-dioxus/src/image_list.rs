use dioxus_primitives::ContentAlign;
use std::collections::HashSet;
use std::io::Write;

use dioxus::{core::Task, html::input_data::MouseButton, prelude::*, stores::index::IndexWrite};
use dioxus_desktop::tao;
use dioxus_heroicons::{solid::Shape, Icon};
use dioxus_primitives::{checkbox::CheckboxState, ContentSide};
use shared::encode::FileInfo;
use tempfile::NamedTempFile;

use crate::{
    components::{
        button::{Button, ButtonVariant},
        checkbox::Checkbox,
        context_menu::{ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger},
        dialog::{DialogContent, DialogRoot},
        dropdown_menu::{DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger},
        input::Input,
        label::Label,
        popover::{PopoverContent, PopoverRoot, PopoverTrigger},
        progress::{Progress, ProgressIndicator},
        select::{Select, SelectItemIndicator, SelectList, SelectOption, SelectTrigger},
        separator::Separator,
    },
    encode::{explore_folder, process_files, ProcessFilesError, ProcessFilesErrorType},
    pack::{FileData, MediaPack},
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
    pub hash: String,
}

enum Direction {
    Left,
    Right,
    Up,
    Down,
}

const ITEM_WIDTH: u32 = 170;
const ITEM_HEIGHT: u32 = 220;
const PADDING: u32 = 10;

#[derive(Clone, Copy)]
pub struct UploadFilesContext {
    progress_bar_open: Signal<bool>,
    processing: Signal<usize>,
    processed: Signal<usize>,
    currently_processing: Signal<String>,
    skipped: Signal<usize>,
    errors: Signal<Vec<ProcessFilesError>>,
    tasks: Signal<Vec<Task>>,
}

impl UploadFilesContext {
    fn new() -> Self {
        Self {
            progress_bar_open: Signal::new(false),
            processing: Signal::new(0),
            processed: Signal::new(0),
            currently_processing: Signal::new(String::new()),
            skipped: Signal::new(0),
            errors: Signal::new(Vec::new()),
            tasks: Signal::new(Vec::new()),
        }
    }

    fn reset(&mut self) {
        self.processed.set(0);
        self.processing.set(0);
        self.currently_processing.set(String::new());
        self.skipped.set(0);
        self.errors.write().clear();
        self.tasks.write().clear();
    }

    fn reset_and_close(&mut self) {
        self.reset();
        self.progress_bar_open.set(false);
    }

    fn cancel_tasks(&mut self) {
        for task in self.tasks.write().drain(..) {
            task.cancel();
        }

        self.reset_and_close();
    }

    fn reset_if_finished(&mut self) {
        if !self.is_processing() {
            self.reset();
        }
    }

    fn add_processing(&mut self, n: usize) {
        self.processing += n;
        self.progress_bar_open.set(true);
    }

    fn add_task(&mut self, task: Task) {
        self.tasks.push(task);
    }

    pub fn set_currently_processing(&mut self, currently_processing: String) {
        *self.currently_processing.write() = currently_processing;
    }

    pub fn increment_processed(&mut self) {
        self.processed += 1;
    }

    pub fn handle_error(&mut self, error: ProcessFilesError) {
        match error.error_type {
            ProcessFilesErrorType::Skipped => {
                self.skipped += 1;
            }
            _ => {
                self.errors.push(error);
            }
        }
    }

    fn progress_bar_open(&self) -> bool {
        (self.progress_bar_open)()
    }

    fn processing(&self) -> usize {
        (self.processing)()
    }

    fn processed(&self) -> usize {
        (self.processed)()
    }

    fn skipped(&self) -> usize {
        (self.skipped)()
    }

    fn is_processing(&self) -> bool {
        self.processed() != self.processing()
    }
}

#[derive(Clone, Copy)]
struct ImageListContext {
    files: Store<Vec<Media>>,
    selected: Signal<HashSet<usize>>,
    primary: Signal<Option<usize>>,
    opened: Signal<Option<usize>>,
    search_str: Signal<String>,
    media_type: Signal<MediaType>,
    filtered_indices: Memo<Vec<usize>>,
}

struct IndexDependentState {
    selected: HashSet<usize>,
    primary: Option<usize>,
    opened: Option<usize>,
}

impl ImageListContext {
    pub fn new(files: Store<Vec<Media>>) -> Self {
        let search_str = Signal::new(String::new());
        let media_type = Signal::new(MediaType::All);

        let selected_indices = Memo::new(move || {
            (1..files.len())
                .filter(|i| {
                    let file = files.get(*i).unwrap();

                    (search_str.is_empty() || file.file_name().contains(&search_str.read()))
                        && match media_type() {
                            MediaType::All => true,
                            MediaType::Image => {
                                matches!(*file.file_info().read(), FileInfo::Image { .. })
                            }
                            MediaType::Video => {
                                matches!(*file.file_info().read(), FileInfo::Video { .. })
                            }
                            MediaType::Audio => {
                                matches!(*file.file_info().read(), FileInfo::Audio { .. })
                            }
                        }
                })
                .collect::<Vec<_>>()
        });

        Self {
            files,
            selected: Signal::new(HashSet::new()),
            primary: Signal::new(None),
            opened: Signal::new(None),
            search_str,
            media_type,
            filtered_indices: selected_indices,
        }
    }

    fn navigate_grid(&mut self, direction: Direction, cols: usize, height: u32) {
        if let Some(primary) = self.primary() {
            if let Some(new_primary) = match direction {
                Direction::Left if primary > 0 => Some(primary - 1),
                Direction::Right if primary < self.filtered_indices.len() - 1 => Some(primary + 1),
                Direction::Up if primary >= cols => Some(primary - cols),
                Direction::Down if (primary + cols) < self.filtered_indices.len() => {
                    Some(primary + cols)
                }
                _ => None,
            } {
                if let Some(file) = self.get(new_primary) {
                    println!("Moving primary");
                    self.clear_selected();
                    self.primary.set(Some(new_primary));
                    self.selected.insert(new_primary);
                    file.selected().set(true);
                    self.scroll_into_view(new_primary, cols, height);
                }
            }
        }
    }

    fn scroll_into_view(&self, index: usize, cols: usize, height: u32) {
        if let Some(file) = self.get(index) {
            let id = file.id()();
            let row = index / cols;
            let max_scroll_top = row as u32 * ITEM_HEIGHT + PADDING;
            let min_scroll_top = (max_scroll_top + ITEM_HEIGHT).saturating_sub(height);

            // Hopefully, the item is rendered in the virtualized grid, in which case we just
            // scroll to it. However, if it isn't (which can happen e.g. if the user holds the down
            // button), we have to compute the scroll position manually.
            document::eval(&format!(
                r#"
                    const el = document.getElementById('thumbnail-{id}');
                    if (el) {{
                        console.log("Scrolling into view");
                        el.scrollIntoView({{ behavior: 'smooth', block: 'nearest' }});
                    }} else {{
                        let grid = document.querySelector('#media-grid');
                        if (grid.scrollTop < {min_scroll_top}) {{
                            grid.scroll({{ top: {min_scroll_top}, left: 0, behavior: 'smooth' }});
                        }} else if (grid.scrollTop > {max_scroll_top}) {{
                            grid.scroll({{ top: {max_scroll_top}, left: 0, behavior: 'smooth' }});
                        }}
                    }}
                "#
            ));
        }
    }

    fn get(
        &self,
        index: usize,
    ) -> Option<Store<Media, IndexWrite<usize, WriteSignal<Vec<Media>>>>> {
        self.filtered_indices
            .get(index)
            .and_then(|i| self.files.get(*i))
    }

    fn clear_selected(&mut self) {
        for i in self.selected.write().drain() {
            if let Some(file) = self
                .filtered_indices
                .get(i)
                .and_then(|i| self.files.get(*i))
            {
                file.selected().set(false);
            }
        }
    }

    fn primary(&self) -> Option<usize> {
        (self.primary)()
    }

    fn opened(&self) -> Option<usize> {
        (self.opened)()
    }

    fn media_type(&self) -> MediaType {
        (self.media_type)()
    }

    fn extract_index_dependent_state(&self) -> IndexDependentState {
        let filtered_indices = self.filtered_indices.read();

        IndexDependentState {
            selected: self
                .selected
                .read()
                .iter()
                .filter_map(|i| filtered_indices.get(*i))
                .cloned()
                .collect(),
            primary: self
                .primary()
                .and_then(|i| filtered_indices.get(i))
                .cloned(),
            opened: self.opened().and_then(|i| filtered_indices.get(i)).cloned(),
        }
    }

    fn update_index_dependent_state(&mut self, mut state: IndexDependentState) {
        let mut selected = HashSet::new();
        let mut primary = None;
        let mut opened = None;

        for (filtered_index, index) in self.filtered_indices.read().iter().enumerate() {
            if state.selected.remove(index) {
                selected.insert(filtered_index);
            }

            if state.primary.is_some_and(|i| *index == i) {
                primary = Some(filtered_index);
            }

            if state.opened.is_some_and(|i| *index == i) {
                opened = Some(filtered_index);
            }
        }

        // The values remaining in `state.selected` are the values which did not appear in
        // `filtered_indices`. Hence, they should not be selected.
        for stale_selected in state.selected {
            if let Some(file) = self.files.get(stale_selected) {
                file.selected().set(false);
            }
        }

        // Some logic (e.g. the sidebar) depends on the fact that if `primary` is `None`, then no
        // items are selected. If `primary` was a stale value but there are still some selected
        // items, then we pick a random one.
        if primary.is_none() {
            if let Some(first_selected) = selected.iter().next() {
                primary = Some(*first_selected);
            }
        }

        *self.selected.write() = selected;
        self.primary.set(primary);
        self.opened.set(opened);
    }

    fn set_search_string(&mut self, str: String) {
        if *self.search_str.read() != str {
            let state = self.extract_index_dependent_state();

            *self.search_str.write() = str;

            self.update_index_dependent_state(state);
        }
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        if self.media_type() != media_type {
            let state = self.extract_index_dependent_state();

            self.media_type.set(media_type);

            self.update_index_dependent_state(state);
        }
    }

    fn remove_files(&mut self, files: HashSet<usize>) {
        let filtered_indices = self.filtered_indices.read();
        let selected_indices: HashSet<_> = files
            .into_iter()
            .filter_map(|i| filtered_indices.get(i))
            .collect();

        let mut i = 0;
        self.files.retain(|_| {
            let is_selected = selected_indices.contains(&i);
            i += 1;
            !is_selected
        });

        self.selected.write().clear();
        self.primary.set(None);
    }

    fn select_all(&mut self) {
        self.selected
            .write()
            .extend(self.filtered_indices.read().iter());

        if self.primary.read().is_none() {
            *self.primary.write() = if self.filtered_indices.is_empty() {
                None
            } else {
                Some(0)
            };
        }
    }
}

#[component]
pub fn ImageList() -> Element {
    let pack = use_context::<Pack>().0;
    let mut files = use_store::<Vec<Media>>(|| vec![]);

    use_resource(move || async move {
        println!("Fetching files");
        files.set(pack.read().get_files().await.unwrap());
    });

    use_context_provider(move || ImageListContext::new(files));
    use_context_provider(|| UploadFilesContext::new());

    rsx! {
        div {
            class: "h-full flex-1 flex flex-col overflow-hidden",
            MediaViewDialog {}
            Header { pack, files }
            div { class: "flex flex-1 overflow-hidden",
                ImageView {}
                Sidebar {}
            }
            ProgressBar { files }
        }
    }
}

#[component]
fn ImageView() -> Element {
    let port = match use_port() {
        Some(x) => x.to_string(),
        None => "".to_string(),
    };
    let mut context = use_context::<ImageListContext>();

    let mut scroll_top = use_signal(|| 0.0);
    let mut width = use_signal(|| 0.0);
    let mut height = use_signal(|| 0.0);

    let cols = use_memo(move || {
        if (width() as u32) < PADDING * 2 {
            0
        } else {
            ((width() as u32 - PADDING * 2) / ITEM_WIDTH).max(1)
        }
    });

    let rows = use_memo(move || {
        if cols() == 0 {
            0
        } else {
            (context.filtered_indices.len() as u32).div_ceil(cols())
        }
    });

    // Make sure we are never scrolled too far down (which can happen if e.g. the window is made
    // bigger or a filter is applied).
    use_effect(move || {
        let total_height = rows() as f64 * ITEM_HEIGHT as f64 + PADDING as f64 * 2.0;
        let max_scroll = (total_height - height()).max(0.0);
        if *scroll_top.peek() > max_scroll {
            // Force the browser scrollbar to match our new scroll_top
            let js = format!(
                "
                let grid = document.querySelector('#media-grid');
                grid.scrollTop = {};
            ",
                max_scroll
            );
            document::eval(&js);

            scroll_top.set(max_scroll);
        }
    });

    let row_range = use_memo(move || {
        let top_row = (((scroll_top() / ITEM_HEIGHT as f64).floor() as isize) - 5).max(0) as usize;
        let total_rows = (height() / ITEM_HEIGHT as f64).floor() as usize + 1;
        let bottom_row = (top_row + total_rows + 10).min(rows() as usize);
        top_row..bottom_row
    });

    block_keybinds(context.primary);

    use_global_key_listener(move |key| {
        if context.opened().is_some() {
            return;
        }

        match key {
            tao::keyboard::Key::ArrowLeft => {
                context.navigate_grid(Direction::Left, cols() as usize, height() as u32)
            }
            tao::keyboard::Key::ArrowRight => {
                context.navigate_grid(Direction::Right, cols() as usize, height() as u32)
            }
            tao::keyboard::Key::ArrowUp => {
                context.navigate_grid(Direction::Up, cols() as usize, height() as u32)
            }
            tao::keyboard::Key::ArrowDown => {
                context.navigate_grid(Direction::Down, cols() as usize, height() as u32)
            }
            tao::keyboard::Key::Enter => {
                if let Some(primary) = context.primary() {
                    context.opened.set(Some(primary));
                }
            }
            tao::keyboard::Key::Escape => {
                context.clear_selected();
                context.primary.set(None);
            }
            _ => {}
        }
    });

    use_effect(move || {
        row_range.read();
        println!("Row range updated");
    });

    rsx! {
        div {
            id: "media-grid",
            role: "grid",
            aria_rowcount: "{rows()}",
            aria_colcount: "{cols()}",
            padding: "{PADDING}px",
            class: "flex-1 overflow-y-auto select-none",
            onresize: move |x| async move {
                println!("Updating size");
                if let Ok(rect) = x.get_content_box_size() {
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
                println!("Click event");
                match event.trigger_button() {
                    Some(MouseButton::Primary) => {
                        context.clear_selected();
                        context.primary.set(None);
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
                        class: "flex justify-around w-full",
                        key: "{row_index}",
                        position: "absolute",
                        top: "{row_index as u32 * ITEM_HEIGHT}px",
                        left: 0,
                        for col_index in (0..cols()) {
                            {
                                let index = row_index * cols() as usize + col_index as usize;
                                rsx! {
                                    if let Some(file) = context.get(index) {
                                        Thumbnail {
                                            key: "{file.id()}",
                                            file,
                                            port: port.clone(),
                                            onclick: move |event: MouseEvent| {
                                                println!("Click event");
                                                match event.trigger_button() {
                                                    Some(MouseButton::Primary) => {
                                                        event.stop_propagation();
                                                        let modifiers = event.modifiers();

                                                        if modifiers.shift() {
                                                            if let Some(primary) = context.primary() {
                                                                let range = if primary < index {
                                                                    primary..=index
                                                                } else {
                                                                    index..=primary
                                                                };

                                                                for i in range {
                                                                    if let Some(file) = context.get(i) {
                                                                        context.selected.insert(i);
                                                                        file.selected().set(true);
                                                                    }
                                                                }
                                                            }
                                                        } else if !modifiers.ctrl() {
                                                            context.clear_selected();
                                                        }
                                                        context.selected.insert(index);
                                                        context.primary.set(Some(index));
                                                        file.selected().set(true);
                                                    }
                                                    Some(MouseButton::Secondary) => todo!(),
                                                    _ => {}
                                                }
                                            },
                                            ondoubleclick: move |_| {
                                                println!("Double click");
                                                context.opened.set(Some(index));
                                            },
                                        }
                                    } else {
                                        div { width: "{ITEM_WIDTH}px", height: "{ITEM_HEIGHT}px" }
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
fn ProgressBar(files: Store<Vec<Media>>) -> Element {
    let mut context = use_context::<UploadFilesContext>();

    let is_processing = use_memo(move || context.is_processing());

    let percentage = use_memo(move || {
        if context.processing() == 0 {
            0
        } else {
            (100.0 * context.processed() as f64 / context.processing() as f64).round() as u32
        }
    });

    let mut errors_open = use_signal(|| false);

    rsx! {
        PopoverRoot { open: errors_open(), on_open_change: move |v| errors_open.set(v),
            div {
                class: "flex justify-between items-center px-5 py-2 bg-gray-50 border-t border-gray-300 overflow-hidden",
                class: if !context.progress_bar_open() { "hidden" },
                div { class: "flex-2 min-w-0",
                    p { class: "truncate",
                        if is_processing() {
                            "Processing ({context.processed}/{context.processing})"
                        } else {
                            "Finished {context.processed} files"
                        }
                    }
                    p {
                        class: "text-sm text-gray-700 text truncate",
                        title: if is_processing() { "{context.currently_processing}" },
                        if is_processing() {
                            "{context.currently_processing}"
                        }
                    }
                }
                div { class: "flex-1 flex gap-5 items-center justify-center px-4 min-w-[150px]",
                    Progress {
                        value: context.processed() as f64,
                        max: context.processing() as f64,
                        ProgressIndicator {}
                    }
                    "{percentage}%"
                }
                div { class: "flex gap-5 items-center justify-end flex-none",
                    if context.skipped() > 0 {
                        p { class: "hidden md:block text-sm text-gray-700",
                            "{context.skipped} skipped"
                        }
                    }
                    if !context.errors.read().is_empty() {
                        PopoverTrigger {
                            div { class: "flex items-center",
                                Icon { size: 20, icon: Shape::InformationCircle }
                                "{context.errors.read().len()}"
                            }
                        }
                        PopoverContent { side: ContentSide::Top, align: ContentAlign::End,
                            div { class: "flex items-center justify-between px-4 py-3 bg-gray-50 border-b border-gray-100",
                                h3 { class: "text-sm font-semibold text-gray-900",
                                    "Errors ({context.errors.read().len()})"
                                }
                                button {
                                    class: "p-1 rounded-md hover:bg-gray-200 text-gray-500 transition-colors",
                                    r#type: "button",
                                    aria_label: "Close",
                                    onclick: move |_| errors_open.set(false),
                                    Icon { icon: Shape::XMark, size: 16 }
                                }
                            }
                            div { class: "max-h-72 overflow-y-auto divide-y divide-gray-100 scrollbar-thin scrollbar-thumb-gray-300",

                                for (i , error) in context.errors.read().iter().enumerate() {
                                    div {
                                        key: "{i}",
                                        class: "p-4 hover:bg-red-50/50 transition-colors group",

                                        div { class: "flex gap-3",
                                            div { class: "mt-0.5 text-red-500",
                                                Icon {
                                                    icon: Shape::ExclamationCircle,
                                                    size: 18,
                                                }
                                            }
                                            div { class: "flex-1 min-w-0",
                                                h4 {
                                                    class: "text-xs font-medium text-gray-900 truncate mb-1",
                                                    title: "{error.path.display()}",
                                                    "{error.path.display()}"
                                                }
                                                p { class: "text-xs text-red-600 leading-relaxed",
                                                    "{error.error_type}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Button { variant: if is_processing() { ButtonVariant::Destructive } else { ButtonVariant::Primary },
                        div {
                            class: "flex gap-2 items-center",
                            onclick: move |_| {
                                if is_processing() {
                                    context.cancel_tasks();
                                } else {
                                    context.reset_and_close();
                                }
                            },
                            Icon { icon: Shape::XMark, size: 20 }
                            if is_processing() {
                                "Cancel"
                            } else {
                                "Close"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Header(pack: ReadSignal<MediaPack>, files: Store<Vec<Media>>) -> Element {
    let mut context = use_context::<ImageListContext>();

    rsx! {
        div { class: "flex justify-between items-center py-2 px-5 bg-gray-50 border-b border-gray-300",
            AddFilesButton { pack, files }
            p { class: "flex", "{files.len()} items" }
            Input {
                oninput: move |event: FormEvent| {
                    context.set_search_string(event.value());
                },
                placeholder: "Search...",
            }
            SelectMediaType {}
        }
    }
}

#[component]
fn AddFilesButton(pack: ReadSignal<MediaPack>, files: Store<Vec<Media>>) -> Element {
    let mut context = use_context::<UploadFilesContext>();

    let mut skip_duplicates = use_signal(|| true);
    let mut recursive = use_signal(|| true);

    rsx! {
        DropdownMenu {
            DropdownMenuTrigger {
                div { class: "flex items-center gap-2",
                    Icon { icon: Shape::Plus, size: 20, class: "my-auto" }
                    "Add files"
                }
            }
            DropdownMenuContent {
                DropdownMenuItem {
                    index: 0usize,
                    value: "",
                    on_select: move |_: String| async move {
                        if let Some(file_handles) = rfd::AsyncFileDialog::new()
                            .set_title("Select files")
                            .pick_files()
                            .await
                        {

                            if !file_handles.is_empty() {
                                context.reset_if_finished();
                                context.add_processing(file_handles.len());
                                let paths = file_handles
                                    .iter()
                                    .map(|file| file.path().to_path_buf())
                                    .collect();
                                let task = spawn(
                                    process_files(pack, paths, context, files, skip_duplicates()),
                                );
                                context.add_task(task);
                            }
                        }
                    },
                    div { class: "flex gap-2",
                        Icon { size: 20, icon: Shape::Document }
                        "Select files"
                    }
                }
                DropdownMenuItem {
                    index: 0usize,
                    value: "",
                    on_select: move |_: String| async move {
                        if let Some(folder) = rfd::AsyncFileDialog::new()
                            .set_title("Select a folder")
                            .pick_folder()
                            .await
                        {

                            let recursive = recursive();
                            let paths = match tokio::task::spawn_blocking(move || explore_folder(
                                    folder.path(),
                                    recursive,
                                ))
                                .await
                            {
                                Ok(paths) => paths,
                                Err(err) => {
                                    eprintln!("{err}");
                                    return;
                                }
                            };
                            if !paths.is_empty() {
                                context.reset_if_finished();
                                context.add_processing(paths.len());
                                let task = spawn(
                                    process_files(pack, paths, context, files, skip_duplicates()),
                                );
                                context.add_task(task);
                            }
                        }
                    },
                    div { class: "flex gap-2",
                        Icon { size: 20, icon: Shape::Folder }
                        "Select folder"
                    }
                }
                div {
                    onclick: move |event| {
                        event.stop_propagation();
                        event.prevent_default();
                    },
                    div { class: "py-2", Separator {} }
                    div { class: "flex px-3 py-2 flex gap-2",
                        Checkbox {
                            id: "skip-duplicates",
                            checked: if skip_duplicates() { CheckboxState::Checked } else { CheckboxState::Unchecked },
                            on_checked_change: move |state| {
                                match state {
                                    CheckboxState::Checked => skip_duplicates.set(true),
                                    CheckboxState::Unchecked => skip_duplicates.set(false),
                                    CheckboxState::Indeterminate => {}
                                }
                            },
                        }
                        Label { html_for: "skip-duplicates", "Skip duplicates" }
                    }
                    div { class: "flex px-3 pt-2 pb-3 flex gap-2",
                        Checkbox {
                            id: "recursive",
                            checked: if recursive() { CheckboxState::Checked } else { CheckboxState::Unchecked },
                            on_checked_change: move |state| {
                                match state {
                                    CheckboxState::Checked => recursive.set(true),
                                    CheckboxState::Unchecked => recursive.set(false),
                                    CheckboxState::Indeterminate => {}
                                }
                            },
                        }
                        Label { html_for: "recursive", "Include subfolders" }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct MediaTypes {
    images: bool,
    videos: bool,
    audio: bool,
}

impl MediaTypes {
    const ALL: Self = Self {
        images: true,
        videos: true,
        audio: true,
    };

    const NONE: Self = Self {
        images: false,
        videos: false,
        audio: false,
    };
}

impl std::fmt::Display for MediaTypes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            &Self::ALL => write!(f, "All")?,
            &Self::NONE => write!(f, "None")?,
            _ => {
                if self.images {
                    write!(f, "Images")?
                }

                if self.images {
                    write!(f, "Videos")?
                }

                if self.audio {
                    write!(f, "Audio")?
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MediaType {
    All,
    Image,
    Video,
    Audio,
}

impl std::fmt::Display for MediaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => write!(f, "All"),
            Self::Image => write!(f, "Image"),
            Self::Video => write!(f, "Video"),
            Self::Audio => write!(f, "Audio"),
        }
    }
}

#[component]
pub fn SelectMediaType() -> Element {
    let mut context = use_context::<ImageListContext>();

    let options = [
        MediaType::All,
        MediaType::Image,
        MediaType::Video,
        MediaType::Audio,
    ]
    .iter()
    .enumerate()
    .map(|(i, media_type)| {
        rsx! {
            SelectOption::<MediaType> { index: i, value: *media_type, text_value: "{media_type}",
                "{media_type}"
                SelectItemIndicator {}
            }
        }
    });

    rsx! {
        Select::<MediaType> {
            on_value_change: move |media_type: Option<MediaType>| {
                if let Some(media_type) = media_type {
                    context.set_media_type(media_type);
                }
            },
            SelectTrigger {
                div { "File type: {context.media_type()}" }
            }
            SelectList { {options} }
        }
    }
}

#[component]
fn Sidebar() -> Element {
    let port = match use_port() {
        Some(x) => x.to_string(),
        None => "".to_string(),
    };
    let context = use_context::<ImageListContext>();

    rsx! {
        div {
            class: "border-l border-gray-300 bg-gray-50 overflow-y-auto",
            width: "20%",
            if let Some(index) = context.primary() {
                if let Some(file) = context.get(index) {
                    div { class: "p-4 flex flex-col gap-4",
                        div {
                            class: "flex items-center justify-center bg-gray-100 rounded-lg",
                            min_height: "200px",
                            match *file.file_info().read() {
                                FileInfo::Image { .. } => rsx! {
                                    img {
                                        src: "http://localhost:{port}/preview/{file.id()}?hash={file.hash()}",
                                        class: "max-w-full max-h-[400px] object-contain rounded-lg",
                                        alt: "{file.file_name()}",
                                    }
                                },
                                FileInfo::Video { .. } => rsx! {
                                    img {
                                        src: "http://localhost:{port}/preview/{file.id()}?hash={file.hash()}",
                                        class: "max-w-full max-h-[400px] object-contain rounded-lg",
                                        alt: "{file.file_name()}",
                                    }
                                },
                                FileInfo::Audio { .. } => rsx! {
                                    Icon { icon: Shape::MusicalNote, size: 120, class: "text-gray-400" }
                                },
                            }
                        }
                        div { class: "flex flex-col gap-2",
                            h3 {
                                class: "font-semibold text-lg break-words",
                                title: if context.selected.read().len() == 1 { "{file.file_name()}" },
                                if context.selected.read().len() == 1 {
                                    "{file.file_name()}"
                                } else {
                                    "{context.selected.read().len()} items selected"
                                }
                            }
                            div { class: "text-sm text-gray-600",
                                match *file.file_info().read() {
                                    FileInfo::Image { width, height, .. } => rsx! {
                                        div { "Type: Image" }
                                        div { "Size: {width} × {height}" }
                                    },
                                    FileInfo::Video { width, height, duration, .. } => rsx! {
                                        div { "Type: Video" }
                                        div { "Size: {width} × {height}" }
                                        div { "Duration: {duration}s" }
                                    },
                                    FileInfo::Audio { duration, .. } => rsx! {
                                        div { "Type: Audio" }
                                        div { "Duration: {duration}s" }
                                    },
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "p-4 text-center text-gray-500", "Select a file to preview" }
            }
        }
    }
}

#[component]
fn Thumbnail(
    file: Store<Media>,
    port: String,
    onclick: EventHandler<MouseEvent>,
    ondoubleclick: EventHandler<MouseEvent>,
) -> Element {
    let pack = use_context::<Pack>().0;
    let mut context = use_context::<ImageListContext>();

    rsx! {
        div {
            onclick,
            ondoubleclick,
            onkeydown: move |x| {
                println!("Key down event");
                if x.key() == Key::Enter {
                    x.stop_propagation();
                }
            },
            ContextMenu { class: "focus:outline-hidden focus-visible:outline-hidden",
                ContextMenuTrigger { class: "m-[10px]",
                    div {
                        role: "gridcell",
                        id: "thumbnail-{file.id()}",
                        width: "150px",
                        height: "200px",
                        flex_direction: "column",
                        class: "group image-container flex flex-col items-center",
                        "data-selected": if file.selected()() { "true" },
                        div {
                            width: "100px",
                            height: "100px",
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
                                    max_width: "100px",
                                    max_height: "100px",
                                    margin: "auto",
                                    box_shadow: "0 0 5px",
                                    class: "rounded-xs ",
                                    class: if file.selected()() { "image-selected" } else { "group-hover:brightness-110" },
                                    src: "http://localhost:{port}/thumbnail/{file.id()}?hash={file.hash()}",
                                }
                            }
                        }
                        p {
                            class: "multiline-ellipses mt-2 ",
                            class: if file.selected()() { "bg-sky-400 text-white" } else { "group-hover:bg-sky-200" },
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
                            println!("What!");
                            context.select_all();
                        },
                        "Select all"
                    }
                    ContextMenuItem {
                        index: 1usize,
                        value: "",
                        on_select: move |_| async move {
                            let selected = context.selected.read().clone();

                            let ids: Vec<u64> = selected
                                .iter()
                                .filter_map(|i| context.get(*i))

                                .map(|file| file.id()())
                                .collect();
                            if let Err(err) = pack.read().remove_files(ids).await {
                                eprintln!("{err}");
                                return;
                            }
                            context.remove_files(selected);
                        },
                        "Delete selected"
                    }
                    ContextMenuItem {
                        index: 2usize,
                        value: "",
                        on_select: move |_| async move {
                            let (file_data, _) = pack.read().get_file(file.id()()).await.unwrap();
                            match file_data {
                                FileData::Path(path) => {
                                    println!("{}", path.display());
                                }
                                FileData::Data(data) => {
                                    let mut file = NamedTempFile::with_suffix("webm").unwrap();
                                    file.write_all(&data).unwrap();
                                    println!("{}", file.path().display());
                                }
                            }
                        },
                        "Extract"
                    }
                }
            }
        }
    }
}

#[component]
fn MediaViewDialog() -> Element {
    let port = use_port().map(|p| p.to_string()).unwrap_or_default();
    let mut context = use_context::<ImageListContext>();

    // Local state for editing to avoid "jitter" in global state while typing
    let mut draft_title = use_signal(|| "".to_string());
    let mut is_editing = use_signal(|| false);

    // Sync draft title when the file changes
    use_effect(move || {
        if let Some(idx) = context.opened() {
            if let Some(file) = context.get(idx) {
                draft_title.set(file.file_name().to_string());
            }
        }
    });

    // Navigation logic
    use_global_key_listener(move |key| {
        // Prevent navigation if the user is currently typing in an input
        if is_editing() {
            return;
        }

        if let Some(idx) = context.opened() {
            match key {
                tao::keyboard::Key::ArrowLeft if idx > 0 => {
                    context.opened.set(Some(idx - 1));
                }
                tao::keyboard::Key::ArrowRight if idx < context.filtered_indices.len() - 1 => {
                    context.opened.set(Some(idx + 1));
                }
                _ => {}
            }
        }
    });

    let current_file = context.opened().and_then(|idx| context.get(idx));

    rsx! {
        DialogRoot {
            open: context.opened().is_some(),
            on_open_change: move |v: bool| {
                if !v {
                    context.opened.set(None)
                }
            },

            // Large width for desktop "Workspace" feel
            DialogContent { class: "max-w-[90vw] w-[1200px] h-[80vh] flex flex-col p-0 overflow-hidden bg-white rounded-sm",

                // TOP BAR
                div { class: "flex justify-between items-center px-4 py-2 border-b bg-gray-50",
                    span { class: "text-xs font-mono text-gray-500",
                        if let Some(idx) = context.opened() {
                            "{idx + 1} / {context.filtered_indices.len()}"
                        }
                    }
                    button {
                        class: "p-1 hover:bg-gray-200 rounded transition-colors",
                        onclick: move |_| context.opened.set(None),
                        Icon { icon: Shape::XMark, size: 20 }
                    }
                }

                div { class: "flex flex-1 min-h-0", // Container for Media + Sidebar

                    // LEFT: Media Display (Flexible)
                    div { class: "flex-1 bg-black flex items-center justify-center p-4 relative group",
                        if let Some(file) = current_file.clone() {
                            MediaRenderer { file, port: port.clone() }
                        }
                    }

                    // RIGHT: Metadata Sidebar (Fixed width)
                    div { class: "w-80 border-l bg-white p-6 flex flex-col gap-6 overflow-y-auto",

                        // Title Section
                        section { class: "flex flex-col gap-2",
                            label { class: "text-[10px] font-bold uppercase tracking-wider text-gray-400",
                                "Title"
                            }
                            input {
                                class: "text-lg font-semibold bg-transparent border-b border-transparent hover:border-gray-200 focus:border-blue-500 outline-none transition-all py-1",
                                value: "{draft_title}",
                                onfocusin: move |_| is_editing.set(true),
                                onfocusout: move |_| {
                                    is_editing.set(false);
                                    // context.update_title(draft_title()); // Trigger your save logic here
                                },
                                oninput: move |e| draft_title.set(e.value()),
                            }
                        }

                        // Tags Section (Placeholder for your next step)
                        section { class: "flex flex-col gap-3",
                            label { class: "text-[10px] font-bold uppercase tracking-wider text-gray-400",
                                "Tags"
                            }
                            div { class: "flex flex-wrap gap-2",
                                // Example Tag
                                span { class: "px-2 py-1 bg-blue-50 text-blue-600 text-xs rounded-md border border-blue-100",
                                    "Work"
                                }
                                button { class: "px-2 py-1 border border-dashed border-gray-300 text-gray-400 text-xs rounded-md hover:border-gray-400 hover:text-gray-500",
                                    "+ Add Tag"
                                }
                            }
                        }

                        hr { class: "border-gray-100" }

                        // File Info Details
                        if let Some(file) = current_file {
                            FileDetails { file }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MediaRenderer(file: Store<Media>, port: String) -> Element {
    let url = format!(
        "http://localhost:{}/file/{}?hash={}",
        port,
        file.id(),
        file.hash()
    );

    match *file.file_info().read() {
        FileInfo::Image { width, height, .. } => rsx! {
            img {
                class: "max-w-full max-h-full object-contain shadow-2xl",
                src: "{url}",
            }
        },
        FileInfo::Video { .. } => rsx! {
            video {
                class: "max-w-full max-h-full",
                src: "{url}",
                controls: true,
            }
        },
        FileInfo::Audio { .. } => rsx! {
            audio { class: "w-full", src: "{url}", controls: true }
        },
    }
}

#[component]
fn FileDetails(file: Store<Media>) -> Element {
    let info = file.file_info();
    rsx! {
        div { class: "flex flex-col gap-2 text-xs text-gray-500",
            match *info.read() {
                FileInfo::Image { width, height, .. } => {
                    rsx! {
                        div { class: "flex justify-between",
                            span { "Dimensions" }
                            span { class: "text-gray-900", "{width} × {height}" }
                        }
                    }
                }
                FileInfo::Video { width, height, duration, .. } => {
                    rsx! {
                        div { class: "flex justify-between",
                            span { "Dimensions" }
                            span { class: "text-gray-900", "{width} × {height}" }
                        }
                        div { class: "flex justify-between",
                            span { "Duration" }
                            span { class: "text-gray-900", "{duration}" }
                        }
                    }
                }
                FileInfo::Audio { duration } => {
                    rsx! {
                        div { class: "flex justify-between",
                            span { "Duration" }
                            span { class: "text-gray-900", "{duration}" }
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
                        break;
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
