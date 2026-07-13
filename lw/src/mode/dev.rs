use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::{fs, io, thread};

use clap::Args;
use notify::{Event, EventKind, RecommendedWatcher, Watcher};
use shared::ipc::{Request, Response};

use crate::mode::build::build_to;
use crate::mode::config::Config;
use crate::mode::{find_root, read_config, types::write_type_stubs};

#[derive(Args)]
pub struct DevArgs {}

pub fn dev(_args: DevArgs) -> anyhow::Result<()> {
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
        Path::new(&root.join("config.jsonc")),
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

    build_to(&mut file.file, &root, config)?;

    println!("Built");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    spawn_log_tail();

    rt.block_on(restart_session(&file.path))?;

    println!("Spawned");

    while let Ok(()) = rx.recv() {
        while let Ok(()) = rx.try_recv() {}
        thread::sleep(Duration::from_millis(200));
        while let Ok(()) = rx.try_recv() {
            thread::sleep(Duration::from_millis(200));
            while let Ok(()) = rx.try_recv() {}
        }

        let config = read_config(&root)?;

        let new_watched_dirs = include_dirs(&root, &config);
        update_watches(&mut watcher, &watched_dirs, &new_watched_dirs);
        watched_dirs = new_watched_dirs;

        file.file.seek(SeekFrom::Start(0))?;
        build_to(&mut file.file, &root, config)?;

        rt.block_on(restart_session(&file.path))?;
    }

    Ok(())
}

/// Asks the supervisor (starting it on demand if it isn't already running) to run this build,
/// replacing whatever session it's currently supervising -- both the very first spawn and every
/// rebuild go through this same call, since `RestartSession` is unconditional.
async fn restart_session(mode_path: &Path) -> anyhow::Result<()> {
    shared::ipc::ensure_supervisor_running().await?;

    match shared::ipc::request(&Request::RestartSession {
        mode_path: mode_path.to_path_buf(),
        dev: true,
    })
    .await?
    {
        Response::Error { message } => anyhow::bail!(message),
        _ => Ok(()),
    }
}

/// Reprints new lines appended to the engine's own rolling log file into this terminal. Now that
/// the supervisor -- not `lw dev` -- spawns the engine, `lw dev` no longer inherits its stdio for
/// free, so this reads the same file `shared::logging::init` already writes to instead (`tracing`'s
/// daily rotation names it `lewdware.log.<date>`, so the newest `lewdware.log*` file in the log
/// dir is tailed rather than a fixed name -- this also means a day rollover mid-session is
/// handled automatically, by switching to the fresh file). Starts from the current end of
/// whichever file is newest, not its history. Known limitation: this log file is shared per-day
/// across every engine session, not just this one -- lines from a concurrently-running
/// Sandbox/Experience session would interleave, an acceptable trade-off for a single-developer
/// workflow.
fn spawn_log_tail() {
    let Some(dir) = shared::logging::log_dir() else {
        return;
    };

    thread::spawn(move || {
        let mut current: Option<PathBuf> = None;
        let mut offset = 0u64;

        loop {
            thread::sleep(Duration::from_millis(150));

            let Some(latest) = latest_log_file(&dir) else {
                continue;
            };

            if current.as_ref() != Some(&latest) {
                // First time, or a day rollover produced a fresh file -- start from its current
                // end (0 if it was just created), not its whole history.
                offset = fs::metadata(&latest).map(|m| m.len()).unwrap_or(0);
                current = Some(latest);
            }
            let path = current.as_ref().expect("just set above");

            let Ok(metadata) = fs::metadata(path) else {
                continue;
            };
            let len = metadata.len();
            if len <= offset {
                continue;
            }

            let Ok(mut log_file) = File::open(path) else {
                continue;
            };
            if log_file.seek(SeekFrom::Start(offset)).is_err() {
                continue;
            }

            let mut buf = String::new();
            if log_file.read_to_string(&mut buf).is_ok() {
                print!("{buf}");
                offset = len;
            }
        }
    });
}

fn latest_log_file(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("lewdware.log"))
        })
        .max_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok())
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
        if !desired.contains(dir)
            && let Err(err) = watcher.unwatch(dir)
        {
            eprintln!("{err}");
        }
    }

    for dir in desired {
        if !current.contains(dir)
            && let Err(err) = watcher.watch(dir, notify::RecursiveMode::Recursive)
        {
            eprintln!("{err}");
        }
    }
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
