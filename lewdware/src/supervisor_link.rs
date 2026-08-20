use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};

use shared::ipc::engine::{EngineLink, EngineToSupervisor, SupervisorToEngine};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

static REPORT_TX: OnceLock<UnboundedSender<EngineToSupervisor>> = OnceLock::new();

/// Connects to the supervisor's engine-link socket and identifies this session via `token`
/// (the episode sequence number the supervisor passed via `--control-token`). Spawns a
/// background thread hosting a small tokio runtime that drains outgoing reports (see [`report`])
/// and watches for an incoming [`SupervisorToEngine::Stop`], forwarded through the returned
/// receiver.
///
/// Runs before the winit `EventLoop`/proxy exist (so even a `load_config()` failure can still be
/// reported via [`report`]) -- the caller wires the returned receiver to the proxy once it does
/// exist. `token: None` (a direct, unsupervised invocation -- no longer a supported path, see
/// `main.rs`) makes this a no-op: [`report`] silently drops every message, and the returned
/// receiver never fires.
pub fn connect(token: Option<String>) -> Receiver<()> {
    let (stop_tx, stop_rx) = channel();

    let Some(token) = token else {
        return stop_rx;
    };

    let (report_tx, report_rx) = unbounded_channel::<EngineToSupervisor>();
    let _ = REPORT_TX.set(report_tx);

    std::thread::spawn(move || run(token, stop_tx, report_rx));

    stop_rx
}

pub fn report(msg: EngineToSupervisor) {
    if let Some(tx) = REPORT_TX.get() {
        let _ = tx.send(msg);
    }
}

fn run(
    token: String,
    stop_tx: Sender<()>,
    mut report_rx: tokio::sync::mpsc::UnboundedReceiver<EngineToSupervisor>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            tracing::warn!("failed to build supervisor-link runtime: {err}");
            return;
        }
    };

    rt.block_on(async move {
        let EngineLink { recv, mut send } = match shared::ipc::engine::connect_engine_link(&token).await {
            Ok(link) => link,
            Err(err) => {
                tracing::warn!("failed to connect to the supervisor: {err}");
                return;
            }
        };

        tokio::spawn(watch_for_stop(recv, stop_tx));

        while let Some(report) = report_rx.recv().await {
            let Ok(mut line) = serde_json::to_string(&report) else {
                continue;
            };
            line.push('\n');
            if send.write_all(line.as_bytes()).await.is_err() {
                return;
            }
        }
    });
}

async fn watch_for_stop(recv: shared::ipc::RecvHalf, stop_tx: Sender<()>) {
    let mut reader = BufReader::new(recv);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }

        if let Ok(SupervisorToEngine::Stop) = serde_json::from_str(line.trim_end()) {
            let _ = stop_tx.send(());
            return;
        }
    }
}
