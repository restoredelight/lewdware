use dioxus::{core::Task, prelude::*};
use dioxus_heroicons::{solid::Shape, Icon};
use dioxus_primitives::{ContentAlign, ContentSide, checkbox::CheckboxState};
use shared::components::{
    button::{Button, ButtonVariant}, checkbox::Checkbox, dropdown_menu::{DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger}, label::Label, popover::{PopoverContent, PopoverRoot, PopoverTrigger}, progress::{Progress, ProgressIndicator}, separator::Separator
};

use crate::{
    editor::{Pack, spawn_in_editor},
    encode::{ProcessFilesError, ProcessFilesErrorType, explore_folder, process_files},
    image_list::Media,
};

#[component]
pub fn AddFilesButton(files: Store<Vec<Media>>) -> Element {
    let pack = use_context::<Pack>().0;
    let mut context = use_context::<UploadFilesContext>();

    let mut skip_duplicates = use_signal(|| true);
    let mut recursive = use_signal(|| true);

    rsx! {
        DropdownMenu {
            DropdownMenuTrigger {
                div { class: "flex items-center gap-2",
                    Icon { icon: Shape::Plus, size: 20, class: "my-auto" }
                    span { class: "@max-xl:hidden", "Add files" }
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
                                let task = spawn_in_editor(
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
                                let task = spawn_in_editor(
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

#[component]
pub fn ProgressBar(files: Store<Vec<Media>>) -> Element {
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
    pub fn new() -> Self {
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
