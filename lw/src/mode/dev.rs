use std::fs::File;
use std::io::{IsTerminal, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::{fs, io, thread};

use clap::Args;
use notify::{Event, EventKind, RecommendedWatcher, Watcher};
use shared::ipc::{Request, Response};
use shared::logging::LogRecord;
use uuid::Uuid;

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

    let dev_stream_id = Uuid::new_v4();
    rt.block_on(shared::ipc::ensure_supervisor_running())?;
    let log_subscription = rt.block_on(shared::ipc::subscribe_dev_logs(dev_stream_id))?;
    spawn_dev_log_stream(log_subscription);

    rt.block_on(restart_session(&file.path, dev_stream_id))?;

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

        rt.block_on(restart_session(&file.path, dev_stream_id))?;
    }

    Ok(())
}

/// Asks the supervisor (starting it on demand if it isn't already running) to run this build,
/// replacing whatever session it's currently supervising -- both the very first spawn and every
/// rebuild go through this same call, since `RestartSession` is unconditional.
async fn restart_session(mode_path: &Path, dev_stream_id: Uuid) -> anyhow::Result<()> {
    shared::ipc::ensure_supervisor_running().await?;

    match shared::ipc::request(&Request::RestartSession {
        mode_path: mode_path.to_path_buf(),
        dev: true,
        dev_stream_id: Some(dev_stream_id),
    })
    .await?
    {
        Response::Error { message } => anyhow::bail!(message),
        _ => Ok(()),
    }
}

/// Renders the typed, session-scoped stream supplied by the supervisor. This thread owns a small
/// runtime because the file watcher below deliberately remains blocking; neither side needs to
/// poll a shared log file or interpret tracing's presentation format.
fn spawn_dev_log_stream(mut subscription: shared::ipc::DevLogSubscription) {
    thread::spawn(move || {
        let use_ansi = std::io::stdout().is_terminal();
        let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        rt.block_on(async move {
            while let Ok(record) = subscription.next().await {
                println!("{}", format_dev_log_record(&record, use_ansi));
            }
        });
    });
}

fn format_dev_log_record(record: &LogRecord, use_ansi: bool) -> String {
    let level = record.level.label();
    let colour = match record.level {
        shared::logging::LogLevel::Error => "\x1b[31m",
        shared::logging::LogLevel::Warn => "\x1b[33m",
        shared::logging::LogLevel::Info => "\x1b[32m",
        shared::logging::LogLevel::Debug => "\x1b[34m",
        shared::logging::LogLevel::Trace => "\x1b[35m",
    };
    let message = if record.message.is_empty() && !record.fields.is_empty() {
        serde_json::to_string(&record.fields).unwrap_or_default()
    } else {
        record.message.clone()
    };

    if use_ansi {
        format!("{colour}{level}\x1b[0m {message}")
    } else {
        format!("{level} {message}")
    }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::format_dev_log_record;
    use shared::logging::{LogLevel, LogRecord};

    fn record(level: LogLevel, message: &str) -> LogRecord {
        LogRecord {
            schema: 1,
            timestamp: Utc::now(),
            level,
            component: "lewdware".into(),
            target: "lewdware::lua".into(),
            message: message.into(),
            file: None,
            line: None,
            session_id: Some("1".into()),
            fields: BTreeMap::new(),
        }
    }

    #[test]
    fn dev_log_records_only_show_level_and_message() {
        assert_eq!(
            format_dev_log_record(&record(LogLevel::Error, "something broke"), false),
            "ERROR something broke"
        );
    }

    #[test]
    fn dev_log_levels_are_colourized_for_terminals() {
        assert_eq!(
            format_dev_log_record(&record(LogLevel::Warn, "be careful"), true),
            "\x1b[33mWARN\x1b[0m be careful"
        );
    }
}
