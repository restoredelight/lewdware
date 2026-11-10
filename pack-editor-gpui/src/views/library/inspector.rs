//! The inspector beside a media tab: what's selected, edited as a whole. Ported from
//! `pack-editor/src/lib/Sidebar.svelte`.
//!
//! One file: a preview, its name and source (committed on Enter or leaving the field), tags and
//! artists, details, and in the tab that owns them its role controls. Several: the tags and
//! artists they share, editable as a set, and the ones only some carry, offered as "add to all"
//! or "remove". Removing asks the browser, which owns the confirmation.
//!
//! The preview keeps the lifetime rules from `media-gpui/README.md`: it opens paused, and
//! changing the primary file, an empty selection or dropping the inspector releases playback and
//! the uploaded image.

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState, SliderValue};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, ClipboardItem, Context, Entity, EventEmitter, Focusable, FontWeight,
    SharedString, Subscription, Task, Window,
};
use media_gpui::MediaView;
use pack_editor_core::changes::Scope;
use pack_editor_core::pack::ops::library::MediaUsageTarget;
use pack_editor_core::pack::{MediaFile, MediaUsage};
use pack_editor_core::session::library::{
    is_popup_audio, label_edit_count, label_names, LabelKind, LabelSummary, MediaTab,
};
use shared::encode::FileInfo;

use super::{format_duration, format_size, plural};
use crate::editor::{CommitPending, Editor, EditorEvent};
use crate::shell::View;
use crate::theme::{self, c};
use crate::ui::{self, Button, RadioOption, Size, Variant};
use crate::views::timeline::{stage_membership, StageMembership};

pub(crate) const WIDTH: f32 = 320.;
/// How far the inspector's edge drags.
pub(crate) const WIDTH_RANGE: (f32, f32) = (260., 560.);

pub(crate) enum InspectorEvent {
    /// Asks to remove these files (the browser confirms first).
    Remove(Vec<u64>),
    /// Asks to open the full-size viewer on this file.
    View(u64),
}

impl EventEmitter<InspectorEvent> for Inspector {}

pub(crate) struct Inspector {
    tab: MediaTab,
    editor: Entity<Editor>,
    /// Selected, in the list's order.
    ids: Vec<u64>,
    primary: Option<u64>,

    preview: Entity<MediaView>,
    previewed: Option<u64>,
    generation: u64,
    _loading: Option<Task<()>>,

    name: Entity<InputState>,
    name_error: Option<SharedString>,
    source: Entity<InputState>,
    tag_input: Entity<InputState>,
    artist_input: Entity<InputState>,

    usage: Vec<MediaUsage>,
    volume: Option<f64>,
    volume_slider: Entity<SliderState>,
    /// The slider's position while dragging, before the release commits it.
    dragged_volume: Option<f32>,
    /// The timeline's "Appears in" strip, made on first use and repointed as the selection moves.
    membership: Option<Entity<StageMembership>>,
    _subscriptions: Vec<Subscription>,
}

