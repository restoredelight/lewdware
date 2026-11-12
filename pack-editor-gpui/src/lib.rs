#![recursion_limit = "256"]
//! The pack editor. See `README.md`.

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

/// Runs the app. Its executable (`src/main.rs` in development, a launcher loading the shared core
/// in releases; see `core/`) does nothing else.
pub fn run() {
    // Before anything else: as an encode worker this process does one encode and exits.
    shared::encode::serve_encode_worker_if_requested();
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
                // The desktop file's name, and the Flatpak's app id: desktops match the window
                // to it for its icon.
                app_id: Some("com.lewdware.pack-editor".into()),
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

/// Encodes run in a child copy of this executable (see `shared::encode::use_encode_worker`), so a
/// file that crashes a decoder fails its import instead of closing the editor.
///
/// Images are the one thing still encoded by a separate program: the avifenc sidecar, which the
/// installers put in a `binaries/` directory (see `sidecar_dirs`). Without it, `shared::encode`
/// looks on `PATH`.
fn use_bundled_binaries() {
    if let Ok(exe) = std::env::current_exe() {
        shared::encode::use_encode_worker(exe);
    }
    for dir in sidecar_dirs() {
        let avifenc = dir.join(format!("lewdware-avifenc{}", std::env::consts::EXE_SUFFIX));
        if avifenc.exists() {
            shared::encode::init_avifenc_path(avifenc);
            return;
        }
    }
}

/// Where each package puts the sidecar:
///
/// - Windows: `binaries\` in the install directory, beside the shared core (and the `.exe`).
/// - Linux (`.deb`, `.rpm`, tarball): `lib/lewdware/binaries/`, beside the shared core.
/// - macOS: the pack editor's own `Contents/Resources/binaries/`, from its `Contents/MacOS/`.
///
/// Debug builds also look in the crate's own `binaries/`, where the `deploy/*/*_avifenc_sidecar`
/// scripts put it.
fn sidecar_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = shared::binaries::core_dir() {
        dirs.push(dir.join("binaries"));
    }
    if let Some(contents) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .and_then(|exe| Some(exe.parent()?.parent()?.to_owned()))
    {
        dirs.push(contents.join("Resources").join("binaries"));
    }
    if cfg!(debug_assertions) {
        dirs.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries"));
    }
    dirs
}
