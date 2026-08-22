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
