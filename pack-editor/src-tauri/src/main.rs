// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    shared::utils::apply_wayland_preload_safeguards();
    pack_editor_tauri_lib::run()
}
