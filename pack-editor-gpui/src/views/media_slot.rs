//! One media slot: the pack's wallpaper or splash (Content), or one of a stage's wallpaper, track,
//! splash or sounds (Timeline). Both views use this, so a slot looks and behaves the same
//! wherever it is.
//!
//! The row shows the file's thumbnail, name and kind, with "Choose from pack...", "Upload..." and
//! "Remove". Clicking the thumbnail opens the file in the media viewer. Choosing opens a picker
//! dialog with a search field over a grid of the pack's files that can go in the slot.
//!
//! Uploading picks a file through the desktop's file dialog and imports it into the slot on the
//! background runtime (ffmpeg), then brings the session up to date with
//! `Session::slot_filled_by_import`. The slot shows as busy only once a file is chosen: choosing
//! isn't loading (`shared-ui/DESIGN.md`).
//!
//! The host owns one entity per slot and calls [`MediaSlotView::set`] with what the slot holds
//! each time it renders. The title and description above the row are the host's.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, img, px, AnyElement, Context, Entity, EventEmitter, FocusHandle, Image, ImageFormat,
    ObjectFit, SharedString, Subscription, Task, Window,
};
use pack_editor_core::pack::ops::content as content_ops;
use pack_editor_core::pack::MediaFile;
use pack_editor_core::session::library::MediaTab;
use pack_editor_core::session::Session;
use shared::behaviour::MediaSlot;
use shared::encode::FileInfo;

use crate::editor::{Editor, EditorEvent};
use crate::icons::icon;
use crate::theme::{self, c};
use crate::ui::{self, Button, Size, Variant};
use crate::views::library::viewer::{Viewer, ViewerEvent};
use crate::{keep_awake, runtime};

/// More than this and the picker asks for a search instead of laying out every file.
const PICKER_LIMIT: usize = 120;
/// The picker dialog's widest, and its grid's spacing.
const PICKER_W: f32 = 680.;
const PICKER_PAD: f32 = 16.;
const TILE_MIN_W: f32 = 112.;
const TILE_GAP: f32 = 8.;
const TILE_THUMB_H: f32 = 88.;
/// The row's thumbnail.
const THUMB: f32 = 56.;

/// What the slot did, for a host that keeps state of its own about it.
pub(crate) enum SlotEvent {
    /// The slot was filled or emptied.
    Changed,
}

impl EventEmitter<SlotEvent> for MediaSlotView {}

/// What a slot can hold. The picker offers only what the feature would actually use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Accepts {
    Image,
    ImageOrVideo,
    Audio,
}

impl Accepts {
    fn of(slot: &MediaSlot) -> Self {
        match slot {
            MediaSlot::StageAudio { .. }
            | MediaSlot::StageEntrySound { .. }
            | MediaSlot::StagePromptSound { .. } => Accepts::Audio,
            MediaSlot::Splash | MediaSlot::StageEntrySplash { .. } => Accepts::ImageOrVideo,
            MediaSlot::Wallpaper | MediaSlot::StageWallpaper { .. } => Accepts::Image,
        }
    }

    fn takes(self, info: &FileInfo) -> bool {
        matches!(
            (self, info),
            (Accepts::Image, FileInfo::Image { .. })
                | (
                    Accepts::ImageOrVideo,
                    FileInfo::Image { .. } | FileInfo::Video { .. }
                )
                | (Accepts::Audio, FileInfo::Audio { .. })
        )
    }

    fn noun(self) -> &'static str {
        match self {
            Accepts::Image => "images",
            Accepts::ImageOrVideo => "images or videos",
            Accepts::Audio => "audio files",
        }
    }
}

fn kind_label(info: &FileInfo) -> &'static str {
    match info {
        FileInfo::Image { .. } => "image",
        FileInfo::Video { .. } => "video",
        FileInfo::Audio { .. } => "audio",
    }
}

/// A key for element ids, unique per slot.
pub(crate) fn slot_key(slot: &MediaSlot) -> String {
    match slot {
        MediaSlot::Wallpaper => "wallpaper".to_string(),
        MediaSlot::Splash => "splash".to_string(),
        MediaSlot::StageWallpaper { stage } => format!("stage-wallpaper-{stage}"),
        MediaSlot::StageAudio { stage } => format!("stage-audio-{stage}"),
        MediaSlot::StageEntrySplash { stage } => format!("stage-entry-splash-{stage}"),
        MediaSlot::StageEntrySound { stage } => format!("stage-entry-sound-{stage}"),
        MediaSlot::StagePromptSound { stage } => format!("stage-prompt-sound-{stage}"),
    }
}

