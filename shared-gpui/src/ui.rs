//! Small components that carry the design guide's rules (`shared-ui/DESIGN.md`), ported from
//! `shared-ui/*.svelte`. gpui-component is still used for the heavier widgets (inputs, selects,
//! scrolling); these are the pieces whose look is specific enough to be worth owning.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, App, ClickEvent, ElementId, FontWeight, SharedString, Window};

use crate::theme::{self, c};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Carmine fill: the single primary action on a surface.
    Primary,
    /// Bordered.
    Secondary,
    /// Transparent, muted until hovered.
    Quiet,
    /// Coral outline and text, never filled.
    Destructive,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Compact,
    Normal,
}

/// `shared-ui/Button.svelte`. The label stays put while loading so the control never changes
/// size; the spinner from the webview build is replaced by a dimmed label for now.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    variant: Variant,
    size: Size,
    disabled: bool,
    loading: bool,
    full_width: bool,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            variant: Variant::Secondary,
            size: Size::Normal,
            disabled: false,
            loading: false,
            full_width: false,
            on_click: None,
        }
    }

    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let inert = self.disabled || self.loading;
        let (bg, hover_bg, border, fg, hover_fg) = match self.variant {
            Variant::Primary => (
                c(theme::ACCENT),
                c(theme::ACCENT_HOVER),
                c(theme::ACCENT),
                c(theme::TEXT),
                c(theme::TEXT),
            ),
            Variant::Secondary => (
                c(theme::SURFACE),
                c(theme::SURFACE_RAISED),
                c(theme::BORDER_STRONG),
                c(theme::TEXT),
                c(theme::TEXT),
            ),
            Variant::Quiet => (
                gpui_kit::transparent_black(),
                c(theme::SURFACE_RAISED),
                gpui_kit::transparent_black(),
                c(theme::MUTED),
                c(theme::TEXT),
            ),
            Variant::Destructive => (
                gpui_kit::transparent_black(),
                c(theme::DANGER_BG),
                c(theme::DANGER_BORDER),
                c(theme::DANGER),
                c(theme::DANGER),
            ),
        };

        div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .px_3()
            .h(match self.size {
                Size::Compact => theme::CONTROL_COMPACT,
                Size::Normal => theme::CONTROL_NORMAL,
            })
            .when(self.full_width, |this| this.w_full())
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_color(fg)
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(self.label)
            .when(self.disabled, |this| this.opacity(0.45))
            .when(self.loading, |this| this.opacity(0.7))
            .when(!inert, |this| {
                this.cursor_pointer()
                    .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
                    .when_some(self.on_click, |this, handler| {
                        this.on_click(move |event, window, cx| handler(event, window, cx))
                    })
            })
    }
}

/// One entry in the vertical settings nav (`shared-ui/Tabs.svelte`, vertical). Active is the
/// raised surface plus a 2px carmine edge with the text left white: the edge carries the accent.
#[derive(IntoElement)]
pub struct NavItem {
    id: ElementId,
    label: SharedString,
    active: bool,
    on_click: Option<ClickHandler>,
}

impl NavItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            active,
            on_click: None,
        }
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for NavItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .w_full()
            .px_3()
            .py_2()
            .rounded(theme::RADIUS_SM)
            .border_l_2()
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .truncate()
            .cursor_pointer()
            .map(|this| {
                if self.active {
                    this.bg(c(theme::SURFACE_RAISED))
                        .border_color(c(theme::ACCENT_HOVER))
                        .text_color(c(theme::TEXT))
                } else {
                    this.border_color(gpui_kit::transparent_black())
                        .text_color(c(theme::MUTED))
                        .hover(|style| style.text_color(c(theme::TEXT)))
                }
            })
            .child(self.label)
            .when_some(self.on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
    }
}

/// `.ui-page-title`: 17px / 650.
pub fn page_title(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(17.))
        .font_weight(FontWeight(650.))
        .text_color(c(theme::TEXT))
        .child(text.into())
}

/// `.ui-metadata`: the readout layer. Mono, small, muted.
pub fn readout(text: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .font_family(theme::FONT_MONO)
        .text_size(px(11.))
        .text_color(c(theme::MUTED))
        .child(text.into())
}

