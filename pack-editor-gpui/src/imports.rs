//! What the shell draws around imports: the import window (progress, stop, skips and failures),
//! the "drop to import" overlay, and the Edgeware import warnings. Ported from
//! `pack-editor/src/lib/UploadProgress.svelte`, `DropOverlay.svelte` and `ImportWarnings.svelte`.
//!
//! All three use the spawned-window motif (`shared-ui/DESIGN.md`): a slim titlebar with a carmine
//! dot and a mono title, a hard offset shadow, and one faded echo frame behind.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, App, FocusHandle, FontWeight, Pixels, Point, Render, SharedString, Window,
};
use pack_editor_core::session::lifecycle::{warning_kind_label, ConversionWarning, ImportProgress};

use crate::theme::{self, c};
use crate::ui::{Button, Size, Variant};

type Handler = Box<dyn Fn(&mut Window, &mut App) + 'static>;

/// The payload of a drag on the import window's titlebar: where in the titlebar it was grabbed,
/// filled in when the drag starts. The shell follows it (`Shell::render`). Draws nothing.
#[derive(Clone, Default)]
pub struct MoveImportWindow(pub Rc<Cell<Point<Pixels>>>, Rc<Cell<Point<Pixels>>>);

impl Render for MoveImportWindow {
    fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        gpui_kit::Empty
    }
}

/// A spawned window: titlebar (dot, mono title, then `controls`), a body, the echo frame behind.
/// With `drag`, the titlebar drags it.
pub fn popup_frame(
    title: impl Into<SharedString>,
    controls: Vec<AnyElement>,
    body: Option<AnyElement>,
    drag: Option<MoveImportWindow>,
) -> gpui_kit::Div {
    let has_body = body.is_some();
    let panel = div()
        .relative()
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
        .child(
            div()
                .id("popup-titlebar")
                .when_some(drag, |this, drag| {
                    let pressed = drag.1.clone();
                    // gpui measures the grab from where the pointer is once it has moved far
                    // enough to count as a drag; measured from the press, the window stays put
                    // under the pointer instead of jumping by that distance.
                    this.on_mouse_down(gpui_kit::MouseButton::Left, move |event, _, _| {
                        pressed.set(event.position)
                    })
                    .on_drag(drag, |drag, offset, window, cx| {
                        let origin = window.mouse_position() - offset;
                        drag.0.set(drag.1.get() - origin);
                        cx.new(|_| drag.clone())
                    })
                })
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap_2()
                .bg(c(theme::SURFACE_RAISED))
                .when(has_body, |this| {
                    this.border_b_1()
                        .border_color(c(theme::BORDER))
                        .rounded_t(theme::RADIUS_MD)
                })
                .when(!has_body, |this| this.rounded(theme::RADIUS_MD))
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
                .children(controls),
        )
        .children(body);
    div()
        .relative()
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
        .child(panel)
}

/// A small square titlebar button (minimise, close).
fn title_button(
    id: &'static str,
    icon: impl IntoElement,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(22.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_SM)
        .text_color(c(theme::MUTED))
        .text_size(px(11.))
        .cursor_pointer()
        .hover(|style| style.bg(c(theme::SURFACE)).text_color(c(theme::TEXT)))
        .child(icon)
        // Not a drag of the titlebar it sits in.
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation()
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx)
        })
        .into_any_element()
}

