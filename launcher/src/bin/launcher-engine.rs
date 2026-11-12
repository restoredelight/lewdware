//! `lewdware-engine`, which the supervisor starts for each session. See `../lib.rs`.
// No console window on Windows in releases, as the app itself had none.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lewdware_launcher::launch(lewdware_launcher::App {
        role: "engine",
        name: "Lewdware",
        // Left to Windows: the session's windows group by the engine's own path, as before.
        app_user_model_id: None,
    })
}
