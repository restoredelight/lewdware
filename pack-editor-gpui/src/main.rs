#![recursion_limit = "256"]
//! The pack editor, ported to gpui. Runs beside `pack-editor/` until it can replace it; see
//! `README.md` for what's ported.

mod app;
mod editor;
mod imports;
mod keep_awake;
mod menu;
mod shell;
mod views;

use shared_gpui::{icons, runtime, theme, ui};

use gpui_kit::{
    px, size, App, AppContext, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions,
};

fn main() {
    let _log_guard = shared::logging::init("pack-editor").expect("Couldn't start logs");
    use_bundled_binaries();
    editor::detect_encoder();

    gpui_kit::application()
        .with_assets(shared_gpui::icons::AppAssets)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            theme::install(cx);
            bind_keys(cx);

            let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(800.), px(520.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(app::APP_TITLE.into()),
                    ..Default::default()
                }),
                app_id: Some("lewdware-pack-editor".into()),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| app::Root::new(window, cx))
            })
            .expect("failed to open the window");
        });
}

/// Cmd on macOS, Ctrl elsewhere. (gpui's `secondary-` only resolves when parsed with the OS,
/// which `KeyBinding::new` doesn't pass.)
fn bind_keys(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    // Bindings borrow their keystroke for `'static`; these few are made once, at startup.
    let key = |keys: &str| -> &'static str { format!("{modifier}-{keys}").leak() };
    let context = Some(app::KEY_CONTEXT);
    let mut bindings = vec![
        KeyBinding::new(key("z"), app::Undo, context),
        KeyBinding::new(key("shift-z"), app::Redo, context),
        KeyBinding::new(key("s"), app::Save, context),
        KeyBinding::new(key("shift-s"), app::SaveAs, context),
        KeyBinding::new(key("w"), app::ClosePack, context),
        KeyBinding::new(key("n"), app::NewPack, context),
        KeyBinding::new(key("o"), app::OpenPack, context),
        KeyBinding::new(key("i"), app::ImportFiles, context),
    ];
    if !cfg!(target_os = "macos") {
        bindings.push(KeyBinding::new("ctrl-y", app::Redo, context));
    }
    cx.bind_keys(bindings);
}

/// The ffmpeg, ffprobe and avifenc sidecars ship in `binaries/` next to the executable (the Tauri
/// build put them in its resource directory). Without them, `shared::encode` looks on `PATH`.
fn use_bundled_binaries() {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("binaries")))
    else {
        return;
    };
    let exe = |name: &str| dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let (ffmpeg, ffprobe, avifenc) = (
        exe("lewdware-ffmpeg"),
        exe("lewdware-ffprobe"),
        exe("lewdware-avifenc"),
    );
    if ffmpeg.exists() && ffprobe.exists() && avifenc.exists() {
        shared::encode::init_binary_paths(ffmpeg, ffprobe, avifenc);
    }
}
