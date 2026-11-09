//! The Metadata view: the pack's name, creator, description and version. Ported from
//! `pack-editor/src/lib/Options.svelte`.
//!
//! Each field commits on Enter or when it loses focus (the description, being multi-line, only
//! on losing focus); Escape puts back what's stored. A field the author isn't in follows the
//! pack, so undo and redo show up in it.

use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::{div, px, Context, Entity, Focusable, Subscription, Window};
use pack_editor_core::changes::Scope;
use pack_editor_core::pack::ops::lifecycle::MetadataField;
use shared::pack::Metadata;

use crate::editor::{CommitPending, Editor, EditorEvent};
use crate::theme::{self, c};
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Creator,
    Version,
}

impl Field {
    fn stored(self, metadata: &Metadata) -> String {
        match self {
            Field::Name => metadata.name.clone(),
            Field::Creator => metadata.creator.clone().unwrap_or_default(),
            Field::Version => metadata.version.clone().unwrap_or_default(),
        }
    }

    fn edit(self, text: String) -> MetadataField {
        match self {
            Field::Name => MetadataField::Name(text),
            Field::Creator => MetadataField::Creator(Some(text)),
            Field::Version => MetadataField::Version(Some(text)),
        }
    }
}

pub struct MetadataView {
    editor: Entity<Editor>,
    name: Entity<InputState>,
    creator: Entity<InputState>,
    version: Entity<InputState>,
    description: Entity<TextareaState>,
    /// Fields typed in since they were last committed (`None` is the description).
    edited: Vec<Option<Field>>,
    _subscriptions: Vec<Subscription>,
}

