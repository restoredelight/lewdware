use anyhow::Result;
use shared::ipc::{self, Request, Response, StatusInfo, Stream, prelude::*};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

use crate::control::ControlMessage;

/// Accepts connections from the CLI client, the config app, and `lw`: one-shot
/// request/response, except `Subscribe`, which holds the connection open as a status stream.
pub async fn run(
    control_tx: mpsc::Sender<ControlMessage>,
    status_rx: watch::Receiver<StatusInfo>,
) -> Result<()> {
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
        let status_rx = status_rx.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(conn, control_tx, status_rx).await {
                tracing::warn!("ipc connection error: {err}");
            }
        });
    }
}

async fn handle_connection(
    conn: Stream,
    control_tx: mpsc::Sender<ControlMessage>,
    status_rx: watch::Receiver<StatusInfo>,
) -> Result<()> {
    let mut reader = BufReader::new(&conn);
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        anyhow::bail!("connection closed before a request was sent");
    }

    let req: Request = serde_json::from_str(line.trim_end())?;

    if matches!(req, Request::Subscribe) {
        return stream_status(conn, status_rx).await;
    }

    let (respond_to, response) = oneshot::channel();
    control_tx
        .send(ControlMessage::Request { req, respond_to })
        .await?;
    let response: Response = response.await?;

    write_response(&conn, &response).await?;

    Ok(())
}

/// Writes the current status immediately, then again on every change, until the subscriber
/// disconnects. A write failure just means the subscriber went away -- not an error worth logging.
async fn stream_status(conn: Stream, mut status_rx: watch::Receiver<StatusInfo>) -> Result<()> {
    loop {
        let status = status_rx.borrow_and_update().clone();
        if write_response(&conn, &Response::Status(status)).await.is_err() {
            return Ok(());
        }
        if status_rx.changed().await.is_err() {
            return Ok(());
        }
    }
}

async fn write_response(conn: &Stream, response: &Response) -> Result<()> {
    let mut sender = conn;
    let mut encoded = serde_json::to_string(response)?;
    encoded.push('\n');
    sender.write_all(encoded.as_bytes()).await?;
    Ok(())
}