impl Inspector {
    pub fn new(
        tab: MediaTab,
        editor: Entity<Editor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let preview = cx.new(|cx| MediaView::new(window, cx));
        let name = cx.new(|cx| InputState::new(window, cx));
        let source = cx.new(|cx| InputState::new(window, cx).placeholder("https://..."));
        let tag_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add tag..."));
        let artist_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add artist..."));
        let volume_slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(1.)
                .step(0.05)
                .default_value(1.)
        });

        let subscriptions = vec![
            // Focused or not: a click elsewhere can take focus without the input reporting a
            // blur. Both are no-ops when the text matches the file.
            cx.subscribe_in(&editor, window, |this, _, _: &CommitPending, window, cx| {
                this.commit_name(window, cx);
                this.commit_source(window, cx);
            }),
            cx.subscribe_in(
                &editor,
                window,
                |this, _, event: &EditorEvent, window, cx| {
                    let EditorEvent::Changed(changes) = event;
                    let primary_changed = this
                        .primary
                        .is_some_and(|id| changes.media.updated.contains(&id));
                    if primary_changed {
                        this.sync_fields(window, cx);
                    }
                    if changes
                        .scopes
                        .intersects(Scope::Slots | Scope::Timeline | Scope::Attributes)
                    {
                        this.refresh_reads(window, cx);
                    }
                    if changes
                        .scopes
                        .intersects(Scope::Media | Scope::Tags | Scope::Artists)
                    {
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(&name, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.commit_name(window, cx);
                }
            }),
            cx.subscribe_in(
                &source,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        this.commit_source(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &tag_input,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let value = input.read(cx).value().trim().to_string();
                        if !value.is_empty() {
                            this.edit_label(LabelKind::Tags, &value, true, cx);
                        }
                        input.update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &artist_input,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let value = input.read(cx).value().trim().to_string();
                        if !value.is_empty() {
                            this.edit_label(LabelKind::Artists, &value, true, cx);
                        }
                        input.update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    InputEvent::Change => cx.notify(),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &volume_slider,
                window,
                |this, _, event: &SliderEvent, _, cx| {
                    let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
                    let SliderValue::Single(value) = *value else {
                        return;
                    };
                    if matches!(event, SliderEvent::Release(_)) {
                        this.dragged_volume = None;
                        this.commit_volume(value as f64, cx);
                    } else {
                        this.dragged_volume = Some(value);
                    }
                    cx.notify();
                },
            ),
        ];

        let mut this = Self {
            tab,
            editor,
            ids: Vec::new(),
            primary: None,
            preview,
            previewed: None,
            generation: 0,
            _loading: None,
            name,
            name_error: None,
            source,
            tag_input,
            artist_input,
            usage: Vec::new(),
            volume: None,
            volume_slider,
            dragged_volume: None,
            membership: None,
            _subscriptions: subscriptions,
        };
        this.track_fields(window, cx);
        this
    }

    /// Redraws on focus changes (field borders, the suggestion lists) and commits the name and
    /// source when they lose focus, on gpui's own focus events, which always arrive.
    fn track_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            self.name.clone(),
            self.source.clone(),
            self.tag_input.clone(),
            self.artist_input.clone(),
        ] {
            self._subscriptions
                .extend(ui::track_focus_of(&input, window, cx));
        }
        let name = self.name.read(cx).focus_handle(cx);
        let source = self.source.read(cx).focus_handle(cx);
        self._subscriptions
            .push(cx.on_blur(&name, window, |this, window, cx| {
                this.commit_name(window, cx)
            }));
        self._subscriptions
            .push(cx.on_blur(&source, window, |this, window, cx| {
                this.commit_source(window, cx)
            }));
    }

    /// Points the inspector at a new selection. Cheap when nothing changed.
    pub fn set_selection(
        &mut self,
        ids: Vec<u64>,
        primary: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let primary = primary.filter(|id| ids.contains(id));
        if ids == self.ids && primary == self.primary {
            return;
        }
        let primary_moved = primary != self.primary;
        self.ids = ids;
        self.primary = primary;
        if primary_moved {
            self.name_error = None;
            self.sync_fields(window, cx);
            self.refresh_reads(window, cx);
            self.load_preview(window, cx);
        }
        cx.notify();
    }

    fn files<'a>(&self, editor: &'a Editor) -> Vec<&'a MediaFile> {
        self.ids
            .iter()
            .filter_map(|id| editor.session().media().get(*id))
            .collect()
    }

    fn primary_file(&self, cx: &gpui_kit::App) -> Option<MediaFile> {
        let id = self.primary?;
        self.editor.read(cx).session().media().get(id).cloned()
    }

    // ── The preview ─────────────────────────────────────────────────────────────────────────

    fn close_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._loading = None;
        self.generation += 1;
        self.previewed = None;
        self.preview
            .update(cx, |preview, cx| preview.clear(window, cx));
    }

    fn load_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.primary_file(cx) else {
            self.close_preview(window, cx);
            return;
        };
        if self.previewed == Some(file.id) {
            return;
        }
        self.close_preview(window, cx);
        self.previewed = Some(file.id);
        let generation = self.generation;
        let id = file.id;
        self.preview
            .update(cx, |preview, cx| preview.begin_loading(window, cx));
        let opening = super::open_preview(self.editor.read(cx), &file);
        self._loading = Some(cx.spawn_in(window, async move |this, cx| {
            let result = opening.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.previewed != Some(id) {
                    return;
                }
                // `load` focuses the preview for its own keys. The list keeps the keyboard
                // (arrows, Shift ranges, Ctrl+A); clicking the preview gives it focus.
                let focused = window.focused(cx);
                this.preview.update(cx, |preview, cx| match result {
                    Ok(source) => preview.load(source, window, cx),
                    Err(error) => preview.fail(format!("{error:#}"), cx),
                });
                if let Some(focused) = focused {
                    window.focus(&focused, cx);
                }
                cx.notify();
            });
        }));
    }

    // ── Fields ──────────────────────────────────────────────────────────────────────────────

    /// Puts the primary file's name and source into their fields, unless one is being typed in.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let file = self.primary_file(cx);
        for (input, value) in [
            (&self.name, file.as_ref().map(|f| f.file_name.clone())),
            (
                &self.source,
                file.as_ref().and_then(|f| f.source_url.clone()),
            ),
        ] {
            if input.read(cx).focus_handle(cx).is_focused(window) {
                continue;
            }
            let value = value.unwrap_or_default();
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    /// Re-reads what the inspector shows from outside the media index: the slots the file fills
    /// and its volume.
    fn refresh_reads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.editor.read(cx).session();
        let (usage, volume) = match self.primary {
            Some(id) => (
                session.media_usage(id).unwrap_or_default(),
                session.audio_volume(id).ok().flatten(),
            ),
            None => (Vec::new(), None),
        };
        self.usage = usage;
        if volume != self.volume || self.dragged_volume.is_none() {
            self.volume = volume;
            let value = volume.unwrap_or(1.) as f32;
            self.volume_slider
                .update(cx, |slider, cx| slider.set_value(value, window, cx));
        }
        cx.notify();
    }

    fn commit_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.primary_file(cx) else {
            return;
        };
        let value = self.name.read(cx).value().trim().to_string();
        if value == file.file_name {
            return;
        }
        if value.is_empty() {
            // Only the name: resetting the source here would lose an uncommitted URL.
            self.name.update(cx, |input, cx| {
                input.set_value(file.file_name.clone(), window, cx)
            });
            return;
        }
        let renamed = self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| session.rename_file(file.id, &value))
        });
        if renamed.is_none() {
            self.name_error = self.editor.read(cx).error.clone();
            self.name.update(cx, |input, cx| {
                input.set_value(file.file_name.clone(), window, cx)
            });
        } else {
            self.name_error = None;
        }
        cx.notify();
    }

    fn commit_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.primary_file(cx) else {
            return;
        };
        let value = self.source.read(cx).value().trim().to_string();
        let url = (!value.is_empty()).then_some(value);
        if url == file.source_url {
            return;
        }
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| {
                session.set_source_url(file.id, url.as_deref())
            })
        });
        self.sync_fields(window, cx);
    }

    /// Escape in a field: put back what's stored and leave it, so the blur commits nothing.
    fn revert_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.blur(cx);
        self.name_error = None;
        self.sync_fields(window, cx);
        cx.notify();
    }

    /// Puts a tag or artist on the whole selection, or takes it off. Skipped when it would change
    /// nothing, so it leaves no empty undo entry.
    fn edit_label(&mut self, kind: LabelKind, name: &str, add: bool, cx: &mut Context<Self>) {
        let editor = self.editor.read(cx);
        let files = self.files(editor);
        if label_edit_count(files.iter().copied(), kind, name, add) == 0 {
            return;
        }
        let ids = self.ids.clone();
        let name = name.to_string();
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| match (kind, add) {
                (LabelKind::Tags, true) => session.add_tag_to_files(&ids, &name),
                (LabelKind::Tags, false) => session.remove_tag_from_files(&ids, &name),
                (LabelKind::Artists, true) => session.add_artist_to_files(&ids, &name),
                (LabelKind::Artists, false) => session.remove_artist_from_files(&ids, &name),
            })
        });
        cx.notify();
    }

    fn set_audio_role(&mut self, popup: bool, cx: &mut Context<Self>) {
        let ids = self.ids.clone();
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| session.set_popup_audio(&ids, popup))
        });
    }

    fn commit_volume(&mut self, value: f64, cx: &mut Context<Self>) {
        let Some(file) = self.primary_file(cx) else {
            return;
        };
        let value = (value * 20.).round() / 20.;
        let stored = (value < 1.).then_some(value);
        if stored == self.volume {
            return;
        }
        let label = format!("Set volume for \"{}\"", file.file_name);
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |session| {
                session.set_audio_volume(&[file.id], stored, &label)
            })
        });
    }

    // ── Rendering ───────────────────────────────────────────────────────────────────────────

    fn section(title: impl Into<SharedString>) -> gpui_kit::Div {
        div().flex().flex_col().gap(px(7.)).child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::BOLD)
                .child(title.into()),
        )
    }

    fn field_label(text: &'static str) -> gpui_kit::Div {
        div()
            .text_size(px(11.))
            .text_color(c(theme::MUTED))
            .child(text)
    }

    fn field(
        &self,
        input: &Entity<InputState>,
        trailing: Option<AnyElement>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let this = cx.entity();
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(theme::CONTROL_COMPACT)
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(if focused { theme::FOCUS } else { theme::BORDER }))
                    .bg(c(theme::BG))
                    .on_key_down(move |event, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            this.update(cx, |this, cx| this.revert_field(window, cx));
                        }
                    })
                    .child(
                        Input::new(input)
                            .appearance(false)
                            .text_size(px(12.))
                            .flex_1(),
                    ),
            )
            .children(trailing)
    }

    fn icon_button(
        id: impl Into<gpui_kit::ElementId>,
        icon: gpui_kit::assets::IconName,
        on_click: impl Fn(&mut Window, &mut gpui_kit::App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .size(theme::CONTROL_COMPACT)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_SM)
            .text_size(px(14.))
            .text_color(c(theme::MUTED))
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(c(theme::SURFACE_RAISED))
                    .text_color(c(theme::TEXT))
            })
            .child(icon)
            .on_click(move |_, window, cx| on_click(window, cx))
    }

    /// Tags or artists: the names the whole selection carries as removable chips, a field to add
    /// one to all of it (with suggestions from the pack), and the names only some carry.
    fn labels(
        &self,
        kind: LabelKind,
        files: &[&MediaFile],
        suggestions: &[String],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let summary = LabelSummary::of(files.iter().copied(), kind);
        let (title, input) = match kind {
            LabelKind::Tags => ("Tags", &self.tag_input),
            LabelKind::Artists => ("Artists", &self.artist_input),
        };
        let typed = input.read(cx).value().trim().to_lowercase();
        // While the field has focus: the pack's names, narrowed by what's typed. Gone once it
        // loses focus.
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        let matches: Vec<String> = if !focused {
            Vec::new()
        } else {
            suggestions
                .iter()
                .filter(|name| {
                    name.to_lowercase().contains(&typed) && !summary.common.contains(*name)
                })
                .take(6)
                .cloned()
                .collect()
        };
        let several = files.len() > 1;
        let kind_id = match kind {
            LabelKind::Tags => "tag",
            LabelKind::Artists => "artist",
        };
        let entity = cx.entity();

        let chips = summary.common.iter().enumerate().map(|(index, name)| {
            let entity = entity.clone();
            let name_owned = name.clone();
            div()
                .flex()
                .items_center()
                .h(px(24.))
                .pl_2()
                .rounded(theme::RADIUS_SM)
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .bg(c(theme::SURFACE_RAISED))
                .text_size(px(11.5))
                .child(name.clone())
                .child(
                    div()
                        .id((SharedString::from(format!("{kind_id}-remove")), index))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .text_color(c(theme::MUTED))
                        .cursor_pointer()
                        .hover(|style| style.text_color(c(theme::TEXT)))
                        .child(gpui_kit::assets::IconName::Close)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.edit_label(kind, &name_owned, false, cx)
                            })
                        }),
                )
        });

        let mixed = summary
            .mixed
            .iter()
            .enumerate()
            .map(|(index, (name, count))| {
                let add = entity.clone();
                let remove = entity.clone();
                let (add_name, remove_name) = (name.clone(), name.clone());
                let small_button = |id: SharedString, icon| {
                    div()
                        .id((id, index))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.))
                        .text_color(c(theme::MUTED))
                        .cursor_pointer()
                        .hover(|style| style.text_color(c(theme::TEXT)))
                        .child(icon)
                };
                div()
                    .flex()
                    .items_center()
                    .h(px(24.))
                    .pl_2()
                    .gap_1()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_dashed()
                    .border_color(c(theme::BORDER_STRONG))
                    .text_size(px(11.5))
                    .child(name.clone())
                    .child(ui::readout(format!("{count}/{}", files.len())))
                    .child(
                        small_button(
                            format!("{kind_id}-mixed-add").into(),
                            gpui_kit::assets::IconName::Plus,
                        )
                        .on_click(move |_, _, cx| {
                            add.update(cx, |this, cx| this.edit_label(kind, &add_name, true, cx))
                        }),
                    )
                    .child(
                        small_button(
                            format!("{kind_id}-mixed-remove").into(),
                            gpui_kit::assets::IconName::Close,
                        )
                        .on_click(move |_, _, cx| {
                            remove.update(cx, |this, cx| {
                                this.edit_label(kind, &remove_name, false, cx)
                            })
                        }),
                    )
            });

        let suggestion_list = (!matches.is_empty()).then(|| {
            let entity = entity.clone();
            let input = input.clone();
            ui::popover_surface()
                .min_w(px(200.))
                .max_w(px(300.))
                .flex()
                .flex_col()
                .p_1()
                .children(matches.into_iter().enumerate().map(move |(index, name)| {
                    let entity = entity.clone();
                    let input = input.clone();
                    div()
                        .id((SharedString::from(format!("{kind_id}-suggestion")), index))
                        .px_2()
                        .py(px(5.))
                        .rounded(theme::RADIUS_SM)
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(|style| style.bg(c(theme::SURFACE)))
                        .child(name.clone())
                        // On press, and keeping focus in the field: the list goes when the field
                        // loses focus, which a click would otherwise do before it landed.
                        .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                            entity.update(cx, |this, cx| this.edit_label(kind, &name, true, cx));
                        })
                }))
        });

        Self::section(title)
            .when(!summary.common.is_empty(), |this| {
                this.child(div().flex().flex_wrap().gap(px(5.)).children(chips))
            })
            // The suggestions float under the field, over what follows it.
            .child(
                div()
                    .relative()
                    .child(self.field(input, None, window, cx))
                    .children(
                        suggestion_list
                            .map(|list| ui::floating(theme::CONTROL_COMPACT + px(4.), false, list)),
                    ),
            )
            .when(several, |this| {
                this.child(ui::hint("Adds to every selected item.").text_size(px(11.)))
            })
            .when(!summary.mixed.is_empty(), |this| {
                this.child(
                    div()
                        .pt_1()
                        .text_size(px(11.))
                        .text_color(c(theme::MUTED))
                        .child("On some selected items"),
                )
                .child(div().flex().flex_wrap().gap(px(5.)).children(mixed))
            })
    }

    fn details(file: &MediaFile) -> impl IntoElement {
        let mut rows: Vec<(&'static str, String)> = match &file.file_info {
            FileInfo::Image {
                width,
                height,
                transparent,
            } => vec![
                (
                    "Type",
                    if *transparent {
                        "Image (transparent)"
                    } else {
                        "Image"
                    }
                    .into(),
                ),
                ("Dimensions", format!("{width} x {height}")),
            ],
            FileInfo::Video {
                width,
                height,
                duration,
                audio,
                transparent,
            } => vec![
                (
                    "Type",
                    if *transparent {
                        "Video (transparent)"
                    } else {
                        "Video"
                    }
                    .into(),
                ),
                ("Dimensions", format!("{width} x {height}")),
                ("Duration", format_duration(*duration)),
                ("Audio", if *audio { "Yes" } else { "No" }.into()),
            ],
            FileInfo::Audio { duration } => vec![
                ("Type", "Audio".into()),
                ("Duration", format_duration(*duration)),
            ],
        };
        rows.push(("File size", format_size(file.size)));
        Self::section("Details").children(rows.into_iter().map(|(label, value)| {
            div()
                .flex()
                .justify_between()
                .gap_2()
                .text_size(px(11.5))
                .child(div().text_color(c(theme::MUTED)).child(label))
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(11.))
                        .child(value),
                )
        }))
    }

    /// "Used as": what a file is to the built-in modes, linking to the surface that sets it. All
    /// media only; the role tabs explain their own role.
    fn used_as(&self, file: &MediaFile) -> Option<impl IntoElement> {
        if self.tab != MediaTab::AllMedia {
            return None;
        }
        let mut uses: Vec<(SharedString, Target)> = Vec::new();
        if matches!(file.file_info, FileInfo::Audio { .. }) {
            let label = if is_popup_audio(file) {
                "Popup audio"
            } else {
                "Background audio"
            };
            uses.push((label.into(), Target::Tab(MediaTab::Audio)));
        } else if file.popup_eligible {
            uses.push(("Popup".into(), Target::Tab(MediaTab::Popups)));
        }
        for usage in &self.usage {
            let target = match usage.target {
                MediaUsageTarget::Stage { .. } => Target::View(View::Timeline),
                _ => Target::View(View::Content),
            };
            uses.push((usage.label.clone().into(), target));
        }
        if uses.is_empty() {
            return None;
        }
        let editor = self.editor.clone();
        let id = file.id;
        Some(
            Self::section("Used as").child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(5.))
                    .children(
                        uses.into_iter()
                            .enumerate()
                            .map(move |(index, (label, target))| {
                                let editor = editor.clone();
                                div()
                                    .id(("used-as", index))
                                    .px(px(6.))
                                    .py(px(3.))
                                    .rounded(theme::RADIUS_SM)
                                    .border_1()
                                    .border_color(c(theme::BORDER))
                                    .bg(c(theme::BG))
                                    .text_size(px(11.))
                                    .cursor_pointer()
                                    .hover(|style| style.border_color(c(theme::BORDER_STRONG)))
                                    .child(label)
                                    .on_click(move |_, window, cx| match target {
                                        Target::Tab(tab) => {
                                            super::reveal(&editor, tab, id, window, cx)
                                        }
                                        Target::View(view) => {
                                            crate::shell::show_view(view, window, cx)
                                        }
                                    })
                            }),
                    ),
            ),
        )
    }

    /// Which timeline stages the single selected file is in (the timeline track's component).
    fn stage_membership_slot(
        &mut self,
        file: &MediaFile,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let membership = match &self.membership {
            Some(membership) => {
                membership.update(cx, |m, cx| m.set_media(file.id, cx));
                membership.clone()
            }
            None => {
                let membership = stage_membership(self.editor.clone(), file.id, cx);
                self.membership = Some(membership.clone());
                membership
            }
        };
        Some(membership.into_any_element())
    }

    fn audio_controls(&self, files: &[&MediaFile], cx: &mut Context<Self>) -> Option<AnyElement> {
        let all_audio = !files.is_empty()
            && files
                .iter()
                .all(|file| matches!(file.file_info, FileInfo::Audio { .. }));
        if self.tab != MediaTab::Audio || !all_audio {
            return None;
        }
        let popup = files.iter().filter(|file| is_popup_audio(file)).count();
        let role = if popup == files.len() {
            Some(true)
        } else if popup == 0 {
            Some(false)
        } else {
            None
        };
        let entity = cx.entity();
        let volume = self
            .dragged_volume
            .map(f64::from)
            .or(self.volume)
            .unwrap_or(1.);
        let reading = if volume >= 0.995 {
            "Full".to_string()
        } else {
            format!("{}%", (volume * 100.).round())
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .child(
                    Self::section("Audio type")
                        .when(role.is_none(), |this| this.child(ui::readout("Mixed")))
                        .child(ui::radio_group(
                            "audio-role",
                            &role,
                            vec![
                                RadioOption::new(Some(false), "Background")
                                    .description("Plays continuously during the session."),
                                RadioOption::new(Some(true), "Popup")
                                    .description("Plays when a popup appears."),
                            ],
                            move |value, _, cx| {
                                if let Some(popup) = value {
                                    entity.update(cx, |this, cx| this.set_audio_role(popup, cx));
                                }
                            },
                        )),
                )
                .when(files.len() == 1, |this| {
                    this.child(
                        Self::section("Volume").child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(div().flex_1().child(Slider::new(&self.volume_slider)))
                                .child(ui::readout(reading).w(px(36.))),
                        ),
                    )
                })
                .into_any_element(),
        )
    }
}

