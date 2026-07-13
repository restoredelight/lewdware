use anyhow::Result;
use shared::ipc::{self, Request, Response, Stream, prelude::*};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::control::ControlMessage;

/// Accepts one-shot request/response connections from the CLI client, the config app, and `lw`.
pub async fn run(control_tx: mpsc::Sender<ControlMessage>) -> Result<()> {
    let name = ipc::control_socket_name()?;
    let listener = ipc::bind_listener(name)?;

    loop {
        let conn = match listener.accept().await {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!("ipc accept error: {err}");
                continue;
            }
        };

        let control_tx = control_tx.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(conn, control_tx).await {
                tracing::warn!("ipc connection error: {err}");
            }
        });
    }
}

async fn handle_connection(conn: Stream, control_tx: mpsc::Sender<ControlMessage>) -> Result<()> {
    let mut reader = BufReader::new(&conn);
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        anyhow::bail!("connection closed before a request was sent");
    }

    let req: Request = serde_json::from_str(line.trim_end())?;

    let (respond_to, response) = oneshot::channel();
    control_tx
        .send(ControlMessage::Request { req, respond_to })
        .await?;
    let response: Response = response.await?;

    let mut sender = &conn;
    let mut encoded = serde_json::to_string(&response)?;
    encoded.push('\n');
    sender.write_all(encoded.as_bytes()).await?;

    Ok(())
}
