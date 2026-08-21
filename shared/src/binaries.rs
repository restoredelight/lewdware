use std::path::PathBuf;
use std::process::Command;

use crate::utils::sanitize_child_env;

fn candidates_for(bin_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

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
    let bin_name = if cfg!(windows) { "lewdware.exe" } else { "lewdware" };
    find_binary(bin_name, "lewdware-config")
}

pub fn find_supervisor_binary() -> Option<Command> {
    find_binary(supervisor_bin_name(), "supervisor")
}

pub fn find_supervisor_binary_path() -> Option<PathBuf> {
    find_installed_path(supervisor_bin_name()).or_else(|| which::which(supervisor_bin_name()).ok())
}
