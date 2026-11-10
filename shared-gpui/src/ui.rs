//! Small components that carry the design guide's rules (`shared-ui/DESIGN.md`), ported from
//! `shared-ui/*.svelte`. gpui-component is still used for the heavier widgets (inputs, selects,
//! scrolling); these are the pieces whose look is specific enough to be worth owning.

use gpui_kit::assets::IconName;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, App, ClickEvent, ElementId, FontWeight, Pixels, SharedString, Window};

use crate::icons::icon;
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
    icon: Option<IconName>,
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
            icon: None,
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

    /// An icon before the label.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
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
            .gap(px(6.))
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
            .children(self.icon.map(|name| icon(name).size(px(14.))))
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

/// A square, borderless button holding one icon, named by a tooltip (the Svelte `IconButton`).
/// Muted until hovered, like a quiet button.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: IconName,
    tooltip: Option<SharedString>,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName, tooltip: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon,
            tooltip: Some(tooltip.into()),
            disabled: false,
            on_click: None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// No tooltip: what it opens (a menu) is showing.
    pub fn without_tooltip(mut self) -> Self {
        self.tooltip = None;
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

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let tooltip = self.tooltip;
        div()
            .id(self.id)
            .flex_none()
            .size(theme::CONTROL_COMPACT)
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_MD)
            .text_color(c(theme::MUTED))
            .child(icon(self.icon).size(px(16.)))
            .when_some(tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            })
            .when(self.disabled, |this| this.opacity(0.45))
            .when(!self.disabled, |this| {
                this.cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(c(theme::SURFACE_RAISED))
                            .text_color(c(theme::TEXT))
                    })
                    .when_some(self.on_click, |this, handler| {
                        this.on_click(move |event, window, cx| handler(event, window, cx))
                    })
            })
    }
}

/// One entry in a vertical nav. Active is a raised, filled row with white text; the rest are
/// muted until hovered. (The 2px carmine edge was dropped from the nav in November 2026; see
/// `shared-ui/DESIGN.md`.)
#[derive(IntoElement)]
pub struct NavItem {
    id: ElementId,
    label: SharedString,
    icon: Option<IconName>,
    active: bool,
    on_click: Option<ClickHandler>,
}

impl NavItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            active,
            on_click: None,
        }
    }

    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
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

impl RenderOnce for NavItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.))
            .px_2()
            .py(px(7.))
            .rounded(theme::RADIUS_MD)
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .map(|this| {
                if self.active {
                    this.bg(c(theme::SURFACE_RAISED)).text_color(c(theme::TEXT))
                } else {
                    this.text_color(c(theme::MUTED))
                }
            })
            // On the active row too, where it changes nothing: gpui only tracks hover on an
            // element with a hover style, so a row clicked while hovered would otherwise come
            // back from being active still drawn hovered, until the pointer next moved.
            .hover(|style| style.text_color(c(theme::TEXT)))
            .children(self.icon.map(|name| icon(name).size(px(15.))))
            .child(div().min_w_0().truncate().child(self.label))
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
/// A scrolling page of sections with no title over them: the pack editor's tabs, whose names
/// the sidebar already shows.
pub fn page_body(
    id: &'static str,
    sections: impl IntoIterator<Item = gpui_kit::AnyElement>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .child(
            div()
                .mx_auto()
                .w_full()
                .max_w(px(896.))
                .p_8()
                .flex()
                .flex_col()
                .gap_6()
                .children(sections),
        )
}