/// `.ui-section-title`: sans, 14px / 700, sentence case.
pub fn section_title(text: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .text_size(px(14.))
        .font_weight(FontWeight::BOLD)
        .text_color(c(theme::TEXT))
        .child(text.into())
}

/// Secondary prose under a heading.
pub fn hint(text: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .text_size(px(12.))
        .text_color(c(theme::MUTED))
        .child(text.into())
}

/// A row or block heading inside a card: 13px / 500.
pub fn label(text: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(c(theme::TEXT))
        .child(text.into())
}

/// `shared-ui/Card.svelte`: a seamed panel, not a floating one.
pub fn card() -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .rounded(theme::RADIUS_MD)
        .border_1()
        .border_color(c(theme::BORDER))
        .bg(c(theme::SURFACE))
}

/// A horizontal seam between rows inside a card.
pub fn seam() -> gpui_kit::Div {
    div().h(px(1.)).w_full().bg(c(theme::BORDER))
}

/// The standard page frame: a scrolling column with a title and lede, capped at 896px wide.
pub fn page(
    id: &'static str,
    title: &'static str,
    lede: &'static str,
    sections: impl IntoIterator<Item = gpui_kit::AnyElement>,
) -> impl IntoElement {
    div().id(id).flex_1().min_h_0().overflow_y_scroll().child(
        div()
            .mx_auto()
            .w_full()
            .max_w(px(896.))
            .p_8()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .max_w(px(672.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(page_title(title))
                    .child(hint(lede).text_size(px(13.))),
            )
            .children(sections),
    )
}

type ToggleHandler = Box<dyn Fn(bool, &mut Window, &mut App) + 'static>;

/// `shared-ui/Toggle.svelte`. The track fills with the accent when on -- the one place outside
/// the primary button that carmine fills, because it is a state indicator inside a control.
#[derive(IntoElement)]
pub struct Toggle {
    id: ElementId,
    checked: bool,
    disabled: bool,
    on_change: Option<ToggleHandler>,
}

impl Toggle {
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        Self {
            id: id.into(),
            checked,
            disabled: false,
            on_change: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Toggle {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let checked = self.checked;
        div()
            .id(self.id)
            .flex_none()
            .w(px(34.))
            .h(px(20.))
            .p(px(2.))
            .rounded_full()
            .border_1()
            .border_color(c(if checked {
                theme::ACCENT
            } else {
                theme::BORDER_STRONG
            }))
            .bg(c(if checked { theme::ACCENT } else { theme::BG }))
            .flex()
            .when(checked, |this| this.justify_end())
            .child(div().size(px(14.)).rounded_full().bg(c(if checked {
                theme::TEXT
            } else {
                theme::MUTED
            })))
            .when(self.disabled, |this| this.opacity(0.45))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .when_some(self.on_change, |this, handler| {
                        this.on_click(move |_, window, cx| handler(!checked, window, cx))
                    })
            })
    }
}

pub struct RadioOption<T> {
    pub value: T,
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub disabled: bool,
}

impl<T> RadioOption<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
            description: None,
            disabled: false,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// `shared-ui/RadioGroup.svelte`: a column of options, the dot filling with the accent.
pub fn radio_group<T: PartialEq + Clone + 'static>(
    id: &'static str,
    selected: &T,
    options: Vec<RadioOption<T>>,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_change = std::rc::Rc::new(on_change);
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(options.into_iter().enumerate().map(move |(index, option)| {
            let checked = &option.value == selected;
            let on_change = on_change.clone();
            let value = option.value.clone();
            div()
                .id((id, index))
                .flex()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .mt(px(1.))
                        .size(px(16.))
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(c(if checked {
                            theme::ACCENT
                        } else {
                            theme::BORDER_STRONG
                        }))
                        .bg(c(theme::BG))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(checked, |this| {
                            this.child(div().size(px(8.)).rounded_full().bg(c(theme::ACCENT)))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().text_size(px(13.)).child(option.label))
                        .when_some(option.description, |this, description| {
                            this.child(hint(description))
                        }),
                )
                .when(option.disabled, |this| this.opacity(0.45))
                .when(!option.disabled && !checked, |this| {
                    this.cursor_pointer()
                        .on_click(move |_, window, cx| on_change(value.clone(), window, cx))
                })
        }))
}

