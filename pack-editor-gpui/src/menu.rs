//! A button that opens a small menu of actions below it: the toolbar's "Import" and "More"
//! menus (`shared-ui/Popover.svelte` with a list of menu items in the Tauri editor). Styled like
//! `ui::Select`'s list. A candidate for `shared-gpui` once something else needs it.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::{div, px, App, ElementId, SharedString, Window};

use crate::theme::{self, c};
use gpui_kit::assets::IconName;

use crate::ui::{Button, IconButton, Size, Variant};

type Handler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

pub enum MenuEntry {
    Item {
        label: SharedString,
        /// Mono, muted, at the right edge: a shortcut.
        hint: Option<SharedString>,
        /// Coral: it throws something away.
        destructive: bool,
        disabled: bool,
        on_click: Handler,
    },
    Separator,
}

impl MenuEntry {
    pub fn item(
        label: impl Into<SharedString>,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        MenuEntry::Item {
            label: label.into(),
            hint: None,
            destructive: false,
            disabled: false,
            on_click: Rc::new(on_click),
        }
    }

    pub fn hint(mut self, text: impl Into<SharedString>) -> Self {
        if let MenuEntry::Item { hint, .. } = &mut self {
            *hint = Some(text.into());
        }
        self
    }

    pub fn destructive(mut self) -> Self {
        if let MenuEntry::Item { destructive, .. } = &mut self {
            *destructive = true;
        }
        self
    }

    pub fn disabled(mut self, value: bool) -> Self {
        if let MenuEntry::Item { disabled, .. } = &mut self {
            *disabled = value;
        }
        self
    }
}

#[derive(IntoElement)]
pub struct Menu {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    /// The trigger is just the icon, with the label as its tooltip.
    icon_only: bool,
    variant: Variant,
    disabled: bool,
    entries: Vec<MenuEntry>,
}

impl Menu {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            icon_only: false,
            variant: Variant::Quiet,
            disabled: false,
            entries: Vec::new(),
        }
    }

    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// An icon before the trigger's label.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The trigger is `icon` alone, named by the label in a tooltip.
    pub fn icon_only(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self.icon_only = true;
        self
    }

    pub fn entry(mut self, entry: MenuEntry) -> Self {
        self.entries.push(entry);
        self
    }
}

impl RenderOnce for Menu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let open_state = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let open = *open_state.read(cx) && !self.disabled;
        let toggle = open_state.clone();
        let on_toggle = move |_: &gpui_kit::ClickEvent, _: &mut Window, cx: &mut App| {
            toggle.update(cx, |open, cx| {
                *open = !*open;
                cx.notify();
            })
        };
        let trigger = match self.icon {
            Some(icon) if self.icon_only => {
                let button = IconButton::new("trigger", icon, self.label)
                    .disabled(self.disabled)
                    .on_click(on_toggle);
                // The tooltip would sit over the menu it opened.
                if open {
                    button.without_tooltip()
                } else {
                    button
                }
                .into_any_element()
            }
            icon => {
                let button = Button::new("trigger", self.label)
                    .variant(self.variant)
                    .size(Size::Compact)
                    .disabled(self.disabled)
                    .on_click(on_toggle);
                match icon {
                    Some(icon) => button.icon(icon),
                    None => button,
                }
                .into_any_element()
            }
        };

        let menu = open.then(|| {
            let close = open_state.clone();
            let items = self
                .entries
                .into_iter()
                .enumerate()
                .map(|(index, entry)| match entry {
                    MenuEntry::Separator => div()
                        .my_1()
                        .h(px(1.))
                        .bg(c(theme::BORDER))
                        .into_any_element(),
                    MenuEntry::Item {
                        label,
                        hint,
                        destructive,
                        disabled,
                        on_click,
                    } => {
                        let open_state = open_state.clone();
                        div()
                            .id(index)
                            .px_2()
                            .py(px(6.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .rounded(theme::RADIUS_SM)
                            .text_size(px(12.))
                            .text_color(c(if destructive {
                                theme::DANGER
                            } else {
                                theme::TEXT
                            }))
                            .child(label)
                            .children(hint.map(|hint| {
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .text_size(px(10.))
                                    .text_color(c(theme::MUTED))
                                    .child(hint)
                            }))
                            .when(disabled, |this| this.opacity(0.45))
                            .when(!disabled, |this| {
                                this.cursor_pointer()
                                    .hover(move |style| {
                                        style.bg(c(if destructive {
                                            theme::DANGER_BG
                                        } else {
                                            theme::SURFACE
                                        }))
                                    })
                                    .on_click(move |_, window, cx| {
                                        open_state.update(cx, |open, cx| {
                                            *open = false;
                                            cx.notify();
                                        });
                                        on_click(window, cx);
                                    })
                            })
                            .into_any_element()
                    }
                });
            let menu = crate::ui::floating(
                theme::CONTROL_COMPACT + px(4.),
                true,
                div()
                    .id("menu")
                    .w(px(200.))
                    .p_1()
                    .flex()
                    .flex_col()
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
                    .occlude()
                    .children(items),
            );
            // Clicking anywhere else closes it, the trigger included (so that click doesn't
            // reopen it straight away).
            let viewport = window.viewport_size();
            let backdrop = gpui_kit::deferred(
                gpui_kit::anchored()
                    .position(gpui_kit::point(px(0.), px(0.)))
                    .child(
                        div()
                            .id("menu-backdrop")
                            .block_mouse_except_scroll()
                            .w(viewport.width)
                            .h(viewport.height)
                            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, _, cx| {
                                close.update(cx, |open, cx| {
                                    *open = false;
                                    cx.notify();
                                })
                            }),
                    ),
            )
            .with_priority(crate::ui::POPOVER_BACKDROP_PRIORITY);
            [backdrop.into_any_element(), menu.into_any_element()]
        });

        div()
            .id(self.id)
            .relative()
            .flex_none()
            .child(trigger)
            .children(menu.into_iter().flatten())
    }
}