impl MetadataView {
    pub fn new(editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let metadata = editor.read(cx).session().metadata();
        let mut subscriptions = Vec::new();

        let mut line = |field: Field, placeholder: &'static str, cx: &mut Context<Self>| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(field.stored(&metadata))
            });
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } | InputEvent::Blur => {
                        this.edited.retain(|edited| *edited != Some(field));
                        let text = input.read(cx).value().to_string();
                        this.commit_line(field, text, input, window, cx);
                    }
                    InputEvent::Change => {
                        if !this.edited.contains(&Some(field)) {
                            this.edited.push(Some(field));
                        }
                        this.commit_unfocused(window, cx);
                    }
                    _ => this.commit_unfocused(window, cx),
                },
            ));
            input
        };
        let name = line(Field::Name, "Pack name", cx);
        let creator = line(Field::Creator, "Creator name", cx);
        let version = line(Field::Version, "e.g. 1.0.0", cx);

        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(4, 12)
                .placeholder("Optional description")
                .default_value(metadata.description.clone().unwrap_or_default())
        });
        // Blur isn't reliably reported (the multi-line state never reports it, and the headless
        // check sees none from single-line fields either), so each field also notes that it's
        // been edited, and is committed once it no longer has focus: checked on render and on
        // the other fields' events.
        subscriptions.push(cx.subscribe_in(
            &description,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) && !this.edited.contains(&None) {
                    this.edited.push(None);
                    cx.notify();
                }
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &editor,
            window,
            |this, _, _: &CommitPending, window, cx| this.commit_edited(window, cx),
        ));
        subscriptions.push(cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &EditorEvent, window, cx| match event {
                EditorEvent::Changed(changes) => {
                    if changes.scopes.contains(Scope::Metadata) {
                        this.follow_pack(window, cx);
                    }
                }
            },
        ));

        Self {
            editor,
            name,
            creator,
            version,
            description,
            edited: Vec::new(),
            _subscriptions: subscriptions,
        }
    }

    fn commit_line(
        &mut self,
        field: Field,
        text: String,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stored = field.stored(&self.editor.read(cx).session().metadata());
        // A blank name isn't allowed: put the stored one back rather than complain.
        let blank_name = field == Field::Name && text.trim().is_empty();
        if !blank_name && text.trim() != stored {
            self.editor.update(cx, |editor, cx| {
                editor.apply(cx, |session| session.set_metadata(field.edit(text)));
            });
        }
        // Show what was stored (trimmed, or unchanged after a refusal).
        let shown = field.stored(&self.editor.read(cx).session().metadata());
        if input.read(cx).value().as_ref() != shown {
            input.update(cx, |input, cx| input.set_value(shown, window, cx));
        }
    }

    fn commit_description(
        &mut self,
        text: String,
        input: &Entity<TextareaState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stored = self.editor.read(cx).session().metadata().description;
        if text.trim() != stored.as_deref().unwrap_or_default() {
            self.editor.update(cx, |editor, cx| {
                editor.apply(cx, |session| {
                    session.set_metadata(MetadataField::Description(Some(text)))
                });
            });
        }
        let shown = self
            .editor
            .read(cx)
            .session()
            .metadata()
            .description
            .unwrap_or_default();
        if input.read(cx).value().as_ref() != shown {
            input.update(cx, |input, cx| input.set_value(shown, window, cx));
        }
    }

    /// Commits each edited field that no longer has focus.
    fn commit_unfocused(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_pending(false, window, cx);
    }

    /// Commits every edited field, focused or not ([`CommitPending`]).
    fn commit_edited(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_pending(true, window, cx);
    }

    fn commit_pending(&mut self, even_focused: bool, window: &mut Window, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.edited);
        for field in pending {
            let focused = match field {
                Some(field) => self.input(field).focus_handle(cx).is_focused(window),
                None => self.description.focus_handle(cx).is_focused(window),
            };
            if focused && !even_focused {
                self.edited.push(field);
                continue;
            }
            match field {
                Some(field) => {
                    let input = self.input(field).clone();
                    let text = input.read(cx).value().to_string();
                    self.commit_line(field, text, &input, window, cx);
                }
                None => {
                    let input = self.description.clone();
                    let text = input.read(cx).value().to_string();
                    self.commit_description(text, &input, window, cx);
                }
            }
        }
    }

    /// Puts back the stored value of the field being edited (Escape).
    fn revert(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        self.edited.retain(|edited| *edited != Some(field));
        let stored = field.stored(&self.editor.read(cx).session().metadata());
        let input = self.input(field).clone();
        input.update(cx, |input, cx| input.set_value(stored, window, cx));
        window.blur(cx);
    }

    fn input(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Name => &self.name,
            Field::Creator => &self.creator,
            Field::Version => &self.version,
        }
    }

    /// Brings the fields the author isn't in up to date with the pack (undo, redo, another view).
    fn follow_pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let metadata = self.editor.read(cx).session().metadata();
        for field in [Field::Name, Field::Creator, Field::Version] {
            let input = self.input(field).clone();
            let stored = field.stored(&metadata);
            if !input.focus_handle(cx).is_focused(window)
                && input.read(cx).value().as_ref() != stored
            {
                input.update(cx, |input, cx| input.set_value(stored, window, cx));
            }
        }
        let stored = metadata.description.unwrap_or_default();
        if !self.description.focus_handle(cx).is_focused(window)
            && self.description.read(cx).value().as_ref() != stored
        {
            self.description
                .update(cx, |input, cx| input.set_value(stored, window, cx));
        }
        cx.notify();
    }

    fn line_field(
        &self,
        field: Field,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let view = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(field_label(label))
            .child(
                div()
                    .h(theme::CONTROL_NORMAL)
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .bg(c(theme::SURFACE))
                    .hover(|style| style.border_color(c(theme::BORDER_STRONG)))
                    .on_key_down(move |event, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            view.update(cx, |this, cx| this.revert(field, window, cx));
                        }
                    })
                    .child(
                        Input::new(self.input(field))
                            .appearance(false)
                            .text_size(px(13.))
                            .flex_1(),
                    ),
            )
            .into_any_element()
    }
}

fn field_label(text: &'static str) -> gpui_kit::Div {
    div()
        .text_size(px(12.))
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(c(theme::TEXT))
        .child(text)
}

impl Render for MetadataView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.commit_unfocused(window, cx);
        let fields = div()
            .max_w(px(512.))
            .flex()
            .flex_col()
            .gap_3()
            .child(self.line_field(Field::Name, "Name", cx))
            .child(self.line_field(Field::Creator, "Creator", cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.))
                    .child(field_label("Description"))
                    .child(
                        div()
                            .px_2()
                            .py(px(6.))
                            .rounded(theme::RADIUS_SM)
                            .border_1()
                            .border_color(c(theme::BORDER))
                            .bg(c(theme::SURFACE))
                            .hover(|style| style.border_color(c(theme::BORDER_STRONG)))
                            .child(
                                Textarea::new(&self.description)
                                    .appearance(false)
                                    .text_size(px(13.)),
                            ),
                    ),
            )
            .child(self.line_field(Field::Version, "Version", cx));
        ui::page_body("metadata-page", [fields.into_any_element()])
    }
}