pub struct SelectOption<T> {
    pub value: T,
    pub label: SharedString,
    pub disabled: bool,
}

impl<T> SelectOption<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// `shared-ui/Select.svelte`: a trigger showing the current choice and a menu under it. Whether
/// the menu is open is kept in keyed window state, so callers only own the value.
#[derive(IntoElement)]
pub struct Select<T: PartialEq + Clone + 'static> {
    id: ElementId,
    selected: T,
    options: Vec<SelectOption<T>>,
    disabled: bool,
    on_change: Option<SelectHandler<T>>,
}

type SelectHandler<T> = std::rc::Rc<dyn Fn(T, &mut Window, &mut App) + 'static>;

impl<T: PartialEq + Clone + 'static> Select<T> {
    pub fn new(id: impl Into<ElementId>, selected: T, options: Vec<SelectOption<T>>) -> Self {
        Self {
            id: id.into(),
            selected,
            options,
            disabled: false,
            on_change: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(T, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(std::rc::Rc::new(handler));
        self
    }
}

impl<T: PartialEq + Clone + 'static> RenderOnce for Select<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let open_state = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let open = *open_state.read(cx) && !self.disabled;
        let current = self
            .options
            .iter()
            .find(|option| option.value == self.selected)
            .map(|option| option.label.clone())
            .unwrap_or_else(|| "Select...".into());

        let toggle_state = open_state.clone();
        let trigger = div()
            .id("trigger")
            .h(theme::CONTROL_NORMAL)
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .child(current),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(c(theme::MUTED))
                    .child(if open { "▲" } else { "▼" }),
            )
            .when(self.disabled, |this| this.opacity(0.45))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .hover(|style| style.border_color(c(theme::MUTED)))
                    .on_click(move |_, _, cx| {
                        toggle_state.update(cx, |open, cx| {
                            *open = !*open;
                            cx.notify();
                        })
                    })
            });

        let menu = open.then(|| {
            let close_state = open_state.clone();
            gpui_kit::deferred(
                div()
                    .id("menu")
                    .absolute()
                    .top(theme::CONTROL_NORMAL + px(4.))
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
                    .on_mouse_down_out(move |_, _, cx| {
                        close_state.update(cx, |open, cx| {
                            *open = false;
                            cx.notify();
                        })
                    })
                    .children(self.options.into_iter().enumerate().map(|(index, option)| {
                        let selected = option.value == self.selected;
                        let on_change = self.on_change.clone();
                        let open_state = open_state.clone();
                        let value = option.value;
                        div()
                            .id(index)
                            .px_2()
                            .py(px(6.))
                            .rounded(theme::RADIUS_SM)
                            .text_size(px(13.))
                            .truncate()
                            .border_l_2()
                            .border_color(if selected {
                                c(theme::ACCENT_HOVER)
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .child(option.label)
                            .when(option.disabled, |this| this.text_color(c(theme::MUTED)))
                            .when(!option.disabled, |this| {
                                this.cursor_pointer()
                                    .hover(|style| style.bg(c(theme::SURFACE)))
                                    .on_click(move |_, window, cx| {
                                        open_state.update(cx, |open, cx| {
                                            *open = false;
                                            cx.notify();
                                        });
                                        if let Some(on_change) = &on_change {
                                            on_change(value.clone(), window, cx);
                                        }
                                    })
                            })
                    })),
            )
            .with_priority(1)
        });

        div()
            .id(self.id)
            .relative()
            .flex_1()
            .min_w_0()
            .child(trigger)
            .children(menu)
    }
}

/// `shared-ui/Checkbox.svelte`: the box fills with the accent when checked.
pub fn checkbox(checked: bool) -> gpui_kit::Div {
    div()
        .flex_none()
        .size(px(16.))
        .rounded(theme::RADIUS_SM)
        .border_1()
        .border_color(c(if checked {
            theme::ACCENT
        } else {
            theme::BORDER_STRONG
        }))
        .bg(c(if checked { theme::ACCENT } else { theme::BG }))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .text_color(c(theme::TEXT))
        .when(checked, |this| this.child("✓"))
}

pub struct DialogAction {
    pub label: SharedString,
    pub variant: Variant,
    pub on_click: ClickHandler,
}

