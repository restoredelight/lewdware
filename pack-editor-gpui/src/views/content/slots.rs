//! The pack's wallpaper and splash slots (`MediaSlot.svelte`), and the picker that fills them from
//! the pack's own media or from a file (`ExplicitMediaPicker.svelte`).
//!
//! Choosing from the pack and clearing are ordinary session edits. Uploading picks a file through
//! the desktop's file dialog and imports it into the slot on the background runtime (ffmpeg),
//! then brings the session up to date with `Session::slot_filled_by_import`.
//!
//! Thumbnails are the 100px WebPs stored with each file at import, read on demand and kept until
//! the file changes or the view goes. The previewing a file at full size that the Tauri picker
//! offered isn't here yet.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, img, px, AnyElement, Context, Entity, Image, ImageFormat, ObjectFit, SharedString,
    Subscription, Task, Window,
};
use pack_editor_core::changes::MediaDelta;
use pack_editor_core::pack::ops::content as content_ops;
use pack_editor_core::pack::MediaFile;
use shared::behaviour::MediaSlot;
use shared::encode::FileInfo;

use super::ContentView;
use crate::theme::{self, c};
use crate::ui::{self, Button, Size, Variant};
use crate::{keep_awake, runtime};

/// The pack-wide slots this view edits. Stage slots belong to the timeline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Slot {
    Wallpaper,
    Splash,
}

impl Slot {
    fn media_slot(self) -> MediaSlot {
        match self {
            Slot::Wallpaper => MediaSlot::Wallpaper,
            Slot::Splash => MediaSlot::Splash,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Slot::Wallpaper => "Wallpaper",
            Slot::Splash => "Splash",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Slot::Wallpaper => "The image Lewdware sets as the desktop wallpaper.",
            Slot::Splash => {
                "Shown once when a session starts. May be a video; an animated GIF is one."
            }
        }
    }

    fn empty_note(self) -> &'static str {
        match self {
            Slot::Wallpaper => "Lewdware will not change the wallpaper.",
            Slot::Splash => "No startup image will be shown.",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Slot::Wallpaper => "wallpaper",
            Slot::Splash => "splash",
        }
    }

    /// Whether `file` can go in this slot: a wallpaper is an image, a splash an image or video.
    fn accepts(self, file: &MediaFile) -> bool {
        match file.file_info {
            FileInfo::Image { .. } => true,
            FileInfo::Video { .. } => self == Slot::Splash,
            FileInfo::Audio { .. } => false,
        }
    }

    fn current(self, view: &ContentView) -> Option<u64> {
        let slots = view.data.slots?;
        match self {
            Slot::Wallpaper => slots.wallpaper,
            Slot::Splash => slots.splash,
        }
    }
}

/// The browser under a slot: a search field and the files that can go in it.
struct Picker {
    slot: Slot,
    search: Entity<InputState>,
    _events: Subscription,
}

/// More than this and the browser asks for a search instead of laying out every file.
const PICKER_LIMIT: usize = 120;

#[derive(Default)]
pub(super) struct SlotsState {
    picker: Option<Picker>,
    thumbnails: HashMap<u64, Option<Arc<Image>>>,
    /// The slot a file is being imported into.
    uploading: Option<Slot>,
    /// Something worth saying about a slot that the slot itself doesn't show: the uploaded file
    /// was already in the pack.
    note: Option<(Slot, SharedString)>,
    _upload: Option<Task<()>>,
}

impl SlotsState {
    pub(super) fn close_picker(&mut self) {
        self.picker = None;
    }

    /// Drops the thumbnails of files that changed or left, so they're read again.
    pub(super) fn forget_thumbnails(&mut self, media: &MediaDelta) {
        for id in media.updated.iter().chain(&media.removed) {
            self.thumbnails.remove(id);
        }
    }
}

fn kind_label(file: &MediaFile) -> &'static str {
    match file.file_info {
        FileInfo::Image { .. } => "image",
        FileInfo::Video { .. } => "video",
        FileInfo::Audio { .. } => "audio",
    }
}

