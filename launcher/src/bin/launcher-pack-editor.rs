//! `lewdware-pack-editor`. See `../lib.rs`.
// No console window on Windows in releases, as the app itself had none.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lewdware_launcher::launch(lewdware_launcher::App {
        role: "pack-editor",
        name: "Lewdware Pack Editor",
        app_user_model_id: Some("com.lewdware.pack-editor"),
    })
}
