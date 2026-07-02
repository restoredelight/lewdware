use std::fs::File;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::{env, fs, io, thread};

use anyhow::{Context, bail};
use clap::Args;
use notify::{Event, EventKind, RecommendedWatcher, Watcher};

use crate::mode::build::build_to;
use crate::mode::config::Config;
use crate::mode::{find_root, read_config, types::write_type_stubs};

#[derive(Args)]
pub struct DevArgs {
    // The mode to use
    mode: Option<String>,
}

pub fn dev(args: DevArgs) -> anyhow::Result<()> {
    let root = find_root()?;

    let (tx, rx) = channel();

    let mut watcher = notify::recommended_watcher(move |event: Result<Event, _>| {
        if let Ok(event) = event {
            match event.kind {
                EventKind::Access(_) => {}
                _ => {
                    let _ = tx.send(());
                }
            }
        }
    })?;

    watcher.watch(
        &Path::new(&root.join("config.jsonc")),
        notify::RecursiveMode::NonRecursive,
    )?;

    write_type_stubs(&root)?;

    let build_dir = root.join("build");
    fs::create_dir_all(&build_dir)?;

    let config = read_config(&root)?;

    let mut watched_dirs = include_dirs(&root, &config);
    for dir in &watched_dirs {
        watcher.watch(dir, notify::RecursiveMode::Recursive)?;
    }

    let path = build_dir.join(format!("{}.lwmode", config.name));
    let mut file = BuildFile::new(path)?;

    println!("Created build file");

    let mode = args
        .mode
        .clone()
        .or_else(|| config.modes.keys().next().cloned())
        .context("config.jsonc contains no modes")?;

    if !config.modes.contains_key(&mode) {
        bail!("Invalid mode '{mode}'");
    }

    build_to(&mut file.file, &root, config)?;

    println!("Built");

    let mut process = spawn_lewdware(&file.path, &mode)?;

    println!("Spawned");

    while let Ok(()) = rx.recv() {
        println!("Oh?");
        while let Ok(()) = rx.try_recv() {}
        thread::sleep(Duration::from_millis(200));
        loop {
            if let Ok(()) = rx.try_recv() {
                thread::sleep(Duration::from_millis(200));
                while let Ok(()) = rx.try_recv() {}
            } else {
                break;
            }
        }

        terminate(&mut process);
        process.wait().ok();

        let config = read_config(&root)?;

        let new_watched_dirs = include_dirs(&root, &config);
        update_watches(&mut watcher, &watched_dirs, &new_watched_dirs);
        watched_dirs = new_watched_dirs;

        let mode = args
            .mode
            .clone()
            .or_else(|| config.modes.keys().next().cloned())
            .context("config.jsonc contains no modes")?;

        if !config.modes.contains_key(&mode) {
            bail!("Invalid mode '{mode}'");
        }

        file.file.seek(SeekFrom::Start(0))?;
        build_to(&mut file.file, &root, config)?;

        process = spawn_lewdware(&file.path, &mode)?;
    }

    Ok(())
}

// Resolves config.include entries (paths relative to root) to canonical,
// existing directory paths. Entries that don't exist are silently skipped,
// matching the behaviour of the equivalent logic in build.rs.
fn include_dirs(root: &Path, config: &Config) -> Vec<PathBuf> {
    config
        .include
        .iter()
        .filter_map(|path| {
            root.join(path)
                .canonicalize()
                .inspect_err(|err| eprintln!("{err}"))
                .ok()
        })
        .collect()
}

// Diffs `current` against `desired` and adjusts the watcher so it ends up
// watching exactly `desired`, so a change to config.include takes effect
// without restarting `lw dev`.
fn update_watches(watcher: &mut RecommendedWatcher, current: &[PathBuf], desired: &[PathBuf]) {
    for dir in current {
        if !desired.contains(dir) {
            if let Err(err) = watcher.unwatch(dir) {
                eprintln!("{err}");
            }
        }
    }

    for dir in desired {
        if !current.contains(dir) {
            if let Err(err) = watcher.watch(dir, notify::RecursiveMode::Recursive) {
                eprintln!("{err}");
            }
        }
    }
}

fn spawn_lewdware(path: &Path, mode: &str) -> anyhow::Result<Child> {
    let mut command = find_lewdware_binary().context("Couldn't find lewdware binary")?;

    Ok(command
        .arg("--mode-path")
        .arg(path)
        .arg("--mode")
        .arg(mode)
        .spawn()?)
}

struct BuildFile {
    path: PathBuf,
    file: File,
}

impl BuildFile {
    fn new(path: PathBuf) -> io::Result<Self> {
        let file = File::create(&path)?;

        Ok(Self { path, file })
    }
}

impl Drop for BuildFile {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_file(&self.path) {
            eprintln!("{err}");
        }
    }
}

fn terminate(child: &mut Child) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;
        let _ = signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM);
    }
    #[cfg(not(unix))]
    let _ = child.kill();
}

fn find_lewdware_binary() -> Option<Command> {
    let bin_name = if cfg!(target_os = "windows") {
        "lewdware-engine.exe"
    } else {
        "lewdware-engine"
    };

    if let Ok(current_exe) = env::current_exe() {
        if let Ok(real_lw_path) = fs::canonicalize(current_exe) {
            if let Some(bin_dir) = real_lw_path.parent() {
                let neighbor = bin_dir.join(bin_name);
                if neighbor.exists() {
                    println!("Found executable: {}", neighbor.display());
                    return Some(Command::new(neighbor));
                }
            }
        }
    }

    #[cfg(debug_assertions)]
    {
        let mut current_dir = env::current_dir().ok();
        while let Some(dir) = current_dir {
            println!("Running cargo command");
            if dir.join("Cargo.toml").exists() {
                let mut cmd = std::process::Command::new("cargo");
                cmd.args(["run", "-p", "lewdware", "--"]);
                return Some(cmd);
            }
            current_dir = dir.parent().map(|p| p.to_path_buf());
        }
    }

    if let Ok(path) = which::which(bin_name) {
        return Some(Command::new(path));
    }

    None
}
