//! The Content view: content groups, captions, prompts, notifications, web links, and the pack's
//! wallpaper and splash. Ported from `pack-editor/src/lib/Content.svelte` and what it contains
//! (`ContentList`, `MessageEntriesEditor`, `WebLinksEditor`, `ContentGroupsEditor`, `MediaSlot`,
//! `ExplicitMediaPicker`, `TagPicker`).
//!
//! Follows `views/library/tags.rs`: reads come from the session (cached here and re-read when a
//! change in one of the view's scopes arrives, from whatever made it), writes go through
//! `Editor::apply`, and text commits on Enter or leaving the field (`fields.rs`).
//!
//! The pools can hold hundreds of entries (a converted Edgeware pack's captions), so a pool is a
//! gpui `list`: only the cards on screen are laid out. A field isn't an input until it's clicked,
//! so a pool costs no input entities either; only the one being edited has one.

mod cards;
mod fields;
mod slots;

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, list, px, AnyElement, AnyView, App, Context, Entity, FocusHandle, FontWeight,
    ListAlignment, ListState, SharedString, Subscription, Window,
};
use pack_editor_core::changes::{Scope, Scopes};
use pack_editor_core::pack::ops::content::{self as content_ops, Entry, EntryKind};
use pack_editor_core::session::Session;
use shared::behaviour::editor::{self as behaviour_editor, MediaSlots};
use shared::behaviour::ContentGroup;

use crate::editor::{CommitPending, Editor, EditorEvent};
use crate::shell::View;
use crate::theme::{self, c};
use crate::ui::{self, Button, DialogAction, Size, Variant};

use fields::{Editing, Field};
use slots::SlotsState;

pub fn create(view: View, editor: Entity<Editor>, window: &mut Window, cx: &mut App) -> AnyView {
    debug_assert!(matches!(view, View::Content));
    cx.new(|cx| ContentView::new(editor, window, cx)).into()
}

/// What this view shows, and so the changes it reloads for.
fn scopes() -> Scopes {
    Scope::Entries | Scope::ContentGroups | Scope::Slots | Scope::Tags | Scope::Media
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Groups,
    Entries(EntryKind),
    Slots,
}

impl Tab {
    const ALL: [(&'static str, &'static [Tab]); 3] = [
        ("Organization", &[Tab::Groups]),
        (
            "Messages",
            &[
                Tab::Entries(EntryKind::Caption),
                Tab::Entries(EntryKind::Prompt),
                Tab::Entries(EntryKind::Notification),
            ],
        ),
        ("Other", &[Tab::Entries(EntryKind::WebLink), Tab::Slots]),
    ];

    fn title(self) -> &'static str {
        match self {
            Tab::Groups => "Content groups",
            Tab::Entries(EntryKind::Caption) => "Captions",
            Tab::Entries(EntryKind::Prompt) => "Prompts",
            Tab::Entries(EntryKind::Notification) => "Notifications",
            Tab::Entries(EntryKind::WebLink) => "Web links",
            Tab::Slots => "Wallpaper and splash",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Tab::Groups => {
                "Create collections people can enable or disable. Media and messages with any of a group's tags belong to that collection."
            }
            Tab::Entries(EntryKind::Caption) => {
                "Short messages shown alongside popup media. Tagged captions are only used with media carrying a matching tag."
            }
            Tab::Entries(EntryKind::Prompt) => "Questions that ask the user for a typed response.",
            Tab::Entries(EntryKind::Notification) => {
                "Messages displayed as desktop notifications. A notification can carry a title above its message; leave it blank to show the message on its own."
            }
            Tab::Entries(EntryKind::WebLink) => {
                "Links the experience may open in the user's browser."
            }
            Tab::Slots => {
                "Pick the image Lewdware uses as the desktop wallpaper, and the one it shows when a session starts."
            }
        }
    }

    fn id(self) -> &'static str {
        match self {
            Tab::Groups => "content-tab-groups",
            Tab::Entries(EntryKind::Caption) => "content-tab-captions",
            Tab::Entries(EntryKind::Prompt) => "content-tab-prompts",
            Tab::Entries(EntryKind::Notification) => "content-tab-notifications",
            Tab::Entries(EntryKind::WebLink) => "content-tab-web-links",
            Tab::Slots => "content-tab-slots",
        }
    }
}

