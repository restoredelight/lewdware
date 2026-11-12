//! Every app but `lw`, in one shared library. Each app's launcher (`launcher/`) loads it and calls
//! [`lewdware_main`] with the role it was built for, so the code, and the FFmpeg statically linked
//! into it, is on disk once rather than once per executable.
//!
//! The process is still the launcher's: `std::env::current_exe()` is the launcher (so an app that
//! re-runs itself, like the pack editor's encode workers, re-runs its own launcher), and on macOS
//! the app's identity comes from the bundle the launcher is in. Files that belong to the install
//! as a whole are found from this library instead, with `shared::binaries::core_dir`.

use std::ffi::{CStr, c_char, c_int};

/// Runs `role` (`config`, `pack-editor`, `engine` or `supervisor`) with this process's command
/// line, and returns the process's exit code.
///
/// # Safety
///
/// `role` must be a valid NUL-terminated string. Call it from the process's main thread, once:
/// the GUI apps' event loops require the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lewdware_main(role: *const c_char) -> c_int {
    let role = unsafe { CStr::from_ptr(role) }.to_string_lossy().into_owned();
    // A panic must not unwind into the launcher, which isn't Rust built with this library's
    // compiler. The panic hook has already reported it.
    std::panic::catch_unwind(|| run(&role)).unwrap_or(101)
}

fn run(role: &str) -> c_int {
    match role {
        "config" => {
            lewdware_config::run();
            0
        }
        "pack-editor" => {
            lewdware_pack_editor::run();
            0
        }
        "engine" => report(lewdware::run().map(c_int::from)),
        "supervisor" => report(lewdware_supervisor::run().map(|()| 0)),
        _ => {
            eprintln!("lewdware_core: unknown role {role:?}");
            2
        }
    }
}

/// What returning an error from `main` does: print it and exit with 1.
fn report(result: anyhow::Result<c_int>) -> c_int {
    result.unwrap_or_else(|err| {
        eprintln!("Error: {err:?}");
        1
    })
}
