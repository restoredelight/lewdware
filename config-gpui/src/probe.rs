//! A headless stand-in for the config app, for testing on machines that can't run gpui (the macOS
//! VMs have no Metal). It runs the app's own backend code from the app's own executable, so
//! whatever it starts runs as the config app would start it: the supervisor as its child, under
//! the same app identity, which is what macOS's permissions go by.
//!
//!   lewdware --probe SCRIPT LOG
//!
//! On macOS start it the way a user would, so it's Lewdware.app to the system:
//! `open -a /Applications/Lewdware.app --args --probe SCRIPT LOG`.
//!
//! SCRIPT has one step per line (blank lines and `#` comments are skipped). Each step and its
//! result goes to LOG, with the time:
//!
//!   status                  the supervisor's status, or that none is running
//!   launch                  start a session, starting a supervisor if needed (the Launch button)
//!   stop                    stop the session
//!   follow SECONDS          subscribe to the supervisor's status for that long, as an open
//!                           window does (which keeps it running), logging each change
//!   wait-shortcut KIND SECS follow until the stop shortcut's status is KIND (`Listening`,
//!                           `PermissionRequired`, ...), or fail after SECS
//!   permission              whether this process has Input Monitoring (on macOS it's the
//!                           supervisor's to have; this is for comparison)
//!   request-permission      the "Open settings" button: asks the supervisor, which shows
//!                           macOS's prompt the first time, else opens System Settings
//!   open-settings           System Settings' Input Monitoring list alone
//!   autostart on|off        register or remove the supervisor's start-at-login
//!   sleep SECONDS
//!
//! Exits 0 if every step succeeded, 1 if not.

use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use shared::ipc::control::StatusInfo;
use tokio::sync::mpsc;

use crate::backend::{schedule, supervisor};

/// `Some(exit code)` if this run is a probe (`--probe` on the command line).
pub fn run_if_requested() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != "--probe" {
        return None;
    }
    let (Some(script), Some(log)) = (args.next(), args.next()) else {
        eprintln!("usage: lewdware --probe SCRIPT LOG");
        return Some(2);
    };
    Some(match run(Path::new(&script), Path::new(&log)) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(error) => {
            eprintln!("probe: {error:#}");
            2
        }
    })
}

fn run(script: &Path, log: &Path) -> Result<bool> {
    let script = std::fs::read_to_string(script).context("couldn't read the script")?;
    let mut log = Log(std::fs::File::create(log).context("couldn't create the log")?);
    log.line(&format!(
        "probe {} as {:?}",
        shared::VERSION,
        std::env::current_exe().ok()
    ));

    let runtime = tokio::runtime::Runtime::new()?;
    let mut ok = true;
    for step in script.lines().map(str::trim) {
        if step.is_empty() || step.starts_with('#') {
            continue;
        }
        log.line(&format!("> {step}"));
        let words: Vec<&str> = step.split_whitespace().collect();
        let result = match words.as_slice() {
            ["permission"] => Ok(format!("granted: {}", shared::input_monitoring::granted())),
            ["open-settings"] => shared::input_monitoring::open_settings()
                .map(|()| "opened".into())
                .map_err(Into::into),
            _ => runtime.block_on(async_step(&words, &mut log)),
        };
        match result {
            Ok(message) => log.line(&format!("  ok: {message}")),
            Err(error) => {
                ok = false;
                log.line(&format!("  FAILED: {error:#}"));
            }
        }
    }
    log.line(if ok {
        "all steps passed"
    } else {
        "some steps failed"
    });
    Ok(ok)
}

async fn async_step(words: &[&str], log: &mut Log) -> Result<String> {
    match words {
        ["status"] => Ok(match supervisor::status().await {
            Some(info) => describe(&info),
            None => "no supervisor running".into(),
        }),
        ["launch"] => supervisor::launch().await.map(|()| "launched".into()),
        ["request-permission"] => supervisor::ask_for_input_monitoring()
            .await
            .map(|()| "asked the supervisor (and opened settings if not granted)".into()),
        ["stop"] => {
            supervisor::stop().await;
            Ok("asked to stop".into())
        }
        ["follow", seconds] => {
            follow(seconds.parse()?, None, log).await?;
            Ok("done".into())
        }
        ["wait-shortcut", kind, seconds] => {
            if follow(seconds.parse()?, Some(kind), log).await? {
                Ok(format!("stop shortcut is {kind}"))
            } else {
                bail!("stop shortcut never became {kind}")
            }
        }
        ["autostart", on @ ("on" | "off")] => {
            let enable = *on == "on";
            tokio::task::spawn_blocking(move || schedule::set_autostart(enable)).await??;
            Ok(format!("autostart {on}"))
        }
        ["sleep", seconds] => {
            tokio::time::sleep(Duration::from_secs_f64(seconds.parse()?)).await;
            Ok("slept".into())
        }
        _ => Err(anyhow!("unknown step")),
    }
}

/// Follows the status like an open window, logging changes. With `until`, stops early once the
/// stop shortcut's status is that kind, and says whether it got there.
async fn follow(seconds: f64, until: Option<&str>, log: &mut Log) -> Result<bool> {
    let (sender, mut updates) = mpsc::unbounded_channel();
    let follower = tokio::spawn(supervisor::follow(sender));
    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(seconds);
    let mut last = None;
    let mut reached = false;
    while let Ok(Some(info)) = tokio::time::timeout_at(deadline, updates.recv()).await {
        let described = info
            .as_ref()
            .map_or("no supervisor running".into(), describe);
        if last.as_ref() != Some(&described) {
            log.line(&format!("  status: {described}"));
            last = Some(described);
        }
        if let (Some(kind), Some(info)) = (until, &info) {
            if shortcut_kind(info) == kind {
                reached = true;
                break;
            }
        }
    }
    follower.abort();
    Ok(reached)
}

fn shortcut_kind(info: &StatusInfo) -> String {
    let debug = format!("{:?}", info.stop_shortcut);
    debug
        .split([' ', '{', '('])
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn describe(info: &StatusInfo) -> String {
    let session = format!("{:?}", info.session);
    let session = session.split([' ', '{']).next().unwrap_or_default();
    format!("session {session}, stop shortcut {:?}", info.stop_shortcut)
}

struct Log(std::fs::File);

impl Log {
    fn line(&mut self, text: &str) {
        let time = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(self.0, "{time} {text}");
        let _ = self.0.flush();
        eprintln!("{text}");
    }
}
