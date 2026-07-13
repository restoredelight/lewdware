use std::path::PathBuf;
use std::process::Command;

use crate::utils::sanitize_child_env;

fn candidates_for(bin_name: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(exe) = std::env::current_exe()
        && let Ok(dir) = exe.canonicalize()
        && let Some(dir) = dir.parent().map(|p| p.to_owned())
    {
        // Windows / macOS, and Linux binary-adjacent installs: the binary sits next to
        // whatever's calling this.
        candidates.push(dir.join(bin_name));

        // Portable tar.gz on Linux: bin/lewdware -> lib/lewdware/{bin_name}.
        if let Some(root) = dir.parent() {
            candidates.push(root.join("lib").join("lewdware").join(bin_name));
        }
    }

    // Linux package installs (deb/rpm).
    #[cfg(target_os = "linux")]
    candidates.push(PathBuf::from("/usr/lib/lewdware").join(bin_name));

    candidates
}

fn find_binary(bin_name: &str, cargo_package: &str) -> Option<Command> {
    for path in candidates_for(bin_name) {
        if path.exists() {
            let mut cmd = Command::new(path);
            sanitize_child_env(&mut cmd);
            return Some(cmd);
        }
    }

    // Dev-workflow fallback: run straight out of the workspace via cargo. Not a real install, so
    // `sanitize_child_env` (which only matters for a packaged AppImage) doesn't apply here.
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

pub fn find_engine_binary() -> Option<Command> {
    let bin_name = if cfg!(windows) {
        "lewdware-engine.exe"
    } else {
        "lewdware-engine"
    };
    find_binary(bin_name, "lewdware")
}

pub fn find_supervisor_binary() -> Option<Command> {
    let bin_name = if cfg!(windows) {
        "lewdware-supervisor.exe"
    } else {
        "lewdware-supervisor"
    };
    find_binary(bin_name, "supervisor")
}