#[derive(Clone, Copy)]
enum Target {
    Tab(MediaTab),
    View(View),
}

impl Render for Inspector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.read(cx);
        let files: Vec<MediaFile> = self.files(editor).into_iter().cloned().collect();
        let primary = self
            .primary
            .and_then(|id| files.iter().find(|file| file.id == id))
            .cloned();
        let tag_names = label_names(editor.session().media(), LabelKind::Tags);
        let artist_names = label_names(editor.session().media(), LabelKind::Artists);
        let shell = div()
            .id("inspector")
            .size_full()
            .flex()
            .flex_col()
            .bg(c(theme::SURFACE));

        let Some(primary) = primary else {
            return shell.child(
                div()
                    .flex_1()
                    .p_6()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(ui::label("Nothing selected"))
                    .child(
                        ui::hint(
                            "Select an item to inspect it. Shift-click selects a range, \
                             Ctrl-click adds or removes one.",
                        )
                        .text_center(),
                    ),
            );
        };
        let refs: Vec<&MediaFile> = files.iter().collect();
        let single = files.len() == 1;
        let entity = cx.entity();

        let header = div()
            .flex_none()
            .h(px(36.))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(FontWeight::BOLD)
                    .child(if single {
                        SharedString::from(primary.file_name.clone())
                    } else {
                        plural(files.len(), "item selected", "items selected").into()
                    }),
            )
            .child({
                let entity = entity.clone();
                let id = primary.id;
                Button::new("inspector-view", "View")
                    .icon(gpui_kit::assets::IconName::Eye)
                    .variant(Variant::Quiet)
                    .size(Size::Compact)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |_, cx| cx.emit(InspectorEvent::View(id)))
                    })
            });

        // Audio has no picture: just the player's controls.
        let audio = matches!(primary.file_info, FileInfo::Audio { .. });
        let preview = div()
            .flex_none()
            .h(px(if audio {
                media_gpui::CONTROLS_HEIGHT
            } else {
                250.
            }))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::BG))
            .child(self.preview.clone());

        let mut body = div()
            .id("inspector-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .p_3()
            .flex()
            .flex_col()
            .gap_4();

        if single {
            body = body.children(self.used_as(&primary));
        }
        body = body.children(self.audio_controls(&refs, cx));

        if single {
            let copy_name = primary.file_name.clone();
            let source = primary.source_url.clone();
            body = body.child(
                Self::section("Media")
                    .child(Self::field_label("File name"))
                    .child(
                        self.field(
                            &self.name,
                            Some(
                                Self::icon_button(
                                    "copy-name",
                                    gpui_kit::assets::IconName::Copy,
                                    move |_, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_name.clone(),
                                        ))
                                    },
                                )
                                .into_any_element(),
                            ),
                            window,
                            cx,
                        ),
                    )
                    .children(self.name_error.clone().map(|error| {
                        div()
                            .text_size(px(11.))
                            .text_color(c(theme::DANGER))
                            .child(error)
                    }))
                    .child(Self::field_label("Source URL"))
                    .child(self.field(
                        &self.source,
                        source.map(|url| {
                            Self::icon_button(
                                "open-source",
                                gpui_kit::assets::IconName::ExternalLink,
                                move |_, cx| cx.open_url(&url),
                            )
                            .into_any_element()
                        }),
                        window,
                        cx,
                    )),
            );
        }

        body = body
            .child(self.labels(LabelKind::Tags, &refs, &tag_names, window, cx))
            .child(self.labels(LabelKind::Artists, &refs, &artist_names, window, cx));

        if single {
            body = body
                .children(self.stage_membership_slot(&primary, cx))
                .child(Self::details(&primary));
        } else {
            let total: u64 = files.iter().map(|file| file.size).sum();
            let kinds: std::collections::HashSet<_> = files
                .iter()
                .map(|file| std::mem::discriminant(&file.file_info))
                .collect();
            body = body.child(
                div()
                    .pt_2()
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .flex()
                    .justify_between()
                    .child(ui::readout(format_size(total)))
                    .child(ui::readout(if kinds.len() == 1 {
                        format!(
                            "{} files",
                            match primary.file_info {
                                FileInfo::Image { .. } => "image",
                                FileInfo::Video { .. } => "video",
                                FileInfo::Audio { .. } => "audio",
                            }
                        )
                    } else {
                        "Mixed media types".to_string()
                    })),
            );
        }

        let ids = self.ids.clone();
        body = body.child(
            div()
                .pt_3()
                .border_t_1()
                .border_color(c(theme::BORDER))
                .child(
                    Button::new(
                        "inspector-remove",
                        if single {
                            "Remove item".to_string()
                        } else {
                            format!("Remove {} items", files.len())
                        },
                    )
                    .variant(Variant::Destructive)
                    .size(Size::Compact)
                    .on_click(move |_, _, cx| {
                        let ids = ids.clone();
                        entity.update(cx, |_, cx| cx.emit(InspectorEvent::Remove(ids)))
                    }),
                ),
        );

        shell.child(header).child(preview).child(body)
    }
}