impl DialogAction {
    pub fn new(
        label: impl Into<SharedString>,
        variant: Variant,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            variant,
            on_click: Box::new(on_click),
        }
    }
}

/// `shared-ui/Dialog.svelte`: a modal styled as a spawned popup window -- the one bold motif in
/// the design. A slim titlebar with a carmine dot and a mono title, sharp corners, a hard offset
/// shadow, and one faded echo frame behind it, as though an earlier popup were still there.
///
/// Drawn over the whole window. `on_close` runs for the backdrop, the close button and Escape;
/// Escape needs the caller to focus `focus` when the dialog opens.
pub fn dialog(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    actions: Vec<DialogAction>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    focus: &gpui_kit::FocusHandle,
    window: &mut Window,
) -> impl IntoElement {
    let on_close = std::rc::Rc::new(on_close);
    let viewport = window.viewport_size();
    let backdrop_close = on_close.clone();
    let button_close = on_close.clone();
    let key_close = on_close.clone();
    let panel_width = px(400.).min(viewport.width - px(48.));

    let panel = div()
        .id("dialog-panel")
        .relative()
        .w(panel_width)
        .flex()
        .flex_col()
        .rounded(theme::RADIUS_MD)
        .border_1()
        .border_color(c(theme::BORDER_STRONG))
        .bg(c(theme::SURFACE))
        .shadow(vec![gpui_kit::BoxShadow {
            color: theme::shadow_pop(),
            offset: gpui_kit::point(px(6.), px(6.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: false,
        }])
        // Clicks inside the panel must not reach the backdrop.
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
        .child(
            div()
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(c(theme::BORDER))
                .bg(c(theme::SURFACE_RAISED))
                .rounded_t(theme::RADIUS_MD)
                .child(
                    div()
                        .size(px(8.))
                        .flex_none()
                        .rounded_full()
                        .bg(c(theme::ACCENT)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(11.5))
                        .font_weight(FontWeight::BOLD)
                        .text_color(c(theme::TEXT))
                        .child(title.into()),
                )
                .child(
                    div()
                        .id("dialog-close")
                        .size(px(24.))
                        .mr(px(-4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS_SM)
                        .text_color(c(theme::MUTED))
                        .cursor_pointer()
                        .hover(|style| style.bg(c(theme::SURFACE)).text_color(c(theme::TEXT)))
                        .child(gpui_kit::assets::IconName::Close)
                        .text_size(px(12.))
                        .on_click(move |_, window, cx| button_close(window, cx)),
                ),
        )
        .child(
            div()
                .p_4()
                .flex()
                .flex_col()
                .child(hint(description).text_size(px(13.)).mb(px(18.)))
                .child(div().flex().justify_end().gap_2().children(
                    actions.into_iter().enumerate().map(|(index, action)| {
                        let on_click = action.on_click;
                        Button::new(("dialog-action", index), action.label)
                            .variant(action.variant)
                            .size(Size::Compact)
                            .on_click(move |event, window, cx| on_click(event, window, cx))
                    }),
                )),
        );

    gpui_kit::deferred(
        gpui_kit::anchored()
            .position(gpui_kit::point(px(0.), px(0.)))
            .child(
                div()
                    .id("dialog-backdrop")
                    .track_focus(focus)
                    .occlude()
                    .w(viewport.width)
                    .h(viewport.height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(gpui_kit::rgba(0x0000009e))
                    .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                        backdrop_close(window, cx)
                    })
                    .on_key_down(move |event, window, cx| {
                        if event.keystroke.key == "escape" {
                            key_close(window, cx);
                        }
                    })
                    .child(
                        div()
                            .relative()
                            // The echo frame: one earlier spawn, behind the live window.
                            .child(
                                div()
                                    .absolute()
                                    .top(px(-10.))
                                    .left(px(-10.))
                                    .right(px(10.))
                                    .bottom(px(10.))
                                    .rounded(theme::RADIUS_MD)
                                    .border_1()
                                    .border_color(c(theme::BORDER))
                                    .bg(gpui_kit::rgba(0x0a080966)),
                            )
                            .child(panel),
                    ),
            ),
    )
    .with_priority(2)
}
