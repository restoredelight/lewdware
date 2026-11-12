// No console window behind the app on Windows. Debug builds keep it, for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    lewdware_pack_editor::run()
}
