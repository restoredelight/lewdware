//! `lewdware`: the config app. See `../lib.rs`.
// No console window on Windows in releases, as the app itself had none.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // macOS starts the supervisor at login through this executable, so that it runs as
    // Lewdware.app (`shared::binaries::supervisor_autostart_command`).
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--supervisor")
    {
        lewdware_launcher::launch(lewdware_launcher::App {
            role: "supervisor",
            name: "Lewdware",
            app_user_model_id: None,
        })
    }
    lewdware_launcher::launch(lewdware_launcher::App {
        role: "config",
        name: "Lewdware",
        app_user_model_id: Some("com.lewdware.config"),
    })
}