/// Everything the view shows, read from the session when a change in its scopes arrives.
#[derive(Default)]
struct Data {
    captions: Vec<Entry>,
    prompts: Vec<Entry>,
    notifications: Vec<Entry>,
    web_links: Vec<Entry>,
    groups: Vec<ContentGroup>,
    slots: Option<MediaSlots>,
    /// The pack's author tags, for suggestions.
    tags: Vec<String>,
}

impl Data {
    fn load(session: &Session) -> anyhow::Result<Self> {
        session.read(|c| {
            Ok(Data {
                captions: content_ops::entries(c, EntryKind::Caption)?,
                prompts: content_ops::entries(c, EntryKind::Prompt)?,
                notifications: content_ops::entries(c, EntryKind::Notification)?,
                web_links: content_ops::entries(c, EntryKind::WebLink)?,
                groups: behaviour_editor::content_groups(c)?,
                slots: Some(behaviour_editor::media_slots(c)?),
                tags: content_ops::author_tags(c)?,
            })
        })
    }

    fn entries(&self, kind: EntryKind) -> &[Entry] {
        match kind {
            EntryKind::Caption => &self.captions,
            EntryKind::Prompt => &self.prompts,
            EntryKind::Notification => &self.notifications,
            EntryKind::WebLink => &self.web_links,
        }
    }

    fn entry(&self, kind: EntryKind, id: i64) -> Option<&Entry> {
        self.entries(kind).iter().find(|entry| entry.id == id)
    }

    fn group(&self, key: &str) -> Option<&ContentGroup> {
        self.groups.iter().find(|group| group.id == key)
    }

    fn count(&self, tab: Tab) -> Option<usize> {
        match tab {
            Tab::Groups => Some(self.groups.len()),
            Tab::Entries(kind) => Some(self.entries(kind).len()),
            Tab::Slots => None,
        }
    }
}

/// What a confirmed removal removes.
#[derive(Clone)]
enum Removal {
    Entry(EntryKind, i64),
    Group(String),
}

struct Removing {
    removal: Removal,
    title: SharedString,
    description: SharedString,
    confirm: SharedString,
}

/// Identifies a card in the list, so the list can be told which cards came and went.
#[derive(Clone, PartialEq, Eq)]
enum CardKey {
    Entry(i64),
    Group(String),
}

pub struct ContentView {
    editor: Entity<Editor>,
    tab: Tab,
    data: Data,
    /// The field being edited, if any: the only input entity the view holds.
    editing: Option<Editing>,
    removing: Option<Removing>,
    dialog_focus: FocusHandle,
    /// The view's own focus, which it takes back when a field closes (`fields::end_editing`).
    focus: FocusHandle,
    /// The current pool's cards.
    list: ListState,
    /// What `list` was last told it holds, in order.
    cards: Vec<CardKey>,
    slots: SlotsState,
    /// Whether the "Make tag toggleable..." menu is open.
    toggle_menu_open: bool,
    _changes: Subscription,
    _commit: Subscription,
}

