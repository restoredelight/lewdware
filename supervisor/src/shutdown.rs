//! Noticing that we are being stopped, and telling the schedule engine who stopped us.
//!
//! The engine learns the user's week from the gaps it cannot see through (`state.rs`), so "the
//! supervisor was not running" has to come with a reason attached. Two of them are ours to know
//! for certain -- the idle self-terminate and scheduling being switched off both go through
//! `Control` -- and this module covers the third: the session or the machine going away under us.
//!
//! | | signal | recorded as |
//! |---|---|---|
//! | logout, shutdown, reboot | `SIGTERM`, `SIGHUP` / `CTRL_LOGOFF`, `CTRL_SHUTDOWN` | [`LastStop::System`] |
//! | somebody stopping the supervisor by hand | `SIGINT` / `CTRL_C`, `CTRL_BREAK`, `CTRL_CLOSE` | [`LastStop::Supervisor`] |
//!
//! Missing the signal is survivable rather than wrong: an unrecorded stop reads as absence, which
//! is the right answer for the causes that leave no chance to record anything (a power cut, a
//! `SIGKILL`) and for a logout we failed to hook. The Windows handlers in particular only fire
//! for a process with a console, and the supervisor is normally spawned without one -- so there
//! the machine-going-away case usually lands on that fallback, arriving at the same reading by a
//! duller route.

use std::time::Duration;

use tokio::sync::mpsc;

use crate::control::ControlMessage;
use crate::state::LastStop;

/// How long `Control` gets to write the record before the process leaves without it.
const GRACE: Duration = Duration::from_secs(5);

pub fn spawn(control_tx: mpsc::Sender<ControlMessage>) {
    platform::spawn(control_tx);
}

/// Hands the stop to `Control`, which records it and exits the process itself.
async fn report(control_tx: mpsc::Sender<ControlMessage>, stop: LastStop) {
    tracing::info!("stopping: {stop:?}");
    let _ = control_tx.send(ControlMessage::Stopping { stop }).await;

    // The backstop for a control loop too busy to get there. The OS is already counting down to a
    // hard kill, and leaving a second late without the record helps nobody.
    tokio::time::sleep(GRACE).await;
    tracing::warn!("the control loop did not act on the stop; exiting anyway");
    std::process::exit(0);
}

#[cfg(unix)]
mod platform {
    use tokio::signal::unix::{SignalKind, signal};
    use tokio::sync::mpsc;

    use crate::control::ControlMessage;
    use crate::state::LastStop;

    pub fn spawn(control_tx: mpsc::Sender<ControlMessage>) {
        // `SIGTERM` is what a logout, a shutdown and a `systemctl stop` all arrive as, and
        // `SIGHUP` is the session's own end. `SIGINT` is a person at a terminal, which is a very
        // different thing: the machine is still there, and so, presumably, is its user.
        watch(SignalKind::terminate(), LastStop::System, &control_tx);
        watch(SignalKind::hangup(), LastStop::System, &control_tx);
        watch(SignalKind::interrupt(), LastStop::Supervisor, &control_tx);
    }

    fn watch(kind: SignalKind, stop: LastStop, control_tx: &mpsc::Sender<ControlMessage>) {
        let mut stream = match signal(kind) {
            Ok(stream) => stream,
            Err(err) => {
                // Not fatal: without the handler the stop is simply unrecorded, which is a
                // reading, just a blunter one.
                tracing::warn!("could not listen for {kind:?}: {err}");
                return;
            }
        };
        let control_tx = control_tx.clone();
        tokio::spawn(async move {
            if stream.recv().await.is_some() {
                super::report(control_tx, stop).await;
            }
        });
    }
}

#[cfg(windows)]
mod platform {
    use tokio::sync::mpsc;

    use crate::control::ControlMessage;
    use crate::state::LastStop;

    pub fn spawn(control_tx: mpsc::Sender<ControlMessage>) {
        macro_rules! watch {
            ($ctrl:path, $stop:expr) => {{
                let control_tx = control_tx.clone();
                tokio::spawn(async move {
                    match $ctrl() {
                        Ok(mut stream) => {
                            if stream.recv().await.is_some() {
                                super::report(control_tx, $stop).await;
                            }
                        }
                        // See the module docs: a console-less supervisor never registers these,
                        // and falls back to an unrecorded stop.
                        Err(err) => tracing::warn!("could not listen for a console event: {err}"),
                    }
                });
            }};
        }

        watch!(tokio::signal::windows::ctrl_shutdown, LastStop::System);
        watch!(tokio::signal::windows::ctrl_logoff, LastStop::System);
        watch!(tokio::signal::windows::ctrl_c, LastStop::Supervisor);
        watch!(tokio::signal::windows::ctrl_break, LastStop::Supervisor);
        watch!(tokio::signal::windows::ctrl_close, LastStop::Supervisor);
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use tokio::sync::mpsc;

    use crate::control::ControlMessage;

    pub fn spawn(_control_tx: mpsc::Sender<ControlMessage>) {}
}
