//! How to stop a session, and what packs and modes may do outside their windows. Ported from
//! `config/src/lib/Safety.svelte`.

use gpui_kit::prelude::*;
use gpui_kit::{
    div, img, px, Context, Entity, FocusHandle, KeyDownEvent, ObjectFit, Subscription, Window,
};
use shared::user_config::{Permissions, WallpaperRestore};

use crate::backend::wallpaper;
use crate::keys::key_from_keystroke;
use crate::runtime;
use crate::store::{FeedbackKind, Store};
use crate::theme::{self, c};
use crate::ui::{self, Button, RadioOption, Size, Toggle};

struct PermissionRow {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    field: fn(&mut Permissions) -> &mut bool,
    read: fn(&Permissions) -> bool,
}

const PERMISSIONS: [PermissionRow; 3] = [
    PermissionRow {
        id: "set-wallpaper",
        label: "Change wallpaper",
        description: "Allow the pack/mode to set your desktop wallpaper.",
        field: |p| &mut p.set_wallpaper,
        read: |p| p.set_wallpaper,
    },
    PermissionRow {
        id: "open-links",
        label: "Open links",
        description: "Allow the pack/mode to open links in your browser.",
        field: |p| &mut p.open_links,
        read: |p| p.open_links,
    },
    PermissionRow {
        id: "send-notifications",
        label: "Show notifications",
        description: "Allow the pack/mode to show desktop notifications.",
        field: |p| &mut p.send_notifications,
        read: |p| p.send_notifications,
    },
];

#[derive(Clone, PartialEq)]
enum RestoreKind {
    Original,
    Image,
}

pub struct SafetyPage {
    store: Entity<Store>,
    /// Whether the original wallpaper can be put back. `None` until the probe answers, and if it
    /// fails; both are treated as "yes" rather than pushing the user into picking an image.
    can_restore_original: Option<bool>,
    recording: bool,
    recorder: FocusHandle,
    picking: bool,
    _blur: Subscription,
}

impl SafetyPage {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let recorder = cx.focus_handle();
        let blur = cx.on_blur(&recorder, window, |this, _, cx| {
            this.recording = false;
            cx.notify();
        });

        // Read once per page view rather than polled: it snapshots the desktop, and the answer
        // only changes across a session switch.
        cx.spawn(async move |this, cx| {
            let support = runtime::blocking(wallpaper::can_restore_original)
                .await
                .ok();
            let _ = this.update(cx, |this, cx| {
                this.can_restore_original = support;
                cx.notify();
            });
        })
        .detach();

        let shortcut_store = store.clone();
        cx.spawn(async move |_, cx| {
            if let Err(error) =
                runtime::run(shared::ipc::control::ensure_supervisor_running()).await
            {
                let _ = shortcut_store.update(cx, |store, cx| {
                    store.stop_shortcut = Some(shared::ipc::control::StopShortcutStatus::Failed {
                        message: format!("Couldn't check the stop shortcut: {error:#}"),
                    });
                    cx.notify();
                });
            }
        })
        .detach();