impl ContentView {
    fn new(editor: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _commit = cx.subscribe_in(&editor, window, |this, _, _: &CommitPending, window, cx| {
            this.commit_editing(window, cx)
        });
        let _changes = cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &EditorEvent, window, cx| match event {
                EditorEvent::Changed(changes) => {
                    if changes.scopes.intersects(scopes()) {
                        this.slots.forget_thumbnails(&changes.media);
                        this.reload(window, cx);
                    }
                }
            },
        );
        let mut this = Self {
            editor,
            tab: Tab::Entries(EntryKind::Caption),
            data: Data::default(),
            editing: None,
            removing: None,
            dialog_focus: cx.focus_handle(),
            focus: cx.focus_handle(),
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            cards: Vec::new(),
            slots: SlotsState::default(),
            toggle_menu_open: false,
            _changes,
            _commit,
        };
        this.reload(window, cx);
        this
    }

    /// Re-reads everything the view shows and tells the list which cards changed. Safe to call
    /// more than once per change: an edit made here reloads straight away (so the view can focus
    /// what it just added), and again when the change arrives.
    fn reload(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        match Data::load(self.editor.read(cx).session()) {
            Ok(data) => self.data = data,
            Err(error) => tracing::warn!("couldn't read the pack's content: {error:#}"),
        }
        self.sync_list();
        cx.notify();
    }

    fn card_keys(&self) -> Vec<CardKey> {
        match self.tab {
            Tab::Groups => self
                .data
                .groups
                .iter()
                .map(|group| CardKey::Group(group.id.clone()))
                .collect(),
            Tab::Entries(kind) => self
                .data
                .entries(kind)
                .iter()
                .map(|entry| CardKey::Entry(entry.id))
                .collect(),
            Tab::Slots => Vec::new(),
        }
    }

    /// Splices the cards that changed into the list, keeping the scroll position where it was,
    /// and has every card measured again (an edit can change any card's height).
    fn sync_list(&mut self) {
        let new = self.card_keys();
        let old = std::mem::take(&mut self.cards);
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        if old != new {
            self.list
                .splice(prefix..old.len() - suffix, new.len() - prefix - suffix);
        }
        if !new.is_empty() {
            self.list.remeasure_items(0..new.len());
        }
        self.cards = new;
    }

    fn select_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.commit_editing(window, cx);
        self.slots.close_picker();
        self.toggle_menu_open = false;
        self.tab = tab;
        self.cards = self.card_keys();
        self.list.reset(self.cards.len());
        cx.notify();
    }

    /// Scrolls the list to `key`'s card.
    fn reveal(&mut self, key: &CardKey) {
        if let Some(index) = self.cards.iter().position(|card| card == key) {
            self.list.scroll_to_reveal_item(index);
        }
    }

    // ── Adding and removing ─────────────────────────────────────────────────────────────────

    fn add_entry(&mut self, kind: EntryKind, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_editing(window, cx);
        let added = self
            .editor
            .update(cx, |editor, cx| editor.apply(cx, |s| s.add_entry(kind)));
        let Some(id) = added else { return };
        self.reload(window, cx);
        self.reveal(&CardKey::Entry(id));
        // The card's first field, as the Tauri editor focused it.
        let field = match kind {
            EntryKind::Notification => Field::NotificationTitle(id),
            _ => Field::Text(kind, id),
        };
        self.start_editing(field, window, cx);
    }

    fn add_group(&mut self, tag: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_editing(window, cx);
        let added = self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |s| match &tag {
                Some(tag) => s.add_content_group_from_tag(tag),
                None => s.add_content_group(),
            })
        });
        let Some(key) = added else { return };
        self.reload(window, cx);
        self.reveal(&CardKey::Group(key.clone()));
        self.start_editing(Field::GroupLabel(key), window, cx);
    }

    /// Removes straight away when there's nothing to lose, and asks first otherwise.
    fn request_removal(&mut self, removal: Removal, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_editing(window, cx);
        let removing = match &removal {
            Removal::Entry(kind, id) => {
                let Some(entry) = self.data.entry(*kind, *id) else {
                    return;
                };
                entry.has_content().then(|| Removing {
                    removal: removal.clone(),
                    title: format!("Remove {}?", kind.noun()).into(),
                    description: format!(
                        "This {} and its tag assignments will be removed. You can undo this from the toolbar.",
                        kind.noun()
                    )
                    .into(),
                    confirm: if *kind == EntryKind::WebLink {
                        "Remove link".into()
                    } else {
                        "Remove".into()
                    },
                })
            }
            Removal::Group(key) => {
                let Some(group) = self.data.group(key) else {
                    return;
                };
                content_ops::group_has_content(group).then(|| Removing {
                    removal: removal.clone(),
                    title: format!("Remove \"{}\"?", group.label).into(),
                    description: "This content group will be removed. Its tags and matching media will remain available, and you can undo this from the toolbar.".into(),
                    confirm: "Remove group".into(),
                })
            }
        };
        match removing {
            Some(removing) => {
                self.removing = Some(removing);
                self.dialog_focus.focus(window, cx);
                cx.notify();
            }
            None => self.remove(removal, cx),
        }
    }

    fn remove(&mut self, removal: Removal, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.apply(cx, |s| match &removal {
                Removal::Entry(kind, id) => s.remove_entry(*kind, *id),
                Removal::Group(key) => s.remove_content_group(key),
            })
        });
    }

    fn confirm_removal(&mut self, cx: &mut Context<Self>) {
        if let Some(removing) = self.removing.take() {
            self.remove(removing.removal, cx);
        }
        cx.notify();
    }

    fn cancel_removal(&mut self, cx: &mut Context<Self>) {
        self.removing = None;
        cx.notify();
    }

    // ── Rendering ───────────────────────────────────────────────────────────────────────────

    fn nav(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let groups =
            Tab::ALL.iter().map(|(heading, tabs)| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .px_3()
                            .pt_3()
                            .pb_1()
                            .text_size(px(11.))
                            .text_color(c(theme::MUTED))
                            .child(*heading),
                    )
                    .children(tabs.iter().map(|&tab| {
                        let active = tab == self.tab;
                        div()
                            .id(tab.id())
                            .w_full()
                            .px_3()
                            .py_2()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(theme::RADIUS_SM)
                            .border_l_2()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .map(|this| {
                                if active {
                                    this.bg(c(theme::SURFACE_RAISED))
                                        .border_color(c(theme::ACCENT_HOVER))
                                        .text_color(c(theme::TEXT))
                                } else {
                                    this.border_color(gpui_kit::transparent_black())
                                        .text_color(c(theme::MUTED))
                                        .hover(|style| style.text_color(c(theme::TEXT)))
                                }
                            })
                            .child(div().flex_1().min_w_0().truncate().child(tab.title()))
                            .children(
                                self.data
                                    .count(tab)
                                    .map(|count| ui::readout(count.to_string())),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_tab(tab, window, cx)
                            }))
                    }))
            });
        div()
            .flex_none()
            .w(px(200.))
            .h_full()
            .px_2()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(c(theme::BORDER))
            .bg(c(theme::SURFACE))
            .children(groups)
    }

    /// The note, title and description above every tab.
    fn heading(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(ui::hint(
                "Read by the built-in modes (Sandbox and Sequence). A custom mode reads none of this.",
            ))
            .child(
                div()
                    .max_w(px(672.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(ui::page_title(self.tab.title()))
                    .child(ui::hint(self.tab.description()).text_size(px(13.))),
            )
    }

    /// The bar over a pool: how many there are, and the way to add one.
    fn toolbar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let count = self.cards.len();
        let (add_label, extra): (SharedString, Option<AnyElement>) = match self.tab {
            Tab::Entries(kind) => {
                // With nothing to count, the empty state's own button is the way to add the
                // first one; two Add buttons on one screen is one too many.
                if count == 0 {
                    return None;
                }
                (format!("Add {}", kind.noun()).into(), None)
            }
            // Groups keep the bar: it also carries the "make a tag toggleable" shortcut.
            Tab::Groups => ("Add group".into(), self.toggle_tag_select(cx)),
            Tab::Slots => return None,
        };
        let tab = self.tab;
        Some(
            div()
                .flex_none()
                .py_2()
                .flex()
                .items_center()
                .gap_3()
                .border_y_1()
                .border_color(c(theme::BORDER))
                .child(ui::readout(format!(
                    "{count} {}",
                    if count == 1 { "item" } else { "items" }
                )))
                .child(div().flex_1())
                .children(extra)
                .child(
                    Button::new("content-add", add_label)
                        .size(Size::Compact)
                        .on_click(cx.listener(move |this, _, window, cx| match tab {
                            Tab::Entries(kind) => this.add_entry(kind, window, cx),
                            _ => this.add_group(None, window, cx),
                        })),
                )
                .into_any_element(),
        )
    }

    /// "Make tag toggleable...": every tag no group has yet, one click from being a group. Styled
    /// as `ui::Select`, with its open state on the view.
    fn toggle_tag_select(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let grouped: std::collections::HashSet<&str> = self
            .data
            .groups
            .iter()
            .flat_map(|group| group.tags.iter().map(String::as_str))
            .collect();
        let available: Vec<String> = self
            .data
            .tags
            .iter()
            .filter(|tag| !grouped.contains(tag.as_str()))
            .cloned()
            .collect();
        if available.is_empty() {
            return None;
        }
        let open = self.toggle_menu_open;
        let trigger = div()
            .id("toggle-tag")
            .h(theme::CONTROL_COMPACT)
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(if open {
                theme::FOCUS
            } else {
                theme::BORDER_STRONG
            }))
            .bg(c(theme::BG))
            .cursor_pointer()
            .hover(|style| style.border_color(c(theme::MUTED)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .child("Make tag toggleable..."),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(c(theme::MUTED))
                    .child(if open { "▲" } else { "▼" }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.toggle_menu_open = !this.toggle_menu_open;
                cx.notify();
            }));
        let menu = open.then(|| {
            gpui_kit::deferred(
                div()
                    .id("toggle-tag-menu")
                    .occlude()
                    .absolute()
                    .top(theme::CONTROL_COMPACT + px(4.))
                    .left_0()
                    .right_0()
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .p_1()
                    .rounded(theme::RADIUS_MD)
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .bg(c(theme::SURFACE_RAISED))
                    .shadow(vec![gpui_kit::BoxShadow {
                        color: theme::shadow_pop(),
                        offset: gpui_kit::point(px(4.), px(4.)),
                        blur_radius: px(0.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.toggle_menu_open = false;
                        cx.notify();
                    }))
                    .children(available.into_iter().enumerate().map(|(index, tag)| {
                        div()
                            .id(index)
                            .px_2()
                            .py(px(6.))
                            .rounded(theme::RADIUS_SM)
                            .text_size(px(12.))
                            .truncate()
                            .cursor_pointer()
                            .hover(|style| style.bg(c(theme::SURFACE)))
                            .child(tag.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.toggle_menu_open = false;
                                this.add_group(Some(tag.clone()), window, cx);
                            }))
                    })),
            )
            .with_priority(1)
        });
        Some(
            div()
                .relative()
                .w(px(220.))
                .child(trigger)
                .children(menu)
                .into_any_element(),
        )
    }

    fn render_card(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = match (self.tab, self.cards.get(index).cloned()) {
            (Tab::Entries(kind), Some(CardKey::Entry(id))) => self
                .data
                .entry(kind, id)
                .cloned()
                .map(|entry| self.entry_card(kind, index, &entry, window, cx)),
            (Tab::Groups, Some(CardKey::Group(key))) => self
                .data
                .group(&key)
                .cloned()
                .map(|group| self.group_card(index, &group, window, cx)),
            _ => None,
        };
        div().pb_2().children(body).into_any_element()
    }

    fn pool(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.cards.is_empty() {
            return div()
                .flex_1()
                .min_h_0()
                .child(self.empty_state(cx))
                .into_any_element();
        }
        div()
            .id("content-cards")
            .relative()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|this, index: usize, window, cx| {
                        this.render_card(index, window, cx)
                    }),
                )
                .flex_1()
                .pt_2(),
            )
            .vertical_scrollbar(&self.list)
            .into_any_element()
    }

    fn removal_dialog(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let removing = self.removing.as_ref()?;
        let cancel = cx.entity();
        let confirm = cx.entity();
        let close = cx.entity();
        Some(
            ui::dialog(
                removing.title.clone(),
                removing.description.clone(),
                vec![
                    DialogAction::new("Cancel", Variant::Secondary, move |_, _, cx| {
                        cancel.update(cx, |this, cx| this.cancel_removal(cx))
                    }),
                    DialogAction::new(
                        removing.confirm.clone(),
                        Variant::Destructive,
                        move |_, _, cx| confirm.update(cx, |this, cx| this.confirm_removal(cx)),
                    ),
                ],
                move |_, cx| close.update(cx, |this, cx| this.cancel_removal(cx)),
                &self.dialog_focus,
                window,
            )
            .into_any_element(),
        )
    }
}

impl Render for ContentView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main = match self.tab {
            Tab::Slots => div()
                .id("content-slots")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div()
                        .mx_auto()
                        .w_full()
                        .max_w(px(848.))
                        .px_6()
                        .pt_6()
                        .pb_6()
                        .flex()
                        .flex_col()
                        .gap_6()
                        .child(self.heading())
                        .child(self.slots_section(window, cx)),
                )
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_h_0()
                .w_full()
                .max_w(px(848.))
                .mx_auto()
                .px_6()
                .pt_6()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.heading())
                .children(self.toolbar(cx))
                .child(self.pool(cx))
                .into_any_element(),
        };
        div()
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .child(self.nav(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(main),
            )
            .children(self.removal_dialog(window, cx))
    }
}