impl ContentView {
    fn thumbnail(&mut self, id: u64, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        if let Some(cached) = self.slots.thumbnails.get(&id) {
            return cached.clone();
        }
        let bytes = self
            .editor
            .read(cx)
            .session()
            .read(|c| content_ops::thumbnail(c, id))
            .unwrap_or_else(|error| {
                tracing::warn!("couldn't read the thumbnail of {id}: {error:#}");
                None
            });
        let image = bytes.map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Webp, bytes)));
        self.slots.thumbnails.insert(id, image.clone());
        image
    }

    /// A square showing `file`'s thumbnail, or what kind of file it is when it has none.
    fn thumbnail_box(&mut self, file: &MediaFile, size: f32, cx: &mut Context<Self>) -> AnyElement {
        let image = self.thumbnail(file.id, cx);
        div()
            .flex_none()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::BG))
            .map(|this| match image {
                Some(image) => this.child(img(image).size_full().object_fit(ObjectFit::Contain)),
                None => this.child(ui::readout(kind_label(file))),
            })
            .into_any_element()
    }

    fn toggle_picker(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .slots
            .picker
            .as_ref()
            .is_some_and(|picker| picker.slot == slot)
        {
            self.slots.picker = None;
            cx.notify();
            return;
        }
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search media..."));
        let _events = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        search.update(cx, |search, cx| search.focus(window, cx));
        self.slots.picker = Some(Picker {
            slot,
            search,
            _events,
        });
        cx.notify();
    }

    fn choose(&mut self, slot: Slot, id: u64, cx: &mut Context<Self>) {
        self.slots.picker = None;
        self.slots.note = None;
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |s| s.set_media_slot(&slot.media_slot(), id))
        });
        cx.notify();
    }

    fn clear(&mut self, slot: Slot, cx: &mut Context<Self>) {
        self.slots.note = None;
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |s| s.clear_media_slot(&slot.media_slot()))
        });
        cx.notify();
    }

    /// Picks a file and imports it into `slot`. The slot shows as busy only once a file is
    /// chosen: choosing isn't loading (`shared-ui/DESIGN.md`).
    fn upload(&mut self, slot: Slot, window: &mut Window, cx: &mut Context<Self>) {
        if self.slots.uploading.is_some() {
            return;
        }
        self.slots.picker = None;
        self.slots.note = None;
        let media_slot = slot.media_slot();
        let (filter, extensions) = content_ops::slot_file_filter(&media_slot);
        let pack = self.editor.read(cx).pack().clone();
        self.slots._upload = Some(cx.spawn_in(window, async move |this, cx| {
            let picked = runtime::run(async move {
                rfd::AsyncFileDialog::new()
                    .set_title("Select a file")
                    .add_filter(filter, extensions)
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            })
            .await;
            let path = match picked {
                Ok(Some(path)) => path,
                Ok(None) => return,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| this.upload_failed(error, cx));
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| {
                this.slots.uploading = Some(slot);
                cx.notify();
            });
            let result = runtime::run(async move {
                let _awake = keep_awake::acquire("Importing media into a pack").await;
                content_ops::import_into_slot(pack, &path, media_slot).await
            })
            .await
            .and_then(|result| result);
            let _ = this.update(cx, |this, cx| {
                this.slots.uploading = None;
                match result {
                    Ok(import) => {
                        let files: Vec<u64> = std::iter::once(import.file.id)
                            .chain(import.deleted)
                            .collect();
                        this.editor.update(cx, |editor, cx| {
                            editor.apply(cx, |s| s.slot_filled_by_import(&files))
                        });
                        if !import.added {
                            this.slots.note = Some((
                                slot,
                                format!("Already in this pack as \"{}\"", import.file.file_name)
                                    .into(),
                            ));
                        }
                    }
                    Err(error) => this.upload_failed(error, cx),
                }
                cx.notify();
            });
        }));
    }

    fn upload_failed(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        tracing::warn!("couldn't import into a slot: {error:#}");
        self.slots.uploading = None;
        self.editor.update(cx, |editor, cx| {
            editor.error = Some(format!("Couldn't import the file: {error}").into());
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn slots_section(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let wallpaper = self.slot(Slot::Wallpaper, window, cx);
        let splash = self.slot(Slot::Splash, window, cx);
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(wallpaper)
            .child(splash)
            .into_any_element()
    }

    fn slot(&mut self, slot: Slot, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let current = slot.current(self);
        let file = current.and_then(|id| self.editor.read(cx).session().media().get(id).cloned());
        let busy = self.slots.uploading == Some(slot);
        let inert = self.slots.uploading.is_some();
        let open = self.slots.picker.as_ref().is_some_and(|p| p.slot == slot);

        let identity = match &file {
            Some(file) => {
                let thumbnail = self.thumbnail_box(file, 64., cx);
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(thumbnail)
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .text_color(c(theme::TEXT))
                                    .child(file.file_name.clone()),
                            )
                            .child(ui::readout(kind_label(file))),
                    )
            }
            None => div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_none()
                        .size(px(64.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS_SM)
                        .border_1()
                        .border_color(c(theme::BORDER))
                        .bg(c(theme::BG))
                        .text_size(px(11.))
                        .text_color(c(theme::MUTED))
                        .child("Empty"),
                )
                .child(ui::hint(if current.is_some() {
                    "That file isn't in this pack any more."
                } else {
                    slot.empty_note()
                })),
        };

        let entity = cx.entity();
        let actions = div()
            .flex_none()
            .flex()
            .gap_2()
            .child(
                Button::new(
                    SharedString::from(format!("{}-choose", slot.id())),
                    if current.is_some() {
                        "Replace from pack..."
                    } else {
                        "Choose from pack..."
                    },
                )
                .size(Size::Compact)
                .disabled(inert)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.toggle_picker(slot, window, cx)),
                ),
            )
            .child(
                Button::new(
                    SharedString::from(format!("{}-upload", slot.id())),
                    "Upload...",
                )
                .size(Size::Compact)
                .disabled(inert && !busy)
                .loading(busy)
                .on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.upload(slot, window, cx))
                }),
            )
            .when(current.is_some(), |this| {
                this.child(
                    Button::new(
                        SharedString::from(format!("{}-remove", slot.id())),
                        "Remove",
                    )
                    .variant(Variant::Destructive)
                    .size(Size::Compact)
                    .disabled(inert)
                    .on_click(cx.listener(move |this, _, _, cx| this.clear(slot, cx))),
                )
            });

        let note = self
            .slots
            .note
            .as_ref()
            .filter(|(noted, _)| *noted == slot)
            .map(|(_, note)| ui::readout(note.clone()));
        let browser = open.then(|| self.browser(slot, current, cx));

        div()
            .id(SharedString::from(format!("slot-{}", slot.id())))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(ui::section_title(slot.title()))
                    .child(ui::hint(slot.description())),
            )
            .child(
                div()
                    .p_2()
                    .flex()
                    .items_center()
                    .gap_3()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .bg(c(theme::SURFACE))
                    .child(identity)
                    .child(actions),
            )
            .children(note)
            .children(browser)
            .into_any_element()
    }

    fn browser(&mut self, slot: Slot, current: Option<u64>, cx: &mut Context<Self>) -> AnyElement {
        let Some(picker) = self.slots.picker.as_ref() else {
            return div().into_any_element();
        };
        let search = picker.search.clone();
        let query = search.read(cx).value().trim().to_lowercase();
        let candidates: Vec<MediaFile> = self
            .editor
            .read(cx)
            .session()
            .media()
            .iter()
            .filter(|file| slot.accepts(file))
            .filter(|file| query.is_empty() || file.file_name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        let total = candidates.len();
        let tiles: Vec<AnyElement> = candidates
            .iter()
            .take(PICKER_LIMIT)
            .map(|file| {
                let id = file.id;
                let selected = Some(id) == current;
                let thumbnail = self.thumbnail_box(file, 72., cx);
                div()
                    .id(SharedString::from(format!("pick-{}-{id}", slot.id())))
                    .w(px(120.))
                    .p(px(7.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .bg(c(theme::SURFACE))
                    .when(selected, |this| {
                        this.border_l_2().border_color(c(theme::ACCENT_HOVER))
                    })
                    .cursor_pointer()
                    .hover(|style| style.border_color(c(theme::BORDER_STRONG)))
                    .child(thumbnail)
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(c(theme::TEXT))
                            .child(file.file_name.clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(slot, id, cx)))
                    .into_any_element()
            })
            .collect();

        let footer = if total == 0 {
            Some(ui::hint("No matching media in this pack."))
        } else if total > PICKER_LIMIT {
            Some(ui::readout(format!(
                "Showing {PICKER_LIMIT} of {total}. Search to narrow it down."
            )))
        } else {
            None
        };
        let entity = cx.entity();

        div()
            .flex()
            .flex_col()
            .rounded(theme::RADIUS_SM)
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .bg(c(theme::BG))
            .child(
                div()
                    .p_2()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(
                        div()
                            .h(theme::CONTROL_COMPACT)
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded(theme::RADIUS_SM)
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .bg(c(theme::BG))
                            .capture_key_down(move |event, _, cx| {
                                if event.keystroke.key == "escape" {
                                    cx.stop_propagation();
                                    entity.update(cx, |this, cx| {
                                        this.slots.picker = None;
                                        cx.notify();
                                    });
                                }
                            })
                            .child(
                                Input::new(&search)
                                    .appearance(false)
                                    .text_size(px(12.))
                                    .flex_1(),
                            ),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("picker-{}", slot.id())))
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_wrap()
                    .gap(px(7.))
                    .children(tiles),
            )
            .children(footer.map(|footer| div().px_2().pb_2().child(footer)))
            .into_any_element()
    }
}