fn bar(fraction: f32, height: f32) -> gpui_kit::Div {
    div()
        .h(px(height))
        .w_full()
        .rounded_full()
        .overflow_hidden()
        .bg(c(theme::BORDER))
        .child(
            div()
                .h_full()
                .w(gpui_kit::relative(fraction.clamp(0., 1.)))
                .rounded_full()
                .bg(c(theme::ACCENT)),
        )
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// What the import window needs to draw itself, and what its controls do.
pub struct ImportWindow {
    pub progress: ImportProgress,
    /// Where its top left corner was dragged to, in the shell; `None` is the bottom right corner.
    pub position: Option<Point<Pixels>>,
    pub drag: MoveImportWindow,
    /// Resolving paths or waiting on the encoder: running, but nothing counted yet.
    pub preparing: bool,
    pub minimized: bool,
    pub errors_open: bool,
    /// Stop has been asked for and the pipeline is winding down.
    pub stopping: bool,
    pub on_toggle_minimized: Handler,
    pub on_toggle_errors: Handler,
    pub on_stop: Handler,
    pub on_dismiss: Handler,
}

impl ImportWindow {
    /// Whether there's anything to show: running, or finished with something to report.
    pub fn visible(progress: &ImportProgress, preparing: bool) -> bool {
        preparing || progress.running() || progress.has_report()
    }

    pub fn render(self) -> impl IntoElement {
        let Self {
            progress,
            position,
            drag,
            preparing,
            minimized,
            errors_open,
            stopping,
            on_toggle_minimized,
            on_toggle_errors,
            on_stop,
            on_dismiss,
        } = self;
        let running = preparing || progress.running();
        let percent = (progress.fraction() * 100.).round() as u32;

        let mut controls = Vec::new();
        if minimized {
            controls.push(
                div()
                    .flex_none()
                    .font_family(theme::FONT_MONO)
                    .text_size(px(11.))
                    .text_color(c(theme::MUTED))
                    .child(if running {
                        format!("{percent}%")
                    } else {
                        format!("{} done", progress.done)
                    })
                    .into_any_element(),
            );
        }
        controls.push(title_button(
            "import-minimize",
            if minimized {
                IconName::ChevronUp
            } else {
                IconName::ChevronDown
            },
            on_toggle_minimized,
        ));
        if !running {
            controls.push(title_button("import-dismiss", IconName::Close, on_dismiss));
        }

        let body =
            (!minimized).then(|| {
                let status = if running {
                    if progress.total == 0 {
                        "Starting...".to_string()
                    } else {
                        format!("{} / {} files", progress.done, progress.total)
                    }
                } else {
                    format!("{} processed", plural(progress.done, "file", "files"))
                };
                let errors = progress.errors.len();
                div()
                    .px(px(12.))
                    .pt(px(10.))
                    .pb(px(11.))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(running, |this| this.child(bar(progress.fraction(), 3.)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .font_family(theme::FONT_MONO)
                                    .text_size(px(11.))
                                    .text_color(c(theme::MUTED))
                                    .child(status),
                            )
                            .when(running, |this| {
                                this.child(
                                    Button::new(
                                        "import-stop",
                                        if stopping { "Stopping..." } else { "Stop" },
                                    )
                                    .variant(Variant::Destructive)
                                    .size(Size::Compact)
                                    .disabled(stopping)
                                    .on_click(move |_, window, cx| on_stop(window, cx)),
                                )
                            }),
                    )
                    .when(progress.skipped > 0, |this| {
                        this.child(div().text_size(px(11.)).text_color(c(theme::MUTED)).child(
                            format!(
                                "{} skipped",
                                plural(progress.skipped, "duplicate", "duplicates")
                            ),
                        ))
                    })
                    .when(errors > 0, |this| {
                        this.child(
                            div()
                                .id("import-errors-toggle")
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .font_family(theme::FONT_MONO)
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(c(theme::DANGER))
                                .cursor_pointer()
                                .child(plural(errors, "error", "errors"))
                                .child(div().size(px(10.)).child(if errors_open {
                                    IconName::ChevronUp
                                } else {
                                    IconName::ChevronDown
                                }))
                                .on_click(move |_, window, cx| on_toggle_errors(window, cx)),
                        )
                        .when(errors_open, |this| {
                            this.child(
                                div()
                                    .id("import-errors")
                                    .max_h(px(160.))
                                    .overflow_y_scrollbar()
                                    .flex()
                                    .flex_col()
                                    .gap(px(6.))
                                    .children(progress.errors.iter().map(|error| {
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap(px(1.))
                                            .text_size(px(11.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .font_family(theme::FONT_MONO)
                                                    .text_size(px(10.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(c(theme::MUTED))
                                                    .child(error.file_name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_color(c(theme::DANGER))
                                                    .child(error.error.clone()),
                                            )
                                    })),
                            )
                        })
                    })
                    .into_any_element()
            });
        let minimized_bar = (minimized && running).then(|| bar(progress.fraction(), 3.));

        div()
            .id("import-window")
            // Clicks on it stop here, not on whatever is under it.
            .occlude()
            .absolute()
            .map(|this| match position {
                Some(position) => this.left(position.x).top(position.y),
                None => this.right(px(16.)).bottom(px(16.)),
            })
            .when(!minimized, |this| this.w(px(320.)))
            .child(popup_frame("Import", controls, body, Some(drag)).children(minimized_bar))
    }
}

/// "Drop to import", over the whole window while files are dragged over it.
pub fn drop_overlay() -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui_kit::rgba(0x0000009e))
        .child(
            div().w(px(380.)).child(popup_frame(
                "Import",
                Vec::new(),
                Some(
                    div()
                        .px_4()
                        .py_5()
                        .text_size(px(13.))
                        .text_color(c(theme::MUTED))
                        .child("Drop files or folders to import them into this pack.")
                        .into_any_element(),
                ),
                None,
            )),
        )
}

/// What the Edgeware converter couldn't carry over, as a modal over the editor.
pub fn warnings_dialog(
    warnings: &[ConversionWarning],
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    focus: &FocusHandle,
    window: &mut Window,
) -> impl IntoElement {
    let on_close = std::rc::Rc::new(on_close);
    let viewport = window.viewport_size();
    let count = warnings.len();
    let description = format!(
        "{} of the Edgeware pack couldn't be converted. The rest of the pack is ready to edit.",
        plural(count, "part", "parts")
    );
    let (backdrop_close, key_close, button_close, ok_close) = (
        on_close.clone(),
        on_close.clone(),
        on_close.clone(),
        on_close,
    );
    let list = div()
        .id("warnings-list")
        .max_h(px((viewport.height.as_f32() - 260.).max(120.)))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_2()
        .children(warnings.iter().map(|warning| {
            div()
                .p_2()
                .rounded(theme::RADIUS_SM)
                .border_1()
                .border_color(c(theme::BORDER))
                .bg(c(theme::BG))
                .text_size(px(12.))
                .child(
                    div()
                        .mb(px(2.))
                        .font_family(theme::FONT_MONO)
                        .text_size(px(11.))
                        .text_color(c(theme::MUTED))
                        .child(warning_kind_label(warning.kind)),
                )
                .child(warning.message.clone())
        }));
    let body = div()
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_size(px(13.))
                .text_color(c(theme::MUTED))
                .child(description),
        )
        .child(list)
        .child(
            div().flex().justify_end().child(
                Button::new("warnings-ok", "Got it")
                    .variant(Variant::Primary)
                    .size(Size::Compact)
                    .on_click(move |_, window, cx| ok_close(window, cx)),
            ),
        )
        .into_any_element();
    let close = title_button("warnings-close", IconName::Close, move |w, cx| {
        button_close(w, cx)
    });

    gpui_kit::deferred(
        gpui_kit::anchored()
            .position(gpui_kit::point(px(0.), px(0.)))
            .child(
                div()
                    .id("warnings-backdrop")
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
                            .w(px(480.).min(viewport.width - px(48.)))
                            .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .child(popup_frame(
                                "Import warnings",
                                vec![close],
                                Some(body),
                                None,
                            )),
                    ),
            ),
    )
    .with_priority(2)
}