pub fn page(
    id: &'static str,
    title: &'static str,
    lede: &'static str,
    sections: impl IntoIterator<Item = gpui_kit::AnyElement>,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .child(
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
                        .when(!lede.is_empty(), |this| {
                            this.child(hint(lede).text_size(px(13.)))
                        }),
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
        .gap_1()
        .children(options.into_iter().enumerate().map(move |(index, option)| {
            let checked = &option.value == selected;
            let on_change = on_change.clone();
            let value = option.value.clone();
            div()
                .id((id, index))
                .w_full()
                .min_h(theme::CONTROL_NORMAL)
                .px_3()
                .py_2()
                .flex()
                .items_center()
                .gap_3()
                .rounded(theme::RADIUS_MD)
                .when(checked, |this| this.bg(c(theme::SURFACE_RAISED)))
                .child(
                    div()
                        .size(px(16.))
                        .flex_none()
                        .rounded_full()
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
                        .text_color(gpui_kit::white())
                        .when(checked, |this| {
                            this.child(icon(IconName::Check).size(px(10.)))
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .when(checked, |this| this.font_weight(FontWeight::MEDIUM))
                                .child(option.label),
                        )
                        .when_some(option.description, |this, description| {
                            this.child(hint(description))
                        }),
                )
                .when(option.disabled, |this| this.opacity(0.45))
                .when(!option.disabled && !checked, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(c(theme::SURFACE_RAISED)))
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
                div().flex_none().text_color(c(theme::MUTED)).child(
                    icon(if open {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .size(px(14.)),
                ),
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
            // Positioned and framed by an outer box, scrolled by an inner one: the scrollbar
            // wrapper keeps the caller's size but not its position, so on a single element the
            // menu would lose `absolute` and collapse into the trigger's flow.
            let list = div()
                .id("menu-list")
                .max_h(px(280.))
                .p_1()
                .overflow_y_scrollbar();
            let menu = gpui_kit::deferred(
                div()
                    .id("menu")
                    .occlude()
                    .absolute()
                    .top(theme::CONTROL_NORMAL + px(4.))
                    .left_0()
                    .right_0()
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
                    .child(list.children(self.options.into_iter().enumerate().map(
                        |(index, option)| {
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
                        },
                    ))),
            )
            .with_priority(POPOVER_PRIORITY);
            let backdrop = dismiss_layer("select-backdrop", window, move |_, cx| {
                close_state.update(cx, |open, cx| {
                    *open = false;
                    cx.notify();
                })
            });
            [backdrop.into_any_element(), menu.into_any_element()]
        });

        div()
            .id(self.id)
            .relative()
            .flex_1()
            .min_w_0()
            .child(trigger)
            .children(menu.into_iter().flatten())
    }
}

/// Redraws the view when `input` gains or loses focus, so a field box around it can light its
/// border: focusing an input redraws the input, not the view that draws the box. gpui's own focus
/// events, which (unlike gpui-component's `InputEvent::Blur`) always arrive.
pub fn track_focus_of<T: 'static, E: gpui_kit::Focusable>(
    input: &gpui_kit::Entity<E>,
    window: &mut Window,
    cx: &mut gpui_kit::Context<T>,
) -> [gpui_kit::Subscription; 2] {
    let handle = input.read(cx).focus_handle(cx);
    [
        cx.on_focus(&handle, window, |_, _, cx| cx.notify()),
        cx.on_blur(&handle, window, |_, _, cx| cx.notify()),
    ]
}

/// Keeps the view's `slot` naming `input` while it has focus, and redraws the view on each change,
/// so a field box drawn around it can light its border without needing the `Window` at render
/// time. Blur only clears the slot if it still names this input, so the order the old field's
/// blur and the new one's focus arrive in doesn't matter.
pub fn track_focused_input<T: 'static, E: gpui_kit::Focusable>(
    input: &gpui_kit::Entity<E>,
    slot: fn(&mut T) -> &mut Option<gpui_kit::EntityId>,
    window: &mut Window,
    cx: &mut gpui_kit::Context<T>,
) -> [gpui_kit::Subscription; 2] {
    let handle = input.read(cx).focus_handle(cx);
    let id = input.entity_id();
    [
        cx.on_focus(&handle, window, move |this, _, cx| {
            *slot(this) = Some(id);
            cx.notify();
        }),
        cx.on_blur(&handle, window, move |this, _, cx| {
            let focused = slot(this);
            if *focused == Some(id) {
                *focused = None;
            }
            cx.notify();
        }),
    ]
}

/// The border of a box drawn around a text input: lit while the input has focus.
pub fn field_border(focused: bool) -> gpui_kit::Hsla {
    c(if focused {
        theme::FOCUS
    } else {
        theme::BORDER_STRONG
    })
}

/// A search box: a magnifier, then the input, the border lit while it has focus. Size it (`w`)
/// at the call site.
pub fn search_field(
    input: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
    focused: bool,
) -> gpui_kit::Div {
    div()
        .h(theme::CONTROL_COMPACT)
        .pl_2()
        .flex()
        .items_center()
        .rounded(theme::RADIUS_MD)
        .border_1()
        .border_color(c(if focused {
            theme::FOCUS
        } else {
            theme::BORDER_STRONG
        }))
        .bg(c(theme::BG))
        .child(
            div()
                .flex_none()
                .text_color(c(theme::MUTED))
                .child(icon(IconName::Search).size(px(14.))),
        )
        .child(
            gpui_kit::component::input::Input::new(input)
                .appearance(false)
                .text_size(px(12.))
                .flex_1(),
        )
}

/// The horizontal padding gpui-component puts inside an input even with its frame off. A box
/// drawn around an input adds none of its own, so the text sits where a label above it starts
/// plus this.
pub const INPUT_PAD: gpui_kit::Pixels = px(10.);

/// Deferred priorities for floating layers. Popovers go above the media viewer (which is drawn at
/// 2), so a dropdown opened from the viewer's rail isn't hidden behind it.
pub const POPOVER_BACKDROP_PRIORITY: usize = 10;
pub const POPOVER_PRIORITY: usize = 11;
/// Dialogs are modal: above everything, an open popover included.
pub const DIALOG_PRIORITY: usize = 20;

/// The box a popover's content sits in: raised, bordered, with the hard offset shadow. Takes
/// presses itself, so a click in it doesn't fall through to the [`dismiss_layer`] under it.
pub fn popover_surface() -> gpui_kit::Div {
    div()
        .occlude()
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
}

/// Floats `content` over the page, `top` below the top of the element this is a child of (which
/// must be `.relative()`), lined up with its left edge, or its right with `align_right`. It takes
/// no room in the layout and draws above everything but dialogs. Where there's no room for it
/// below, it opens above the element instead, and it's kept inside the window either way.
pub fn floating(
    top: gpui_kit::Pixels,
    align_right: bool,
    content: impl IntoElement,
) -> gpui_kit::Div {
    // The gap between the element and the layer, as padding: the layer is anchored at the
    // element's top and pushed down, so flipping it puts its bottom at the element's top, and
    // the padding keeps the same gap there. It's not part of the surface, so a press in it
    // still lands on the dismiss layer.
    const GAP: f32 = 4.;
    let layer = gpui_kit::anchored()
        .anchor(if align_right {
            gpui_kit::Anchor::TopRight
        } else {
            gpui_kit::Anchor::TopLeft
        })
        .offset(gpui_kit::point(px(0.), top - px(GAP)))
        .child(div().py(px(GAP)).child(content));
    div()
        .absolute()
        .top_0()
        .map(|this| {
            if align_right {
                this.right_0()
            } else {
                this.left_0()
            }
        })
        .child(gpui_kit::deferred(layer).with_priority(POPOVER_PRIORITY))
}

/// An invisible layer over the whole window, under an open popover: a press anywhere else lands
/// on it and closes the popover. Including the popover's own trigger, so a second click on the
/// trigger closes it rather than closing it on press and reopening it on release.
pub fn dismiss_layer(
    id: &'static str,
    window: &Window,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let viewport = window.viewport_size();
    gpui_kit::deferred(
        gpui_kit::anchored()
            .position(gpui_kit::point(px(0.), px(0.)))
            .child(
                div()
                    .id(id)
                    .block_mouse_except_scroll()
                    .w(viewport.width)
                    .h(viewport.height)
                    .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                        on_dismiss(window, cx)
                    }),
            ),
    )
    .with_priority(POPOVER_BACKDROP_PRIORITY)
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
        .text_color(c(theme::TEXT))
        .when(checked, |this| {
            this.child(icon(IconName::Check).size(px(12.)))
        })
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
    let body =
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
            ));
    dialog_window(title, px(400.), body, on_close, focus, window)
}

