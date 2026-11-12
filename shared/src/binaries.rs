use std::path::PathBuf;
use std::process::Command;

use crate::utils::sanitize_child_env;

/// The file name of the shared core library every app but `lw` is built into in releases
/// (`core/`).
#[cfg(windows)]
const CORE: &str = "lewdware_core.dll";
#[cfg(target_os = "macos")]
const CORE: &str = "liblewdware_core.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const CORE: &str = "liblewdware_core.so";

/// The directory the running code was loaded from: the shared core library's in a release, where
/// each app's executable is a small launcher that loads it (`launcher/`), and otherwise this
/// executable's. Use it, not `current_exe()`, for files that belong to the install rather than to
/// one app: the pack editor's launcher on macOS is in a bundle of its own, away from the rest.
///
/// Where it is in each install:
///
/// - Windows: the install directory, beside every executable.
/// - Linux: `/usr/lib/lewdware/` (or `lib/lewdware/` in the tarball), beside the engine and the
///   supervisor; the config app and the pack editor are in `../../bin/`.
/// - macOS: `Lewdware.app/Contents/Frameworks/`; the executables are in `../MacOS/`.
pub fn core_dir() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let core = loaded_from(core_dir as *const ())
            .filter(|path| path.file_name().is_some_and(|name| name == CORE));
        // Not built into the core: this is an app's own executable, in development. (dladdr
        // names the main executable however it was started, possibly relatively, so it isn't
        // asked for that.)
        let path = match core {
            Some(core) => core,
            None => std::env::current_exe().ok()?,
        };
        Some(path.canonicalize().ok()?.parent()?.to_owned())
    })
    .clone()
}

/// The file of the module (executable or shared library) that `address` is in.
#[cfg(unix)]
fn loaded_from(address: *const ()) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    // Safety: dladdr only reads `address`, and fills `info` with pointers into the loader's own
    // tables, which live as long as the module does; this one never unloads.
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(address.cast(), &mut info) == 0 || info.dli_fname.is_null() {
            return None;
        }
        let name = std::ffi::CStr::from_ptr(info.dli_fname);
        Some(PathBuf::from(std::ffi::OsStr::from_bytes(name.to_bytes())))
    }
}

/// The file of the module (executable or DLL) that `address` is in.
#[cfg(windows)]
fn loaded_from(address: *const ()) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(
            flags: u32,
            name: *const u16,
            module: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn GetModuleFileNameW(module: *mut std::ffi::c_void, name: *mut u16, size: u32) -> u32;
    }

    // Safety: with FROM_ADDRESS, `name` is read as an address inside a module, not a string, and
    // the handle isn't reference-counted, so there is nothing to free. The buffer's length is
    // passed with it.
    unsafe {
        let mut module = std::ptr::null_mut();
        if GetModuleHandleExW(
            FROM_ADDRESS | UNCHANGED_REFCOUNT,
            address.cast(),
            &mut module,
        ) == 0
        {
            return None;
        }
        let mut name = vec![0u16; 32 * 1024];
        let len = GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize;
        (len > 0 && len < name.len())
            .then(|| PathBuf::from(std::ffi::OsString::from_wide(&name[..len])))
    }
}

