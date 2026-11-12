#![recursion_limit = "256"]
//! The config app: configure the engine, pick a pack and mode, launch and stop sessions.

mod backend;
mod keys;
mod pages;
mod probe;
mod shell;
mod single_instance;
mod store;
mod theme;

use shared_gpui::{runtime, ui};

use gpui_kit::{px, size, App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions};

pub const APP_ID: &str = "com.lewdware.config";

/// Runs the app. Its executable (`src/main.rs` in development, a launcher loading the shared core
/// in releases; see `core/`) does nothing else.
pub fn run() {
    // A headless stand-in for testing, before anything else: see `probe.rs`.
    if let Some(code) = probe::run_if_requested() {
        std::process::exit(code);
    }
    let _log_guard = shared::logging::init("config").expect("Couldn't start logs");

    let mut show_requests = match runtime::block_on(single_instance::claim()) {
        Ok(single_instance::Claim::Secondary) => return,
        Ok(single_instance::Claim::Primary(requests)) => Some(requests),
        // Not being able to coordinate is no reason not to open.
        Err(error) => {
            tracing::warn!("couldn't set up single-instance handling: {error}");
            None
        }
    };

    gpui_kit::application()
        .with_assets(shared_gpui::icons::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::install(cx);

            let bounds = Bounds::centered(None, size(px(1000.), px(720.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Lewdware".into()),
                    ..Default::default()
                }),
                // The desktop file's name (`deploy/linux/`), which is where Wayland and X11 desktops
                // get the window icon from.
                app_id: Some(APP_ID.into()),
                ..Default::default()
            };
            let (window, _) = gpui_kit::open_window(options, cx, |_, cx| cx.new(shell::Shell::new))
                .expect("failed to open the window");

            if let Some(mut requests) = show_requests.take() {
                cx.spawn(async move |cx| {
                    while requests.recv().await.is_some() {
                        let _ = window.update(cx, |_, window, _| window.activate_window());
                    }
                })
                .detach();
            }
        });
}