fn is_stage_slot(slot: &MediaSlot) -> bool {
    !matches!(slot, MediaSlot::Wallpaper | MediaSlot::Splash)
}

/// The picker dialog, while it's open.
struct Picker {
    search: Entity<InputState>,
    _events: Subscription,
}

pub(crate) struct MediaSlotView {
    editor: Entity<Editor>,
    slot: MediaSlot,
    /// What the slot holds, as the host last said.
    current: Option<u64>,
    /// What leaving the slot empty means, as the host last said.
    empty_note: SharedString,
    thumbnails: HashMap<u64, Option<Arc<Image>>>,
    picker: Option<Picker>,
    dialog_focus: FocusHandle,
    uploading: bool,
    /// Worth saying and not otherwise shown: the uploaded file was already in the pack.
    note: Option<SharedString>,
    viewer: Option<Entity<Viewer>>,
    _upload: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MediaSlotView {
    pub fn new(editor: Entity<Editor>, slot: MediaSlot, cx: &mut Context<Self>) -> Self {
        let changes = cx.subscribe(&editor, |this, _, event: &EditorEvent, cx| {
            let EditorEvent::Changed(changes) = event;
            // Thumbnails of files that changed or left are read again.
            for id in changes.media.updated.iter().chain(&changes.media.removed) {
                this.thumbnails.remove(id);
            }
            if !changes.media.removed.is_empty() {
                cx.notify();
            }
        });
        Self {
            editor,
            slot,
            current: None,
            empty_note: SharedString::default(),
            thumbnails: HashMap::new(),
            picker: None,
            dialog_focus: cx.focus_handle(),
            uploading: false,
            note: None,
            viewer: None,
            _upload: None,
            _subscriptions: vec![changes],
        }
    }

    /// What the slot holds and what leaving it empty means, from the host's render.
    pub fn set(&mut self, current: Option<u64>, empty_note: SharedString, cx: &mut Context<Self>) {
        if self.current != current || self.empty_note != empty_note {
            self.current = current;
            self.empty_note = empty_note;
            cx.notify();
        }
    }

    /// Closes the picker, as when the host switches to something else.
    pub fn close_picker(&mut self, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            cx.notify();
        }
    }

