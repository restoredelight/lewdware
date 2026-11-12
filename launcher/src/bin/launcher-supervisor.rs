//! `lewdware-supervisor`. See `../lib.rs`.
//!
//! A console program on Windows, like the supervisor itself: its one-shot commands
//! (`lewdware-supervisor stop`, which the installers run) print to the console. The config app
//! starts the daemon without a window.

fn main() {
    lewdware_launcher::launch(lewdware_launcher::App {
        role: "supervisor",
        name: "Lewdware",
        app_user_model_id: None,
    })
}
