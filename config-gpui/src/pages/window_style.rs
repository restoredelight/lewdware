//! How popup frames, buttons and text fields look. Ported from
//! `config/src/lib/WindowStyle.svelte`.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, Context, Entity, Window, WindowAppearance};
use shared::theme::{Appearance, AppearanceChoice, APPEARANCES};

use crate::pages::theme_preview::ThemePreview;
use crate::store::Store;
use crate::theme::{self, c};
use crate::ui::{self, SelectOption};

pub struct WindowStylePage {
    store: Entity<Store>,
}

impl WindowStylePage {
    pub fn new(store: Entity<Store>, _: &mut Window, _: &mut Context<Self>) -> Self {
        Self { store }
    }
}

/// What `auto` previews as: the appearance the engine will actually use. On Linux that is the
/// XDG portal's answer, read the same way the engine reads it; elsewhere the window's own. No
/// answer falls back to light, as the engine does.
fn system_appearance(window: &Window) -> Appearance {
    if cfg!(target_os = "linux") {
        shared::theme::system_appearance().unwrap_or(Appearance::Light)
    } else {
        match window.appearance() {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
            _ => Appearance::Light,
        }
    }
}

impl Render for WindowStylePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let Some(config) = store.config.as_ref() else {
            return div().into_any_element();
        };
        let current = config.theme;
        let preview_appearance = match config.appearance {
            AppearanceChoice::Light => Appearance::Light,
            AppearanceChoice::Dark => Appearance::Dark,
            AppearanceChoice::Auto => system_appearance(window),
        };
        let selected = store.themes.iter().find(|t| t.is_selected(current));
        let note = match selected {
            Some(t) if !t.supports_dark && preview_appearance == Appearance::Dark => Some(format!(
                "{} has no dark version and is always drawn light.",
                t.label
            )),
            Some(t) if t.matches_system => Some(
                "Windows are drawn to look like this machine's own, so a pack you share won't look the same on someone else's."
                    .to_string(),
            ),
            _ => None,
        };

        let appearance_entity = self.store.clone();
        let appearance_select = ui::Select::new(
            "appearance",
            config.appearance,
            APPEARANCES
                .iter()
                .filter_map(|info| {
                    AppearanceChoice::from_name(info.name)
                        .map(|choice| SelectOption::new(choice, info.label))
                })
                .collect(),
        )
        .on_change(move |choice, _, cx| {
            appearance_entity.update(cx, |store, cx| store.set_appearance(choice, cx))
        });

        let cards = store.themes.iter().enumerate().map(|(index, entry)| {
            let is_selected = entry.is_selected(current);
            let choice = entry.choice;
            let entity = self.store.clone();
            let tag = if entry.matches_system {
                Some(if choice.name() == "native-retro" {
                    "your system, retro"
                } else {
                    "matches your system"
                })
            } else if !entry.supports_dark {
                Some("light only")
            } else {
                None
            };

            div()
                .flex_grow(1.)
                .flex_basis(px(270.))
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(theme::RADIUS_SM)
                .border_1()
                .border_color(c(if is_selected {
                    theme::ACCENT
                } else {
                    theme::BORDER
                }))
                .bg(c(theme::BG))
                .child(
                    div()
                        .id(("theme-pick", index))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |store, cx| store.set_theme(choice, cx))
                        })
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .size(px(14.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(c(if is_selected {
                                            theme::ACCENT
                                        } else {
                                            theme::BORDER_STRONG
                                        }))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .when(is_selected, |this| {
                                            this.child(
                                                div()
                                                    .size(px(7.))
                                                    .rounded_full()
                                                    .bg(c(theme::ACCENT)),
                                            )
                                        }),
                                )
                                .child(ui::label(entry.label)),
                        )
                        .when_some(tag, |this, tag| {
                            this.child(ui::readout(tag).text_size(px(10.)))
                        }),
                )
                // The preview stays interactive: trying the widgets is its point, not a second way
                // to select.
                .child(ThemePreview::new(
                    ("theme-preview", index),
                    *entry.look(preview_appearance),
                    entry.label,
                ))
        });

        let section = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(ui::section_title("Theme"))
                            .child(ui::hint(
                                "Each preview is live -- hover its close button, press a button, or type in the field.",
                            )),
                    )
                    .child(div().w(px(176.)).flex().child(appearance_select)),
            )
            .child(
                ui::card()
                    .gap_4()
                    .p_4()
                    .child(div().flex().flex_wrap().gap_3().children(cards))
                    .when_some(note, |this, note| this.child(ui::hint(note))),
            );

        ui::page(
            "window-style",
            "Window style",
            "Choose how popup frames, buttons and text fields look. A mode can still choose a style for a particular window where the look is part of what it's doing.",
            [section.into_any_element()],
        )
        .into_any_element()
    }
}