    fn thumbnail(&mut self, id: u64, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        if let Some(cached) = self.thumbnails.get(&id) {
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
        self.thumbnails.insert(id, image.clone());
        image
    }

    /// `file`'s thumbnail, fitted into the space it's given, or a stand-in for its kind.
    fn picture(&mut self, file: &MediaFile, icon_size: f32, cx: &mut Context<Self>) -> AnyElement {
        if let FileInfo::Audio { .. } = file.file_info {
            return div()
                .text_color(c(theme::MUTED))
                .child(icon(IconName::Music).size(px(icon_size)))
                .into_any_element();
        }
        match self.thumbnail(file.id, cx) {
            Some(image) => img(image)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => ui::readout(kind_label(&file.file_info)).into_any_element(),
        }
    }

    // ── Edits ──────────────────────────────────────────────────────────────────────────────

    fn choose(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        self.note = None;
        let slot = self.slot.clone();
        self.editor.update(cx, |editor, cx| {
            if is_stage_slot(&slot) {
                editor.apply(cx, |s: &mut Session| s.set_stage_slot(&slot, id));
            } else {
                editor.apply(cx, |s: &mut Session| s.set_media_slot(&slot, id));
            }
        });
        window.blur(cx);
        cx.emit(SlotEvent::Changed);
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.picker = None;
        self.note = None;
        let slot = self.slot.clone();
        self.editor.update(cx, |editor, cx| {
            if is_stage_slot(&slot) {
                editor.apply(cx, |s: &mut Session| s.clear_stage_slot(&slot));
            } else {
                editor.apply(cx, |s: &mut Session| s.clear_media_slot(&slot));
            }
        });
        cx.emit(SlotEvent::Changed);
        cx.notify();
    }

    fn upload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.uploading {
            return;
        }
        self.picker = None;
        self.note = None;
        let slot = self.slot.clone();
        let (filter, extensions) = content_ops::slot_file_filter(&slot);
        let pack = self.editor.read(cx).pack().clone();
        self._upload = Some(cx.spawn_in(window, async move |this, cx| {
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
                this.uploading = true;
                cx.notify();
            });
            let result = runtime::run(async move {
                let _awake = keep_awake::acquire("Importing media into a pack").await;
                content_ops::import_into_slot(pack, &path, slot).await
            })
            .await
            .and_then(|result| result);
            let _ = this.update(cx, |this, cx| {
                this.uploading = false;
                match result {
                    Ok(import) => {
                        let files: Vec<u64> = std::iter::once(import.file.id)
                            .chain(import.deleted)
                            .collect();
                        this.editor.update(cx, |editor, cx| {
                            editor.apply(cx, |s| s.slot_filled_by_import(&files))
                        });
                        if !import.added {
                            this.note = Some(
                                format!("Already in this pack as \"{}\"", import.file.file_name)
                                    .into(),
                            );
                        }
                        cx.emit(SlotEvent::Changed);
                    }
                    Err(error) => this.upload_failed(error, cx),
                }
                cx.notify();
            });
        }));
    }

    fn upload_failed(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        tracing::warn!("couldn't import into a slot: {error:#}");
        self.uploading = false;
        self.editor.update(cx, |editor, cx| {
            editor.error = Some(format!("Couldn't import the file: {error}").into());
            cx.notify();
        });
        cx.notify();
    }

    // ── The picker and the viewer ───────────────────────────────────────────────────────────

    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search files..."));
        let _events = cx.subscribe_in(&search, window, |_, input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                crate::views::library::tags::clean_search_field(
                    input,
                    pack_editor_core::session::library::search_query,
                    window,
                    cx,
                );
                cx.notify();
            }
        });
        search.update(cx, |search, cx| search.focus(window, cx));
        self.picker = Some(Picker { search, _events });
        cx.notify();
    }

    fn view(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.editor.clone();
        let viewer =
            cx.new(|cx| Viewer::new(editor, MediaTab::AllMedia, vec![id], id, false, window, cx));
        let subscription = cx.subscribe_in(
            &viewer,
            window,
            |this, _, event: &ViewerEvent, window, cx| {
                if let ViewerEvent::Close = event {
                    // Dropping the entity stops its preview and frees its image (`media-gpui`).
                    this.viewer = None;
                    window.blur(cx);
                    cx.notify();
                }
            },
        );
        self._subscriptions.push(subscription);
        viewer.update(cx, |viewer, cx| viewer.focus(window, cx));
        self.viewer = Some(viewer);
        cx.notify();
    }

    fn picker_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let search = self.picker.as_ref()?.search.clone();
        let accepts = Accepts::of(&self.slot);
        let key = slot_key(&self.slot);
        let query = search.read(cx).value().trim().to_lowercase();
        let candidates: Vec<MediaFile> = self
            .editor
            .read(cx)
            .session()
            .media()
            .iter()
            .filter(|file| accepts.takes(&file.file_info))
            .filter(|file| query.is_empty() || file.file_name.to_lowercase().contains(&query))
            .cloned()
            .collect();
        let total = candidates.len();

        // The dialog's width is known, so the grid fills it exactly: as many columns as fit.
        let inner = f32::from(ui::dialog_width(px(PICKER_W), window)) - 2. * PICKER_PAD - 2.;
        let columns = (((inner + TILE_GAP) / (TILE_MIN_W + TILE_GAP)).floor() as u16).max(1);

        let current = self.current;
        let tiles: Vec<AnyElement> = candidates
            .iter()
            .take(PICKER_LIMIT)
            .map(|file| {
                let id = file.id;
                let chosen = Some(id) == current;
                let picture = self.picture(file, 28., cx);
                // As the library grid draws a tile: no frame until hovered or chosen; chosen is
                // the carmine edge on a raised surface.
                div()
                    .id(SharedString::from(format!("{key}-pick-{id}")))
                    .min_w_0()
                    .p(px(4.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .rounded(theme::RADIUS_MD)
                    .border_2()
                    .map(|this| {
                        if chosen {
                            this.border_color(c(theme::ACCENT_HOVER))
                                .bg(c(theme::SURFACE_RAISED))
                        } else {
                            this.border_color(gpui_kit::transparent_black())
                                .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
                        }
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .h(px(TILE_THUMB_H))
                            .p(px(4.))
                            .rounded(theme::RADIUS_SM)
                            .bg(c(theme::BG))
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .child(picture),
                    )
                    .child(
                        div()
                            .px_1()
                            .truncate()
                            .text_center()
                            .text_size(px(11.))
                            .child(file.file_name.clone()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| this.choose(id, window, cx)))
                    .into_any_element()
            })
            .collect();

        let footer = if total > PICKER_LIMIT {
            Some(ui::readout(format!(
                "Showing {PICKER_LIMIT} of {total}. Search to narrow it down."
            )))
        } else {
            None
        };
        let entity = cx.entity();
        let grid_h = (f32::from(window.viewport_size().height) - 220.).clamp(160., 440.);

        let body = div()
            .p(px(PICKER_PAD))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                // Opened with focus in it, so it's lit.
                div()
                    .h(theme::CONTROL_COMPACT)
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(theme::FOCUS))
                    .bg(c(theme::BG))
                    .capture_key_down({
                        let entity = entity.clone();
                        move |event, window, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                entity.update(cx, |this, cx| {
                                    this.close_picker(cx);
                                    window.blur(cx);
                                });
                            }
                        }
                    })
                    .child(
                        Input::new(&search)
                            .appearance(false)
                            .text_size(px(13.))
                            .flex_1(),
                    ),
            )
            .child(if tiles.is_empty() {
                div()
                    .h(px(120.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ui::hint(if query.is_empty() {
                        format!("This pack has no {} yet.", accepts.noun())
                    } else {
                        format!("No {} match.", accepts.noun())
                    }))
                    .into_any_element()
            } else {
                div()
                    .id(SharedString::from(format!("{key}-picker-grid")))
                    .max_h(px(grid_h))
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .grid()
                            .grid_cols(columns)
                            .gap(px(TILE_GAP))
                            .children(tiles),
                    )
                    .into_any_element()
            })
            .children(footer);

        Some(
            ui::dialog_window(
                match self.slot {
                    MediaSlot::Wallpaper | MediaSlot::StageWallpaper { .. } => "Choose a wallpaper",
                    MediaSlot::Splash | MediaSlot::StageEntrySplash { .. } => "Choose a splash",
                    MediaSlot::StageAudio { .. } => "Choose a track",
                    MediaSlot::StageEntrySound { .. } | MediaSlot::StagePromptSound { .. } => {
                        "Choose a sound"
                    }
                },
                px(PICKER_W),
                body,
                move |window, cx| {
                    entity.update(cx, |this, cx| this.close_picker(cx));
                    window.blur(cx);
                },
                &self.dialog_focus,
                window,
            )
            .into_any_element(),
        )
    }
}

