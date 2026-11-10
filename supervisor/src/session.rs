use std::time::Duration;

use anyhow::Result;
use shared::ipc::engine::{EngineToSupervisor, SupervisorToEngine};
use shared::ipc::{RecvHalf, SendHalf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::control::ControlMessage;

const GRACE_PERIOD: Duration = Duration::from_secs(5);

pub enum SessionCommand {
    /// Ask the engine to stop via the engine-link connection, escalating to a hard kill if it
    /// doesn't exit within the grace period.
    Terminate,
    /// Hard-kill immediately, with no grace period (the global stop shortcut).
    Kill,
    /// Handed off by `engine_link` once the spawned engine's `Hello` has been matched to this
    /// session.
    AttachLink { recv: RecvHalf, send: SendHalf },
}

/// What actually happened when the child exited -- `control.rs` (not this module) decides what
/// it *means* (crash-restart vs. graceful or immediate stop), since that policy depends on the
/// `Episode`'s own `intent`, which only `control.rs` tracks.
pub struct SessionExit {
    pub status: Option<std::process::ExitStatus>,
    /// True iff this exit followed a `Terminate`/`Kill` we sent -- i.e. not a spontaneous crash.
    pub we_commanded_stop: bool,
}

impl SessionExit {
    pub fn mode_completed(&self) -> bool {
        self.status.and_then(|s| s.code())
            == Some(i32::from(shared::ipc::engine::MODE_COMPLETED_EXIT_CODE))
    }
}

/// Spawns the child process and its supervising task. Returns the pid and a channel for sending
/// it `SessionCommand`s.
pub fn spawn(
    seq: u64,
    mut cmd: tokio::process::Command,
    control_tx: mpsc::Sender<ControlMessage>,
) -> Result<(u32, mpsc::Sender<SessionCommand>)> {
    let mut child = cmd.spawn()?;
    let pid = child
        .id()
        .ok_or_else(|| anyhow::anyhow!("child exited before its pid could be read"))?;

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<SessionCommand>(8);

    tokio::spawn(async move {
        let mut engine_send: Option<SendHalf> = None;
        let mut reader_task: Option<JoinHandle<()>> = None;

        loop {
            tokio::select! {
                status = child.wait() => {
                    if let Some(rt) = reader_task.take() {
                        rt.abort();
                    }
                    let _ = control_tx
                        .send(ControlMessage::SessionExited {
                            seq,
                            exit: SessionExit { status: status.ok(), we_commanded_stop: false },
                        })
                        .await;
                    return;
                }
                Some(command) = cmd_rx.recv() => {
                    match command {
                        SessionCommand::AttachLink { recv, send } => {
                            engine_send = Some(send);
                            let tx = control_tx.clone();
                            reader_task = Some(tokio::spawn(read_reports(seq, recv, tx)));
                        }
                        SessionCommand::Terminate | SessionCommand::Kill => {
                            let immediate = matches!(command, SessionCommand::Kill);

                            if !immediate && let Some(send) = engine_send.as_mut() {
                                let _ = send_stop(send).await;
                            }

                            let grace = if immediate { Duration::ZERO } else { GRACE_PERIOD };
                            let status = tokio::select! {
                                s = child.wait() => s,
                                _ = tokio::time::sleep(grace) => {
                                    let _ = child.start_kill();
                                    child.wait().await
                                }
                            };

                            if let Some(rt) = reader_task.take() {
                                rt.abort();
                            }

                            let _ = control_tx
                                .send(ControlMessage::SessionExited {
                                    seq,
                                    exit: SessionExit { status: status.ok(), we_commanded_stop: true },
                                })
                                .await;
                            return;
                        }
                    }
                }
            }
        }
    });

    Ok((pid, cmd_tx))
}

async fn send_stop(send: &mut SendHalf) -> Result<()> {
    let mut line = serde_json::to_string(&SupervisorToEngine::Stop)?;
    line.push('\n');
    send.write_all(line.as_bytes()).await?;
    Ok(())
}

/// Reads `EngineToSupervisor` reports off the engine-link connection for the lifetime of the
/// session, forwarding each as a `ControlMessage::EngineReported`. Aborted from the outside (via
/// the `JoinHandle`) once the session ends, so no explicit exit condition is needed here beyond
/// the connection closing.
async fn read_reports(seq: u64, recv: RecvHalf, control_tx: mpsc::Sender<ControlMessage>) {
    let mut reader = BufReader::new(recv);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }

        let Ok(report) = serde_json::from_str::<EngineToSupervisor>(line.trim_end()) else {
            continue;
        };

        if control_tx
            .send(ControlMessage::EngineReported { seq, report })
            .await
            .is_err()
        {
            return;
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    /// Run stateful regressions with a private D-Bus and private settings/data directories.
    /// They cannot connect to the user's desktop or delete a real wallpaper recovery snapshot.
    #[test]
    fn desktop_and_lifecycle_regressions() {
        let dir = tempfile::tempdir().unwrap();
        // The host may be KDE and have no GNOME schema package. Supply the relevant public
        // schema types/defaults so the real gsettings/dconf integration remains portable.
        let schema = dir.path().join("org.gnome.test.gschema.xml");
        std::fs::write(&schema, r#"<schemalist>
          <schema id="org.gnome.settings-daemon.plugins.media-keys" path="/org/gnome/settings-daemon/plugins/media-keys/">
            <key name="custom-keybindings" type="as"><default>[]</default></key>
          </schema>
          <schema id="org.gnome.settings-daemon.plugins.media-keys.custom-keybinding">
            <key name="name" type="s"><default>''</default></key>
            <key name="command" type="s"><default>''</default></key>
            <key name="binding" type="s"><default>''</default></key>
          </schema>
        </schemalist>"#).unwrap();
        assert!(
            std::process::Command::new("glib-compile-schemas")
                .arg(dir.path())
                .status()
                .unwrap()
                .success()
        );
        let output = std::process::Command::new("dbus-run-session")
            .arg("--")
            .arg(std::env::current_exe().unwrap())
            .args([
                "isolated_regression",
                "--ignored",
                "--test-threads=1",
                "--nocapture",
            ])
            .env("LEWDWARE_ISOLATED_TEST", "1")
            .env("GSETTINGS_SCHEMA_DIR", dir.path())
            .env("XDG_DATA_HOME", dir.path().join("data"))
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("XAUTHORITY")
            .env("GSETTINGS_BACKEND", "dconf")
            .env("XDG_CURRENT_DESKTOP", "GNOME")
            .env("XDG_SESSION_TYPE", "wayland")
            .status()
            .expect("dbus-run-session is required for desktop regression tests");
        assert!(
            output.success(),
            "isolated desktop regressions failed: {output}"
        );
    }
}
