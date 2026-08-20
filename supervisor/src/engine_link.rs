use anyhow::Result;
use shared::ipc::engine::{EngineToSupervisor, engine_link_socket_name};
use shared::ipc::{Stream, bind_listener, prelude::*};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::control::ControlMessage;

/// Accepts connections from spawned engines. Each engine connects out (it's always the client;
/// the supervisor is always the resident server), sends `Hello{token}` to identify which
/// episode it belongs to, and is then handed off to that episode's `session.rs` task -- this
/// module's job stops at the handshake.
pub async fn run(control_tx: mpsc::Sender<ControlMessage>) -> Result<()> {
    let name = engine_link_socket_name()?;
    let listener = bind_listener(name)?;

    loop {
        let conn = match listener.accept().await {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!("engine-link accept error: {err}");
                continue;
            }
        };

        let control_tx = control_tx.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(conn, control_tx).await {
                tracing::warn!("engine-link connection error: {err}");
            }
        });
    }
}

async fn handle_connection(conn: Stream, control_tx: mpsc::Sender<ControlMessage>) -> Result<()> {
    let (recv, send) = conn.split();
    let mut reader = BufReader::new(recv);

    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        anyhow::bail!("connection closed before Hello");
    }

    let seq = match serde_json::from_str::<EngineToSupervisor>(line.trim_end())? {
        EngineToSupervisor::Hello { token } => token.parse::<u64>()?,
        _ => anyhow::bail!("expected Hello as the first message on the engine link"),
    };

    let recv = reader.into_inner();
    control_tx
        .send(ControlMessage::EngineConnected { seq, recv, send })
        .await?;

    Ok(())
}
