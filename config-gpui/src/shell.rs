//! The window's root view: sidebar navigation, the session control in its footer, and the active
//! page. Ported from `config/src/routes/+page.svelte` and `config/src/lib/SessionControl.svelte`.

use gpui_kit::prelude::*;
use gpui_kit::{
    div, point, px, AnyElement, BoxShadow, Context, Entity, FontWeight, SharedString, Window,
};

use crate::pages::{
    audio::AudioPage, diagnostics::DiagnosticsPage, monitors::MonitorsPage,
    pack_mode::PackModePage, safety::SafetyPage, scheduling::SchedulingPage,
    window_style::WindowStylePage,
};
use crate::store::{EngineAction, FeedbackKind, Store, Tab};
use crate::theme::{self, c};
use crate::ui::{self, Button, NavItem, Size, Variant};

pub struct Shell {
    store: Entity<Store>,
    safety: Option<Entity<SafetyPage>>,
    audio: Option<Entity<AudioPage>>,
    window_style: Option<Entity<WindowStylePage>>,
    monitors: Option<Entity<MonitorsPage>>,
    diagnostics: Option<Entity<DiagnosticsPage>>,
    pack_mode: Option<Entity<PackModePage>>,
    scheduling: Option<Entity<SchedulingPage>>,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = cx.new(Store::new);
        cx.observe(&store, |_, _, cx| cx.notify()).detach();
        Self {
            store,
            safety: None,
            audio: None,
            window_style: None,
            monitors: None,
            diagnostics: None,
            pack_mode: None,
            scheduling: None,
        }
    }

    /// The active page's view, created the first time its tab is shown.
    fn page(
        &mut self,
        tab: Tab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let store = self.store.clone();
        Some(match tab {
            Tab::Safety => self
                .safety
                .get_or_insert_with(|| cx.new(|cx| SafetyPage::new(store, window, cx)))
                .clone()
                .into_any_element(),
            Tab::Audio => self
                .audio
                .get_or_insert_with(|| cx.new(|cx| AudioPage::new(store, window, cx)))
                .clone()
                .into_any_element(),
            Tab::WindowStyle => self
                .window_style
                .get_or_insert_with(|| cx.new(|cx| WindowStylePage::new(store, window, cx)))
                .clone()
                .into_any_element(),
            Tab::Monitors => self
                .monitors
                .get_or_insert_with(|| cx.new(|cx| MonitorsPage::new(store, window, cx)))
                .clone()
                .into_any_element(),
            Tab::Diagnostics => self
                .diagnostics
                .get_or_insert_with(|| cx.new(|cx| DiagnosticsPage::new(store, window, cx)))
                .clone()
                .into_any_element(),
            Tab::PackMode => self
                .pack_mode
                .get_or_insert_with(|| cx.new(|cx| PackModePage::new(store, window, cx)))
                .clone()
                .into_any_element(),
            Tab::Scheduling => self
                .scheduling
                .get_or_insert_with(|| cx.new(|cx| SchedulingPage::new(store, window, cx)))
                .clone()
                .into_any_element(),
        })
    }

    fn nav(&self, tabs: &[Tab], active: Tab) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(tabs.iter().map(|&tab| {
                let store = self.store.clone();
                NavItem::new(
                    SharedString::from(format!("nav-{tab:?}")),
                    tab.label(),
                    tab == active,
                )
                .on_click(move |_, _, cx| store.update(cx, |store, cx| store.set_tab(tab, cx)))
            }))
    }

    /// The one control that outlives every page: run state, a summary of what will launch (each
    /// value links back to its page), and Launch/Stop. Raised surface and strong border so it
    /// detaches from the sidebar; no pop shadow, which stays exclusive to dialogs.
    fn session_control(&self, store: &Store) -> impl IntoElement {
        let running = store.engine.running;
        let busy = store.engine_action.is_some();
        let pack_name = store.pack_name();
        let stop_key = store
            .config
            .as_ref()
            .map(|config| config.stop_button.to_string().replace(" + ", "+"));

        let field = |label: &'static str, value: String| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w_0()
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(c(theme::MUTED))
                        .child(label),
                )
                .child(
                    div()
                        .font_family(theme::FONT_MONO)
                        .text_size(px(11.))
                        .text_color(c(theme::TEXT))
                        .truncate()
                        .child(value),
                )
        };
        let link = |id: &'static str, tab: Tab| {
            let entity = self.store.clone();
            div()
                .id(id)
                .flex()
                .flex_col()
                .gap(px(6.))
                .min_w_0()
                .mx(px(-4.))
                .px_1()
                .py(px(2.))
                .rounded(theme::RADIUS_SM)
                .cursor_pointer()
                .hover(|style| style.bg(c(theme::SURFACE)))
                .on_click(move |_, _, cx| entity.update(cx, |store, cx| store.set_tab(tab, cx)))
        };

        let action = if running {
            let entity = self.store.clone();
            Button::new("stop", "Stop")
                .variant(Variant::Destructive)
                .loading(store.engine_action == Some(EngineAction::Stop))
                .disabled(busy && store.engine_action != Some(EngineAction::Stop))
                .on_click(move |_, _, cx| entity.update(cx, |store, cx| store.stop(cx)))
        } else {
            let entity = self.store.clone();
            Button::new("launch", "Launch")
                .variant(Variant::Primary)
                .disabled(pack_name.is_none())
                .loading(store.engine_action == Some(EngineAction::Launch))
                .on_click(move |_, _, cx| entity.update(cx, |store, cx| store.launch(cx)))
        }
        .size(Size::Compact)
        .full_width();

        div()
            .mx_3()
            .mb_3()
            .mt_auto()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(theme::RADIUS_MD)
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .bg(c(theme::SURFACE_RAISED))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .bg(c(if running {
                                theme::ACCENT
                            } else {
                                theme::BORDER_STRONG
                            }))
                            .when(busy, |this| this.opacity(0.5)),
                    )
                    .child(ui::readout(if running { "running" } else { "stopped" })),
            )
            .child(match pack_name {
                None => link("summary", Tab::PackMode)
                    .child(ui::readout("select a pack first"))
                    .into_any_element(),
                Some(pack) => link("summary", Tab::PackMode)
                    .child(field("Pack", pack))
                    .child(field(
                        "Mode",
                        store.mode_name().unwrap_or_else(|| "unknown".into()),
                    ))
                    .into_any_element(),
            })
            .when_some(stop_key, |this, key| {
                this.child(link("stop-key", Tab::Safety).child(field("Stop shortcut", key)))
            })
            .child(action)
    }

    fn sidebar(&self, store: &Store) -> impl IntoElement {
        div()
            .w(px(192.))
            .flex_none()
            .flex()
            .flex_col()
            .bg(c(theme::SURFACE))
            .border_r_1()
            .border_color(c(theme::BORDER))
            .child(
                div()
                    .h(px(64.))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .px_4()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Lewdware"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(theme::MUTED))
                            .child("Settings"),
                    ),
            )
            .child(
                div()
                    .p_3()
                    .flex()
                    .flex_col()
                    .child(self.nav(&Tab::SETTINGS, store.active_tab))
                    .child(
                        div()
                            .mt_3()
                            .pt_3()
                            .border_t_1()
                            .border_color(c(theme::BORDER))
                            .child(self.nav(&Tab::UTILITY, store.active_tab)),
                    ),
            )
            .when(store.ready(), |this| {
                this.child(self.session_control(store))
            })
    }

    fn main(&self, store: &Store, page: Option<AnyElement>) -> AnyElement {
        let centered = || {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .p_8()
        };

        if let Some(error) = &store.load_error {
            let entity = self.store.clone();
            return centered()
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Settings couldn't be loaded"),
                )
                .child(
                    div()
                        .max_w(px(448.))
                        .text_size(px(12.))
                        .text_color(c(theme::MUTED))
                        .child(error.clone()),
                )
                // The sidebar footer isn't shown here, so this is the surface's one primary.
                .child(
                    Button::new("retry", "Try again")
                        .variant(Variant::Primary)
                        .on_click(move |_, _, cx| entity.update(cx, |store, cx| store.load(cx))),
                )
                .into_any_element();
        }

        if !store.ready() {
            return centered()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(c(theme::MUTED))
                        .child("Loading settings..."),
                )
                .into_any_element();
        }

        page.unwrap_or_else(|| div().into_any_element())
    }

    /// `shared-ui/UpdateBanner.svelte`: a thin carmine-tinted strip across the top of the window.
    fn update_banner(&self, store: &Store) -> Option<impl IntoElement> {
        let url = store.update_url.clone()?;
        let dismiss = self.store.clone();
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(c(theme::ACCENT).opacity(0.3))
                .bg(c(theme::ACCENT).opacity(0.1))
                .text_size(px(14.))
                .child(
                    div()
                        .flex_1()
                        .child("A new version of Lewdware is available."),
                )
                .child(
                    div()
                        .id("download-update")
                        .font_weight(FontWeight::MEDIUM)
                        .underline()
                        .cursor_pointer()
                        .hover(|style| style.text_color(gpui_kit::rgb(0xff668f)))
                        .child("Download update")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .child(
                    div()
                        .id("dismiss-update")
                        .size(px(24.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(0.6)
                        .cursor_pointer()
                        .hover(|style| style.opacity(1.))
                        .child(gpui_kit::assets::IconName::Close)
                        .on_click(move |_, _, cx| {
                            dismiss.update(cx, |store, cx| store.dismiss_update(cx))
                        }),
                ),
        )
    }

    /// `TaskStatus.svelte`: the one transient message, pinned bottom-right.
    fn task_status(&self, store: &Store) -> Option<impl IntoElement> {
        let (feedback, queued) = store.current_feedback()?;
        let task = feedback.task;
        let tone = match feedback.kind {
            FeedbackKind::Progress | FeedbackKind::Confirm => c(theme::MUTED),
            FeedbackKind::Warning => c(theme::WARNING),
            FeedbackKind::Error => c(theme::DANGER),
        };
        let entity = self.store.clone();

        Some(
            div()
                .absolute()
                .right(px(18.))
                .bottom(px(18.))
                .max_w(px(440.))
                .min_h(px(34.))
                .px(px(10.))
                .py(px(7.))
                .flex()
                .items_center()
                .gap(px(7.))
                .rounded(theme::RADIUS_MD)
                .border_1()
                .border_color(c(theme::BORDER))
                .bg(c(theme::SURFACE_RAISED))
                .shadow(vec![BoxShadow {
                    color: theme::shadow_pop(),
                    offset: point(px(6.), px(6.)),
                    blur_radius: px(0.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .font_family(theme::FONT_MONO)
                .text_size(px(11.5))
                .text_color(tone)
                .child(div().truncate().child(feedback.message.clone()))
                .when(queued > 0, |this| {
                    this.child(
                        div()
                            .text_color(c(theme::MUTED))
                            .child(format!("+{queued}")),
                    )
                })
                .when(
                    matches!(feedback.kind, FeedbackKind::Warning | FeedbackKind::Error),
                    |this| {
                        this.child(
                            div()
                                .id("dismiss-status")
                                .px_1()
                                .text_color(c(theme::MUTED))
                                .cursor_pointer()
                                .hover(|style| style.text_color(c(theme::TEXT)))
                                .child("✕")
                                .on_click(move |_, _, cx| {
                                    entity.update(cx, |store, cx| store.dismiss_feedback(task, cx))
                                }),
                        )
                    },
                ),
        )
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.store.read(cx).active_tab;
        let page = if self.store.read(cx).ready() {
            self.page(tab, window, cx)
        } else {
            None
        };
        let store = self.store.read(cx);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(c(theme::BG))
            .text_color(c(theme::TEXT))
            .font_family(theme::FONT_SANS)
            .text_size(px(13.))
            .children(self.update_banner(store))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.sidebar(store))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .child(self.main(store, page)),
                    ),
            )
            .children(self.task_status(store))
    }
}