/// The width a [`dialog_window`] of `width` actually gets in this window: it keeps a margin.
pub fn dialog_width(width: Pixels, window: &Window) -> Pixels {
    width.min(window.viewport_size().width - px(48.))
}

/// The spawned-popup frame of [`dialog`] around any body: titlebar, hard shadow, echo frame,
/// backdrop. `width` is the most it takes; see [`dialog_width`].
pub fn dialog_window(
    title: impl Into<SharedString>,
    width: Pixels,
    body: impl IntoElement,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    focus: &gpui_kit::FocusHandle,
    window: &mut Window,
) -> impl IntoElement {
    let on_close = std::rc::Rc::new(on_close);
    let viewport = window.viewport_size();
    let backdrop_close = on_close.clone();
    let button_close = on_close.clone();
    let key_close = on_close.clone();
    let panel_width = dialog_width(width, window);

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
        .child(body);

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
    .with_priority(DIALOG_PRIORITY)
}

// ── Resizable sidebars ──────────────────────────────────────────────────────────────────────

/// Which side of the window's content a [`resizable_sidebar`] sits on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Every sidebar's dragged width, by id, so it survives the view being dropped and made again.
#[derive(Default)]
struct SidebarWidths(std::collections::HashMap<&'static str, Pixels>);

impl gpui_kit::Global for SidebarWidths {}

/// The payload of a sidebar's resize drag. It draws nothing: the sidebar itself follows it.
#[derive(Clone)]
struct ResizeSidebar(&'static str);

impl gpui_kit::Render for ResizeSidebar {
    fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        gpui_kit::Empty
    }
}

/// A sidebar `default` wide whose inner edge drags between `min` and `max` (double-click resets
/// it). `content` fills it; the edge draws the dividing hairline, so `content` needs no border
/// on that side. The width is remembered by `id` for as long as the app runs.
pub fn resizable_sidebar(
    id: &'static str,
    side: Side,
    default: Pixels,
    (min, max): (Pixels, Pixels),
    content: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    use gpui_kit::component::resizable::{resize_handle, HandleEdge};
    use gpui_kit::component::InteractiveElementExt as _;

    let width = cx
        .try_global::<SidebarWidths>()
        .and_then(|widths| widths.0.get(id).copied())
        .unwrap_or(default)
        .clamp(min, max);
    let edge = match side {
        Side::Left => HandleEdge::Trailing,
        Side::Right => HandleEdge::Leading,
    };
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(width)
        .h_full()
        .flex()
        .child(div().flex_1().min_w_0().h_full().flex().child(content))
        .child(
            resize_handle::<ResizeSidebar, ResizeSidebar>(
                SharedString::from(format!("{id}-edge")),
                gpui_kit::Axis::Horizontal,
            )
            .inside(edge)
            .on_drag(ResizeSidebar(id), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_double_click(move |_, window, cx| {
                cx.default_global::<SidebarWidths>().0.remove(id);
                window.refresh();
            }),
        )
        .on_drag_move::<ResizeSidebar>(move |event, window, cx| {
            if event.drag(cx).0 != id {
                return;
            }
            let x = event.event.position.x;
            let width = match side {
                Side::Left => x - event.bounds.left(),
                Side::Right => event.bounds.right() - x,
            };
            cx.default_global::<SidebarWidths>()
                .0
                .insert(id, width.clamp(min, max));
            window.refresh();
        })
}