fn candidates_for(bin_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // Releases: relative to the shared core (see `core_dir` for the layouts).
    if let Some(dir) = core_dir() {
        candidates.push(dir.join(bin_name));
        #[cfg(target_os = "macos")]
        if let Some(contents) = dir.parent() {
            candidates.push(contents.join("MacOS").join(bin_name));
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        if let Some(prefix) = dir.parent().and_then(|lib| lib.parent()) {
            candidates.push(prefix.join("bin").join(bin_name));
        }
    }

    if let Ok(exe) = std::env::current_exe()
        && let Ok(dir) = exe.canonicalize()
        && let Some(dir) = dir.parent().map(|p| p.to_owned())
    {
        candidates.push(dir.join(bin_name));

        if let Some(root) = dir.parent() {
            candidates.push(root.join("lib").join("lewdware").join(bin_name));
        }
    }

    #[cfg(target_os = "linux")]
    candidates.push(PathBuf::from("/usr/lib/lewdware").join(bin_name));

    candidates
}

fn find_installed_path(bin_name: &str) -> Option<PathBuf> {
    candidates_for(bin_name)
        .into_iter()
        .find(|path| path.exists())
}

fn find_binary(bin_name: &str, #[allow(unused)] cargo_package: &str) -> Option<Command> {
    if let Some(path) = find_installed_path(bin_name) {
        let mut cmd = Command::new(path);
        sanitize_child_env(&mut cmd);
        return Some(cmd);
    }

    // Dev-workflow fallback: run straight out of the workspace via cargo.
    #[cfg(debug_assertions)]
    {
        let mut current_dir = std::env::current_dir().ok();
        while let Some(dir) = current_dir {
            if dir.join("Cargo.toml").exists() {
                let mut cmd = Command::new("cargo");
                cmd.args(["run", "-p", cargo_package, "--"]);
                return Some(cmd);
            }
            current_dir = dir.parent().map(|p| p.to_path_buf());
        }
    }

    if let Ok(path) = which::which(bin_name) {
        let mut cmd = Command::new(path);
        sanitize_child_env(&mut cmd);
        return Some(cmd);
    }

    None
}

fn supervisor_bin_name() -> &'static str {
    if cfg!(windows) {
        "lewdware-supervisor.exe"
    } else {
        "lewdware-supervisor"
    }
}

pub fn find_engine_binary() -> Option<Command> {
    let bin_name = if cfg!(windows) {
        "lewdware-engine.exe"
    } else {
        "lewdware-engine"
    };
    find_binary(bin_name, "lewdware")
}

/// The config app -- what the tray's "Open Lewdware" launches. Its `[[bin]]` is plainly
/// `lewdware`, unlike the engine and supervisor which carry suffixed names.
pub fn find_config_binary() -> Option<Command> {
    let bin_name = if cfg!(windows) {
        "lewdware.exe"
    } else {
        "lewdware"
    };
    find_binary(bin_name, "lewdware-config")
}

pub fn find_supervisor_binary() -> Option<Command> {
    find_binary(supervisor_bin_name(), "lewdware-supervisor")
}

pub fn find_supervisor_binary_path() -> Option<PathBuf> {
    find_installed_path(supervisor_bin_name()).or_else(|| which::which(supervisor_bin_name()).ok())
}

/// Makes the config app's executable run the supervisor instead (`launcher/src/bin/
/// launcher-config.rs`). The supervisor accepts and ignores it.
pub const SUPERVISOR_ROLE_FLAG: &str = "--supervisor";

/// What to register to start the supervisor at login: a program and its arguments.
///
/// macOS: Lewdware.app's main executable in the supervisor role, not `lewdware-supervisor`.
/// Permissions (Input Monitoring, for the stop shortcut) go to the app a process runs as, and a
/// process launchd starts runs as its own executable unless that's the app's main one. The
/// supervisor the config app starts runs as Lewdware.app by being its child; started at login as
/// `lewdware-supervisor` it would need a grant of its own, which nothing asks for.
pub fn supervisor_autostart_command() -> Option<(PathBuf, Vec<String>)> {
    #[cfg(target_os = "macos")]
    if let Some(main) = find_installed_path("lewdware").filter(|path| {
        // Only inside a bundle: a development build's `lewdware` is the config app alone.
        path.parent()
            .filter(|dir| dir.ends_with("Contents/MacOS"))
            .is_some()
    }) {
        return Some((main, vec![SUPERVISOR_ROLE_FLAG.to_owned()]));
    }
    find_supervisor_binary_path().map(|path| (path, Vec::new()))
}
