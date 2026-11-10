//! The pack's wallpaper and splash (`MediaSlot.svelte`). Each is a `MediaSlotView`, the slot the
//! timeline's stage slots use too; this module owns the two and the headings above them.

use gpui_kit::prelude::*;
use gpui_kit::{div, AnyElement, Context, Entity, Window};
use shared::behaviour::MediaSlot;

use super::ContentView;
use crate::editor::Editor;
use crate::ui;
use crate::views::media_slot::MediaSlotView;

/// The pack-wide slots this view edits. Stage slots belong to the timeline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Wallpaper,
    Splash,
}

impl Slot {
    fn title(self) -> &'static str {
        match self {
            Slot::Wallpaper => "Wallpaper",
            Slot::Splash => "Splash",
        }
    }

    fn empty_note(self) -> &'static str {
        match self {
            Slot::Wallpaper => "Lewdware will not change the wallpaper.",
            Slot::Splash => "No startup image or video will be shown.",
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

pub(super) struct SlotsState {
    wallpaper: Entity<MediaSlotView>,
    splash: Entity<MediaSlotView>,
}

impl SlotsState {
    pub(super) fn new(editor: &Entity<Editor>, cx: &mut Context<ContentView>) -> Self {
        Self {
            wallpaper: cx.new(|cx| MediaSlotView::new(editor.clone(), MediaSlot::Wallpaper, cx)),
            splash: cx.new(|cx| MediaSlotView::new(editor.clone(), MediaSlot::Splash, cx)),
        }
    }

    pub(super) fn close_picker(&self, cx: &mut Context<ContentView>) {
        for slot in [&self.wallpaper, &self.splash] {
            slot.update(cx, |slot, cx| slot.close_picker(cx));
        }
    }
}

impl ContentView {
    pub(super) fn slots_section(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.slot(Slot::Wallpaper, cx))
            .child(self.slot(Slot::Splash, cx))
            .into_any_element()
    }

    fn slot(&mut self, slot: Slot, cx: &mut Context<Self>) -> AnyElement {
        let entity = match slot {
            Slot::Wallpaper => self.slots.wallpaper.clone(),
            Slot::Splash => self.slots.splash.clone(),
        };
        let current = slot.current(self);
        entity.update(cx, |view, cx| view.set(current, slot.empty_note().into(), cx));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(ui::section_title(slot.title()))
            .child(entity)
            .into_any_element()
    }
}
