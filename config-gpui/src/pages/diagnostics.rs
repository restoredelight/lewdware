//! Recent activity, and what's needed to investigate a problem. Ported from
//! `config/src/lib/Diagnostics.svelte`.

use std::time::Duration;

use chrono::Local;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, AnyElement, ClipboardItem, Context, Entity, ScrollEvent, ScrollHandle, Subscription,
    Task, Window,
};
use shared::logging::{LogLevel, LogRecord};

use crate::backend::diagnostics::{self, format_record, source_label, Diagnostics};
use crate::runtime;
use crate::store::{FeedbackKind, Store};
use crate::theme::{self, c};
use crate::ui::{self, Button, SelectOption, Size, Toggle};

/// The viewer shows the newest matching entries only. Keeping the element tree bounded matters
/// more than having every row on screen; "Copy diagnostics" still includes everything read.
const SHOWN: usize = 500;
/// While following, logs are re-read this often.
const REFRESH: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq, Eq)]
enum LevelFilter {
    All,
    Error,
    Warn,
    Info,
}

pub struct DiagnosticsPage {
    store: Entity<Store>,
    diagnostics: Option<Diagnostics>,
    loading: bool,
    load_error: Option<String>,
    /// `None` is every component.
    source: Option<String>,
    level: LevelFilter,
    search: Entity<InputState>,
    /// Set while the search field has focus, for its border.
    focused_input: Option<gpui_kit::EntityId>,
    /// Keep the newest entry in view. Scrolling up turns it off; scrolling back to the bottom
    /// turns it on again.
    follow: bool,
    scroll: ScrollHandle,
    _refresh: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl DiagnosticsPage {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search logs"));
        let mut subscriptions = vec![cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })];
        subscriptions.extend(ui::track_focused_input(
            &search,
            |this: &mut Self| &mut this.focused_input,
            window,
            cx,
        ));
        let refresh = cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(REFRESH).await;
            let Ok(follow) = this.update(cx, |this, _| this.follow) else {
                return;
            };
            if follow {
                let _ = this.update(cx, |this, cx| this.refresh(true, cx));
            }
        });

        let mut page = Self {
            store,
            diagnostics: None,
            loading: false,
            load_error: None,
            source: None,
            level: LevelFilter::All,
            search,
            focused_input: None,
            follow: true,
            scroll: ScrollHandle::new(),
            _refresh: refresh,
            _subscriptions: subscriptions,
        };
        page.refresh(false, cx);
        page
    }

    /// `silent` is the background refresh: no busy state, and a failure keeps what's shown.
    fn refresh(&mut self, silent: bool, cx: &mut Context<Self>) {
        if !silent {
            self.loading = true;
            cx.notify();
        }
        cx.spawn(async move |this, cx| {
            let result = runtime::blocking(diagnostics::read).await;
            let _ = this.update(cx, |this, cx| {
                if !silent {
                    this.loading = false;
                }
                match result {
                    Ok(diagnostics) => {
                        this.diagnostics = Some(diagnostics);
                        this.load_error = None;
                        if this.follow {
                            this.scroll.scroll_to_bottom();
                        }
                    }
                    Err(error) if !silent => this.load_error = Some(error.to_string()),
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn visible<'a>(&'a self, cx: &Context<Self>) -> Vec<&'a LogRecord> {
        let needle = self.search.read(cx).value().trim().to_lowercase();
        self.diagnostics
            .as_ref()
            .map(|d| {
                d.logs
                    .iter()
                    .filter(|r| self.source.as_ref().is_none_or(|s| &r.program == s))
                    .filter(|r| match self.level {
                        LevelFilter::All => true,
                        LevelFilter::Error => r.level == LogLevel::Error,
                        LevelFilter::Warn => r.level == LogLevel::Warn,
                        LevelFilter::Info => r.level == LogLevel::Info,
                    })
                    .filter(|r| diagnostics::matches(r, &needle))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn copy(&mut self, text: String, confirmation: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let confirmation = confirmation.to_string();
        self.store
            .update(cx, |store, cx| store.confirm("clipboard", confirmation, cx));
    }

    fn open_logs(&mut self, cx: &mut Context<Self>) {
        match diagnostics::log_dir() {
            Ok(dir) => cx.open_with_system(&dir),
            Err(error) => self.store.update(cx, |store, cx| {
                store.set_feedback(
                    "logs",
                    FeedbackKind::Error,
                    format!("Couldn't open the logs folder: {error}"),
                );
                cx.notify();
            }),
        }
    }

    fn report(&self, cx: &Context<Self>) -> String {
        let Some(diagnostics) = &self.diagnostics else {
            return String::new();
        };
        let store = self.store.read(cx);
        let system = &diagnostics.system;
        let monitors = if store.monitors.is_empty() {
            "- Unavailable".to_string()
        } else {
            store
                .monitors
                .iter()
                .map(|m| {
                    // The area belongs in a bug report: "popups only appear in one corner" is
                    // indistinguishable from a placement bug without it.
                    let region = store.monitor_region(&m.id);
                    let area = if region.is_full() {
                        String::new()
                    } else {
                        format!(
                            " (area {}%×{}% at {}%,{}%)",
                            (region.width * 100.).round(),
                            (region.height * 100.).round(),
                            (region.x * 100.).round(),
                            (region.y * 100.).round()
                        )
                    };
                    format!(
                        "- {}: {}×{}{}{}{}",
                        m.name,
                        m.width,
                        m.height,
                        if m.primary { " (primary)" } else { "" },
                        if store.monitor_enabled(&m.id) {
                            ""
                        } else {
                            " (disabled)"
                        },
                        area
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let (theme_name, appearance) = store
            .config
            .as_ref()
            .map(|c| (c.theme.name(), c.appearance.name()))
            .unwrap_or(("unknown", "unknown"));

        let mut lines = vec![
            "### Lewdware diagnostics".to_string(),
            String::new(),
            format!("Generated: {}", chrono::Utc::now().to_rfc3339()),
            String::new(),
            "#### System".into(),
            String::new(),
            format!("- Lewdware: {}", system.lewdware_version),
            format!("- OS: {}", system.os),
            format!("- Architecture: {}", system.architecture),
            String::new(),
            "#### Session".into(),
            String::new(),
            format!(
                "- State: {}",
                if store.engine.running {
                    "Running"
                } else {
                    "Stopped"
                }
            ),
            format!(
                "- Pack: {}",
                store.pack_name().unwrap_or_else(|| "None".into())
            ),
            format!(
                "- Mode: {}",
                store.mode_name().unwrap_or_else(|| "Unknown".into())
            ),
            format!("- Window style: {theme_name} ({appearance})"),
            String::new(),
            "#### Monitors".into(),
            String::new(),
            monitors,
            String::new(),
            format!("#### Recent logs ({})", diagnostics.logs.len()),
            String::new(),
            "````text".into(),
        ];
        lines.extend(diagnostics.logs.iter().map(format_record));
        lines.push("````".into());
        lines.join("\n")
    }

    /// Scrolling up stops following; reaching the bottom again resumes it.
    fn on_scroll(&mut self, _: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.scroll.offset().y;
        let max = self.scroll.max_offset().y;
        let at_bottom = -offset >= max - px(8.);
        if at_bottom != self.follow {
            self.follow = at_bottom;
            cx.notify();
        }
    }

    fn system_info(&self) -> AnyElement {
        let value = |text: Option<&str>| text.unwrap_or("Loading...").to_string();
        let system = self.diagnostics.as_ref().map(|d| &d.system);
        let cell = |label: &'static str, text: String| {
            div()
                .flex_1()
                .min_w_0()
                .p_4()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(ui::hint(label))
                .child(
                    ui::readout(text)
                        .text_size(px(11.5))
                        .text_color(c(theme::TEXT))
                        .truncate(),
                )
        };
        ui::card()
            .flex_row()
            .child(cell(
                "Lewdware version",
                value(system.map(|s| s.lewdware_version.as_str())),
            ))
            .child(div().w(px(1.)).bg(c(theme::BORDER)))
            .child(cell(
                "Operating system",
                value(system.map(|s| s.os.as_str())),
            ))
            .child(div().w(px(1.)).bg(c(theme::BORDER)))
            .child(cell(
                "Architecture",
                value(system.map(|s| s.architecture.as_str())),
            ))
            .into_any_element()
    }

    fn log_row(record: &LogRecord) -> impl IntoElement {
        let level_color = match record.level {
            LogLevel::Error => c(theme::DANGER),
            LogLevel::Warn => c(theme::WARNING),
            _ => c(theme::MUTED),
        };
        div()
            .flex()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .font_family(theme::FONT_MONO)
            .text_size(px(11.))
            .child(
                div()
                    .w(px(88.))
                    .flex_none()
                    .text_color(c(theme::MUTED))
                    .child(
                        record
                            .timestamp
                            .with_timezone(&Local)
                            .format("%H:%M:%S")
                            .to_string(),
                    ),
            )
            .child(
                div()
                    .w(px(64.))
                    .flex_none()
                    .text_color(level_color)
                    .child(record.level.label().to_lowercase()),
            )
            .child(
                div()
                    .w(px(104.))
                    .flex_none()
                    .truncate()
                    .text_color(c(theme::MUTED))
                    .child(source_label(&record.program).to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(c(theme::TEXT))
                    .child(record.message.clone()),
            )
    }

    fn logs(&self, cx: &mut Context<Self>) -> AnyElement {
        let visible = self.visible(cx);
        let total = visible.len();
        let shown = &visible[total.saturating_sub(SHOWN)..];
        let copy_text = visible
            .iter()
            .map(|r| format_record(r))
            .collect::<Vec<_>>()
            .join("\n");

        let mut sources: Vec<String> = self
            .diagnostics
            .iter()
            .flat_map(|d| d.logs.iter().map(|r| r.program.clone()))
            .collect();
        sources.sort();
        sources.dedup();
        let mut source_options = vec![SelectOption::new(None, "All components")];
        source_options.extend(
            sources
                .into_iter()
                .map(|s| SelectOption::new(Some(s.clone()), source_label(&s).to_string())),
        );

        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(c(theme::BORDER))
            .child(div().w(px(170.)).flex().child(
                ui::Select::new("log-source", self.source.clone(), source_options).on_change({
                    let this = cx.entity();
                    move |source, _, cx| {
                        this.update(cx, |this, cx| {
                            this.source = source;
                            cx.notify();
                        })
                    }
                }),
            ))
            .child(
                div().w(px(140.)).flex().child(
                    ui::Select::new(
                        "log-level",
                        self.level,
                        vec![
                            SelectOption::new(LevelFilter::All, "All levels"),
                            SelectOption::new(LevelFilter::Error, "Errors"),
                            SelectOption::new(LevelFilter::Warn, "Warnings"),
                            SelectOption::new(LevelFilter::Info, "Information"),
                        ],
                    )
                    .on_change({
                        let this = cx.entity();
                        move |level, _, cx| {
                            this.update(cx, |this, cx| {
                                this.level = level;
                                cx.notify();
                            })
                        }
                    }),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(theme::CONTROL_NORMAL)
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(theme::RADIUS_SM)
                    .border_1()
                    .border_color(ui::field_border(self.focused_input.is_some()))
                    .bg(c(theme::BG))
                    .text_size(px(12.))
                    .child(
                        Input::new(&self.search)
                            .appearance(false)
                            .flex_1()
                            .text_size(px(12.)),
                    ),
            )
            .child(
                Button::new("copy-visible", "Copy")
                    .size(Size::Compact)
                    .disabled(total == 0)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.copy(copy_text.clone(), "Visible logs copied", cx)
                    })),
            );

        let body: AnyElement = if self.loading && self.diagnostics.is_none() {
            ui::hint("Loading logs...")
                .p_6()
                .flex()
                .justify_center()
                .into_any_element()
        } else if shown.is_empty() {
            ui::hint("No matching log entries.")
                .p_6()
                .flex()
                .justify_center()
                .into_any_element()
        } else {
            div()
                .children(shown.iter().map(|r| Self::log_row(r)))
                .into_any_element()
        };

        ui::card()
            .flex_1()
            .min_h(px(240.))
            .child(toolbar)
            .child(
                div()
                    .id("log-viewport")
                    .flex_1()
                    .min_h(px(112.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .bg(c(theme::BG))
                    .on_scroll(cx.listener(Self::on_scroll))
                    .child(body),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .child(
                        ui::readout(format!(
                            "Showing {} of {total} matching entries",
                            shown.len()
                        ))
                        .text_size(px(10.)),
                    )
                    .child(
                        ui::hint("Copied diagnostics may contain filenames or URLs. Review before sharing.")
                            .text_size(px(10.)),
                    ),
            )
            .into_any_element()
    }
}

impl Render for DiagnosticsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .items_start()
            .justify_between()
            .gap_6()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(px(672.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(ui::page_title("Diagnostics")),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("open-logs", "Open logs folder")
                            .on_click(cx.listener(|this, _, _, cx| this.open_logs(cx))),
                    )
                    .child(
                        Button::new("copy-diagnostics", "Copy diagnostics")
                            .disabled(self.diagnostics.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let report = this.report(cx);
                                this.copy(report, "Diagnostics copied", cx)
                            })),
                    ),
            );

        let content: Vec<AnyElement> = match (&self.load_error, &self.diagnostics) {
            (Some(error), None) => vec![div()
                .p_6()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .rounded(theme::RADIUS_MD)
                .border_1()
                .border_dashed()
                .border_color(c(theme::BORDER_STRONG))
                .child(ui::label("Diagnostics couldn't be loaded"))
                .child(ui::hint(error.clone()).max_w(px(448.)))
                .child(
                    Button::new("retry-diagnostics", "Try again")
                        .size(Size::Compact)
                        .loading(self.loading)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(false, cx))),
                )
                .into_any_element()],
            _ => vec![
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(ui::section_title("System information"))
                    .child(self.system_info())
                    .into_any_element(),
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pt_6()
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .justify_between()
                            .gap_4()
                            .child(ui::section_title("Recent logs"))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(Toggle::new("follow", self.follow).on_change({
                                        let this = cx.entity();
                                        move |checked, _, cx| {
                                            this.update(cx, |this, cx| {
                                                this.follow = checked;
                                                if checked {
                                                    this.scroll.scroll_to_bottom();
                                                }
                                                cx.notify();
                                            })
                                        }
                                    }))
                                    .child(ui::hint("Follow newest"))
                                    .child(
                                        Button::new("refresh-logs", "Refresh")
                                            .size(Size::Compact)
                                            .loading(self.loading)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.refresh(false, cx)
                                            })),
                                    ),
                            ),
                    )
                    .child(self.logs(cx))
                    .into_any_element(),
            ],
        };

        // Fills the window rather than scrolling as a page, so the log viewer gets the height;
        // below a minimum the page scrolls instead.
        div().id("diagnostics").flex_1().overflow_y_scroll().child(
            div()
                .mx_auto()
                .w_full()
                .max_w(px(1024.))
                .h_full()
                .min_h(px(576.))
                .p_8()
                .flex()
                .flex_col()
                .gap_6()
                .child(header)
                .children(content),
        )
    }
}
