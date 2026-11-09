#![recursion_limit = "256"]
//! The config app, rebuilt on gpui. A pilot: it runs beside `config/` until it can replace it.

mod backend;
mod keys;
mod pages;
mod shell;
mod single_instance;
mod store;
mod theme;

use shared_gpui::{runtime, ui};

use gpui_kit::{px, size, App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions};

fn main() {
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
                app_id: Some("lewdware-config".into()),
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
