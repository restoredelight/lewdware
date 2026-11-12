//! What each app's executable does in a release: find the shared core library (`core/`), load it,
//! and run this app's role in it. See `Cargo.toml` for why there is one executable per app.
//!
//! Where the core is, relative to this executable:
//!
//! - Windows: `lewdware_core.dll` beside it.
//! - Linux: `liblewdware_core.so` beside it (`/usr/lib/lewdware/`, for the engine and supervisor),
//!   or in `../lib/lewdware/` (`/usr/bin/`, for the config app and the pack editor). The same
//!   relative layout works unpacked from the tarball.
//! - macOS: in `Contents/Frameworks/` of this app's bundle (`Lewdware.app`). The pack editor's
//!   bundle has none of its own: it uses the one in a `Lewdware.app` beside it, or failing that,
//!   wherever LaunchServices says `Lewdware.app` is.
//!
//! Beside the executable also covers `target/release/` in a development checkout.

use std::ffi::{CString, c_char, c_int};
use std::path::{Path, PathBuf};

#[cfg(windows)]
const CORE: &str = "lewdware_core.dll";
#[cfg(target_os = "macos")]
const CORE: &str = "liblewdware_core.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const CORE: &str = "liblewdware_core.so";

/// The app a launcher starts.
pub struct App {
    /// The role `lewdware_main` runs (`core/src/lib.rs`).
    pub role: &'static str,
    /// Shown when the core can't be found or loaded.
    pub name: &'static str,
    /// Windows: the AppUserModelID, which the taskbar groups and pins windows by. The installer's
    /// Start menu shortcut carries the same one (`deploy/windows/installer.iss`), so a pinned
    /// shortcut and the running app share a taskbar button. `None` leaves Windows to derive one
    /// from the executable's path.
    pub app_user_model_id: Option<&'static str>,
}

/// Loads the core and runs `app` in it, then exits with its exit code.
pub fn launch(app: App) -> ! {
    #[cfg(windows)]
    if let Some(id) = app.app_user_model_id {
        windows::set_app_user_model_id(id);
    }

    let exe = std::env::current_exe().and_then(|exe| exe.canonicalize());
    let Some(core) = exe.as_deref().ok().and_then(find_core) else {
        fail(&app, &format!(
            "{} couldn't find {CORE}, the library that holds Lewdware's code. Reinstall Lewdware to fix this.",
            app.name
        ));
    };

    type Main = unsafe extern "C" fn(*const c_char) -> c_int;
    let role = CString::new(app.role).expect("role has no NUL");
    // Safety: the core is ours, and `lewdware_main` has this signature (core/src/lib.rs). Its
    // initialisers have no requirements, and this is the main thread, which it asks for.
    let code = unsafe {
        let library = match libloading::Library::new(&core) {
            Ok(library) => library,
            Err(err) => fail(&app, &format!("{} couldn't load {}: {err}", app.name, core.display())),
        };
        let main: libloading::Symbol<Main> = match library.get(b"lewdware_main\0") {
            Ok(main) => main,
            Err(err) => fail(&app, &format!("{} is not Lewdware's core: {err}", core.display())),
        };
        let code = main(role.as_ptr());
        // Never unloaded: threads it started, and on macOS the Objective-C classes it registered,
        // keep pointing into it until the process exits.
        std::mem::forget(library);
        code
    };
    std::process::exit(code);
}

fn find_core(exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    let mut candidates = vec![dir.join(CORE)];

    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(prefix) = dir.parent() {
        candidates.push(prefix.join("lib/lewdware").join(CORE));
    }

    #[cfg(target_os = "macos")]
    {
        // dir = X.app/Contents/MacOS
        if let Some(contents) = dir.parent() {
            candidates.push(contents.join("Frameworks").join(CORE));
            if let Some(folder) = contents.parent().and_then(Path::parent) {
                candidates.push(folder.join(macos::MAIN_APP).join(macos::FRAMEWORKS).join(CORE));
            }
        }
        if let Some(app) = macos::main_app_from_launch_services() {
            candidates.push(app.join(macos::FRAMEWORKS).join(CORE));
        }
    }

    candidates.into_iter().find(|path| path.is_file())
}

/// Tells the user why the app can't start, then exits.
fn fail(app: &App, message: &str) -> ! {
    eprintln!("{message}");
    #[cfg(windows)]
    windows::message_box(app.name, message);
    #[cfg(target_os = "macos")]
    macos::alert(app.name, message);
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = app;
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;

    pub const MAIN_APP: &str = "Lewdware.app";
    pub const FRAMEWORKS: &str = "Contents/Frameworks";
    /// `Lewdware.app`'s (deploy/macos/build_installer.sh).
    const MAIN_BUNDLE_ID: &str = "com.lewdware";

    /// Where LaunchServices last saw `Lewdware.app`, for when the pack editor isn't beside it.
    pub fn main_app_from_launch_services() -> Option<PathBuf> {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::NSString;

        let url = NSWorkspace::sharedWorkspace()
            .URLForApplicationWithBundleIdentifier(&NSString::from_str(MAIN_BUNDLE_ID))?;
        Some(PathBuf::from(url.path()?.to_string()))
    }

    pub fn alert(title: &str, message: &str) {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSAlert, NSApplication, NSApplicationActivationPolicy};
        use objc2_foundation::NSString;

        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(message));
        alert.runModal();
    }
}

#[cfg(windows)]
mod windows {
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SetCurrentProcessExplicitAppUserModelID(id: *const u16) -> i32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(owner: *mut core::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    /// Before any window exists, which is when it has to be set.
    pub fn set_app_user_model_id(id: &str) {
        let id = wide(id);
        // Safety: a valid NUL-terminated string, copied by the call. A failure (an HRESULT) only
        // means the taskbar groups by the executable's path instead.
        unsafe { SetCurrentProcessExplicitAppUserModelID(id.as_ptr()) };
    }

    pub fn message_box(title: &str, message: &str) {
        const MB_ICONERROR: u32 = 0x10;
        let (title, message) = (wide(title), wide(message));
        // Safety: valid NUL-terminated strings and no owner window.
        unsafe { MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title.as_ptr(), MB_ICONERROR) };
    }
}