impl Render for MediaSlotView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key = slot_key(&self.slot);
        let current = self.current;
        let file = current.and_then(|id| self.editor.read(cx).session().media().get(id).cloned());
        let busy = self.uploading;

        let identity = match &file {
            Some(file) => {
                let id = file.id;
                let picture = self.picture(file, 20., cx);
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id(SharedString::from(format!("{key}-view")))
                            .flex_none()
                            .size(px(THUMB))
                            .p(px(2.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .rounded(theme::RADIUS_SM)
                            .border_1()
                            .border_color(c(theme::BORDER))
                            .bg(c(theme::BG))
                            .cursor_pointer()
                            .hover(|style| style.border_color(c(theme::BORDER_STRONG)))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("View").build(window, cx)
                            })
                            .child(picture)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.view(id, window, cx)
                            })),
                    )
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
                            .child(ui::readout(kind_label(&file.file_info))),
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
                        .size(px(THUMB))
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
                    SharedString::from("That file isn't in this pack any more.")
                } else {
                    self.empty_note.clone()
                })),
        };

        let actions = div()
            .flex_none()
            .flex()
            .gap_2()
            .child(
                Button::new(
                    SharedString::from(format!("{key}-choose")),
                    if current.is_some() {
                        "Replace..."
                    } else {
                        "Choose..."
                    },
                )
                .size(Size::Compact)
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.open_picker(window, cx))),
            )
            .child(
                Button::new(SharedString::from(format!("{key}-upload")), "Upload...")
                    .size(Size::Compact)
                    .loading(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.upload(window, cx))),
            )
            .when(current.is_some(), |this| {
                this.child(
                    Button::new(SharedString::from(format!("{key}-remove")), "Remove")
                        .variant(Variant::Destructive)
                        .size(Size::Compact)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                )
            });

        let picker = self.picker_dialog(window, cx);

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .p_2()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .bg(c(theme::BG))
                    .child(identity)
                    .child(actions),
            )
            .children(self.note.clone().map(ui::readout))
            .children(picker)
            .children(self.viewer.clone())
    }
}