        Self {
            store,
            can_restore_original: None,
            recording: false,
            recorder,
            picking: false,
            _blur: blur,
        }
    }

    /// macOS shows its own prompt the first time it's asked, and adds Lewdware to the Input
    /// Monitoring list. After that it only answers, so open the list for the user to switch it on.
    /// Either way the supervisor notices the grant by itself (`supervisor/src/stop_key.rs`).
    fn ask_for_input_monitoring(&mut self, cx: &mut Context<Self>) {
        if shared::input_monitoring::request() {
            return;
        }
        if let Err(error) = shared::input_monitoring::open_settings() {
            self.store.update(cx, |store, cx| {
                store.set_feedback(
                    "input-monitoring",
                    FeedbackKind::Error,
                    format!("Couldn't open System Settings: {error}"),
                );
                cx.notify();
            });
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.recording {
            return;
        }
        // A bare modifier only qualifies another key; keep listening for that key.
        let Some(key) = key_from_keystroke(&event.keystroke) else {
            return;
        };
        cx.stop_propagation();
        self.recording = false;
        self.store
            .update(cx, |store, cx| store.set_stop_button(key, cx));
        cx.notify();
    }

    /// Choosing the image option must never leave the choice empty, so it adopts the bundled
    /// near-black placeholder at once -- obviously a placeholder, which nudges towards replacing it.
    fn choose_image_option(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        cx.spawn(async move |_, cx| {
            match runtime::blocking(wallpaper::default_restore_image)
                .await
                .and_then(|r| r)
            {
                Ok(path) => store.update(cx, |store, cx| {
                    store.set_wallpaper_restore(WallpaperRestore::Image { path }, cx)
                }),
                Err(error) => store.update(cx, |store, cx| {
                    store.set_feedback(
                        "wallpaper",
                        FeedbackKind::Error,
                        format!("Couldn't prepare the placeholder image: {error}"),
                    );
                    cx.notify();
                }),
            }
        })
        .detach();
    }

    fn pick_image(&mut self, cx: &mut Context<Self>) {
        self.picking = true;
        cx.notify();
        let store = self.store.clone();
        cx.spawn(async move |this, cx| {
            // Choosing is not loading: nothing shows busy until a file has been picked.
            let picked = runtime::run(wallpaper::pick_image()).await.ok().flatten();
            let adopted = match picked {
                Some(picked) => Some(
                    runtime::blocking(move || wallpaper::adopt_image(&picked))
                        .await
                        .and_then(|r| r),
                ),
                None => None,
            };
            store.update(cx, |store, cx| match adopted {
                Some(Ok(path)) => store.set_wallpaper_restore(WallpaperRestore::Image { path }, cx),
                Some(Err(error)) => {
                    store.set_feedback(
                        "wallpaper",
                        FeedbackKind::Error,
                        format!("Couldn't use that image: {error}"),
                    );
                    cx.notify();
                }
                None => {}
            });
            let _ = this.update(cx, |this, cx| {
                this.picking = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn setup_stop_shortcut(&mut self, retry: bool, _: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.clone();
        cx.spawn(async move |_, cx| {
            let result = runtime::run(async move {
                shared::ipc::control::ensure_supervisor_running().await?;
                let request = if retry {
                    shared::ipc::control::Request::RefreshStopShortcut
                } else {
                    shared::ipc::control::Request::SetupStopShortcut
                };
                match shared::ipc::control::request(&request).await? {
                    shared::ipc::control::Response::Ok => Ok(()),
                    shared::ipc::control::Response::Error { message } => {
                        Err(anyhow::anyhow!(message))
                    }
                    _ => Err(anyhow::anyhow!("Unexpected shortcut setup response")),
                }
            })
            .await;
            if let Err(error) = result {
                let _ = store.update(cx, |store, cx| {
                    store.set_feedback(
                        "stop-shortcut",
                        FeedbackKind::Error,
                        format!("Couldn't set up desktop shortcut: {error}"),
                    );
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Asks the desktop to show its own settings for the stop shortcut. The new key arrives as a
    /// status update once the user changes it there.
    fn configure_stop_shortcut(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        cx.spawn(async move |_, cx| {
            let result = runtime::run(async move {
                match shared::ipc::control::request(
                    &shared::ipc::control::Request::ConfigureStopShortcut,
                )
                .await?
                {
                    shared::ipc::control::Response::Ok => Ok(()),
                    shared::ipc::control::Response::Error { message } => {
                        Err(anyhow::anyhow!(message))
                    }
                    _ => Err(anyhow::anyhow!("Unexpected response")),
                }
            })
            .await;
            if let Err(error) = result {
                let _ = store.update(cx, |store, cx| {
                    store.set_feedback(
                        "stop-shortcut",
                        FeedbackKind::Error,
                        format!("Couldn't open the desktop's shortcut settings: {error}"),
                    );
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn stop_shortcut(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.store.read(cx).config.as_ref();
        let current = config
            .map(|c| c.stop_button.to_string())
            .unwrap_or_default();
        // On the portal path the desktop owns the binding and ignores a changed preference, so
        // show what it bound and send the user to its settings rather than offering a recorder
        // whose key would never take effect.
        let desktop_trigger = self
            .store
            .read(cx)
            .desktop_stop_trigger()
            .map(str::to_owned);
        use shared::ipc::control::StopShortcutStatus as Shortcut;
        let shortcut = self.store.read(cx).stop_shortcut.as_ref();
        let status = match shortcut {
            Some(Shortcut::Registered {
                managed_by_desktop: true,
                ..
            }) => "Registered with your desktop".into(),
            Some(Shortcut::Registered {
                backend, trigger, ..
            }) => format!("Registered via {backend}: {trigger}"),
            Some(Shortcut::Listening { backend }) => format!("Listening via {backend}"),
            Some(
                Shortcut::SetupRequired { message, .. }
                | Shortcut::PermissionRequired { message }
                | Shortcut::Failed { message },
            ) => message.clone(),
            Some(Shortcut::Checking) => "Checking desktop shortcut...".into(),
            None => "Supervisor is idle. Check the shortcut to verify desktop setup.".into(),
        };
        let can_configure = matches!(
            shortcut,
            Some(Shortcut::Registered {
                can_configure: true,
                ..
            })
        );
        let can_retry = matches!(shortcut, None | Some(Shortcut::Failed { .. }));
        let needs_permission = matches!(shortcut, Some(Shortcut::PermissionRequired { .. }));
        let can_setup = matches!(
            shortcut,
            Some(Shortcut::SetupRequired {
                can_setup: true,
                ..
            })
        );
        let recording = self.recording;
        let recorder = self.recorder.clone();

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(ui::section_title("Stop shortcut"))
            .child(ui::hint(
                "Pressing this key combination stops a running session immediately.",
            ))
            .child(ui::readout(status))
            .when(can_setup, |this| {
                this.child(
                    Button::new("setup-stop-shortcut", "Set up desktop shortcut")
                        .size(Size::Compact)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.setup_stop_shortcut(false, window, cx)
                        })),
                )
            })
            .when(can_retry, |this| {
                this.child(
                    Button::new(
                        "retry-stop-shortcut",
                        if shortcut.is_some() {
                            "Retry shortcut"
                        } else {
                            "Check shortcut"
                        },
                    )
                    .size(Size::Compact)
                    .on_click(
                        cx.listener(|this, _, window, cx| {
                            this.setup_stop_shortcut(true, window, cx)
                        }),
                    ),
                )
            })
            .when(needs_permission, |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded(theme::RADIUS_MD)
                        .border_1()
                        .border_color(c(theme::WARNING_BORDER))
                        .bg(c(theme::WARNING_BG))
                        .text_color(c(theme::WARNING))
                        .text_size(px(13.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child("The stop shortcut needs Input Monitoring permission.")
                                .child(div().flex_1())
                                .child(
                                    Button::new("input-monitoring", "Open settings")
                                        .size(Size::Compact)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ask_for_input_monitoring(cx)
                                        })),
                                ),
                        )
                        .child(
                            "In System Settings, turn on Lewdware under Privacy & Security, \
                             Input Monitoring. The shortcut starts working as soon as it's on; \
                             there's no need to restart Lewdware.",
                        ),
                )
            })
            .child(
                ui::card()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .p_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(ui::label("Current shortcut"))
                            .when(desktop_trigger.is_some() && !can_configure, |this| {
                                this.child(ui::hint(
                                    "Change it in your desktop's shortcut settings, under Lewdware.",
                                ))
                            }),
                    )
                    .when_some(desktop_trigger.clone(), |this, trigger| {
                        this.child(
                            div().flex().items_center().gap_2().child(
                            div()
                                .id("stop-desktop-trigger")
                                .min_w(px(160.))
                                .px_4()
                                .py_2()
                                .flex()
                                .justify_center()
                                .rounded(theme::RADIUS_MD)
                                .border_2()
                                .border_color(c(theme::BORDER))
                                .bg(c(theme::BG))
                                .text_size(px(13.))
                                .font_family(theme::FONT_MONO)
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .child(trigger),
                            )
                            .when(can_configure, |this| {
                                this.child(
                                    Button::new("configure-stop-shortcut", "Change...")
                                        .size(Size::Compact)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.configure_stop_shortcut(cx)
                                        })),
                                )
                            }),
                        )
                    })
                    .when(desktop_trigger.is_none(), |this| {
                        this.child(
                        div()
                            .id("stop-recorder")
                            .track_focus(&self.recorder)
                            .on_key_down(cx.listener(Self::on_key_down))
                            .min_w(px(160.))
                            .px_4()
                            .py_2()
                            .flex()
                            .justify_center()
                            .rounded(theme::RADIUS_MD)
                            .border_2()
                            .cursor_pointer()
                            .text_size(px(13.))
                            .map(|this| {
                                if recording {
                                    this.border_color(c(theme::ACCENT))
                                        .bg(c(theme::SURFACE_RAISED))
                                        .text_color(c(theme::MUTED))
                                        .italic()
                                        .child("Press a key...")
                                } else {
                                    this.border_color(c(theme::BORDER))
                                        .bg(c(theme::BG))
                                        .hover(|style| style.border_color(c(theme::BORDER_STRONG)))
                                        .font_family(theme::FONT_MONO)
                                        .font_weight(gpui_kit::FontWeight::BOLD)
                                        .child(current)
                                }
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.recording = true;
                                window.focus(&recorder, cx);
                                cx.notify();
                            })),
                        )
                    }),
            )
    }

    fn wallpaper_restore(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.store.read(cx).config.as_ref();
        let restore = config
            .map(|c| c.wallpaper.restore.clone())
            .unwrap_or_default();
        let can_restore_original = self.can_restore_original.unwrap_or(true);
        let (kind, image) = match &restore {
            WallpaperRestore::Original => (RestoreKind::Original, None),
            WallpaperRestore::Image { path } => (RestoreKind::Image, Some(path.clone())),
        };

        let store = self.store.clone();
        let this = cx.entity();
        let mut original = RadioOption::new(RestoreKind::Original, "Whatever it was before")
            .disabled(!can_restore_original);
        if !can_restore_original {
            original = original.description("Unavailable on this desktop");
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .px_4()
            .py_3()
            .bg(c(theme::BG))
            .child(ui::label("When Lewdware stops, set my wallpaper to").text_size(px(12.)))
            .child(ui::radio_group(
                "restore",
                &kind,
                vec![original, RadioOption::new(RestoreKind::Image, "This image")],
                move |kind, _, cx| match kind {
                    RestoreKind::Original => store.update(cx, |store, cx| {
                        store.set_wallpaper_restore(WallpaperRestore::Original, cx)
                    }),
                    RestoreKind::Image => this.update(cx, |this, cx| this.choose_image_option(cx)),
                },
            ))
            .when_some(image, |this, path| {
                // A stat per render is cheap, and it notices a file deleted while the app is open.
                let readable = path.is_file();
                this.child(
                    div()
                        .pl(px(28.))
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .w(px(96.))
                                .h(px(56.))
                                .flex_none()
                                .overflow_hidden()
                                .rounded(theme::RADIUS_MD)
                                .border_1()
                                .border_color(c(theme::BORDER))
                                .bg(c(theme::BG))
                                .when(readable, |this| {
                                    this.child(
                                        img(path.clone()).size_full().object_fit(ObjectFit::Cover),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(ui::readout(path.display().to_string()).truncate())
                                .when(!readable, |this| {
                                    this.child(ui::hint(
                                        "This image can't be read any more. Choose another.",
                                    ))
                                }),
                        )
                        .child(
                            Button::new("pick-restore", "Choose...")
                                .size(Size::Compact)
                                .disabled(self.picking)
                                .on_click(cx.listener(|this, _, _, cx| this.pick_image(cx))),
                        ),
                )
            })
    }

    fn permissions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let permissions = store
            .config
            .as_ref()
            .map(|c| c.permissions.clone())
            .unwrap_or_else(|| Permissions {
                set_wallpaper: false,
                open_links: false,
                send_notifications: false,
            });
        let image_chosen = matches!(
            store.config.as_ref().map(|c| &c.wallpaper.restore),
            Some(WallpaperRestore::Image { .. })
        );
        // A permission that is on but can never take effect should not claim to be "Allowed".
        let wallpaper_usable = self.can_restore_original.unwrap_or(true) || image_chosen;

        let mut rows = ui::card();
        for (index, row) in PERMISSIONS.iter().enumerate() {
            let allowed = (row.read)(&permissions);
            let usable = row.id != "set-wallpaper" || wallpaper_usable;
            let entity = self.store.clone();
            let field = row.field;
            if index > 0 {
                rows = rows.child(ui::seam());
            }
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_4()
                    .py_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(ui::label(row.label))
                            .child(ui::hint(row.description)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(c(if allowed && usable {
                                theme::TEXT
                            } else {
                                theme::MUTED
                            }))
                            .child(match (allowed, usable) {
                                (false, _) => "Denied",
                                (true, true) => "Allowed",
                                (true, false) => "Unavailable",
                            }),
                    )
                    .child(
                        Toggle::new(row.id, allowed).on_change(move |checked, _, cx| {
                            entity.update(cx, |store, cx| store.set_permission(field, checked, cx))
                        }),
                    ),
            );
            if row.id == "set-wallpaper" && allowed {
                rows = rows.child(ui::seam()).child(self.wallpaper_restore(cx));
            }
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_6()
            .border_t_1()
            .border_color(c(theme::BORDER))
            .child(ui::section_title("Permissions"))
            .child(rows)
    }
}

impl Render for SafetyPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shortcut = self.stop_shortcut(cx);
        let permissions = self.permissions(cx);
        ui::page(
            "safety",
            "Safety",
            "",
            [shortcut.into_any_element(), permissions.into_any_element()],
        )
    }
}
