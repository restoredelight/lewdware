use anyhow::Result;
use shared::ipc::control::{Request, Response, StatusInfo, control_socket_name};
use shared::ipc::{Stream, bind_listener, prelude::*};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use uuid::Uuid;

type DevLog = (Uuid, shared::logging::LogRecord);

use crate::control::ControlMessage;

/// Accepts connections from the CLI client, the config app, and `lw`: one-shot
/// request/response, except `Subscribe`, which holds the connection open as a status stream.
pub async fn run(
    control_tx: mpsc::Sender<ControlMessage>,
    status_rx: watch::Receiver<StatusInfo>,
    dev_log_rx: broadcast::Receiver<DevLog>,
) -> Result<()> {
    let name = control_socket_name()?;
    let listener = bind_listener(name)?;

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
        let dev_log_rx = dev_log_rx.resubscribe();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(conn, control_tx, status_rx, dev_log_rx).await {
                tracing::warn!("ipc connection error: {err}");
            }
        });
    }
}

async fn handle_connection(
    conn: Stream,
    control_tx: mpsc::Sender<ControlMessage>,
    status_rx: watch::Receiver<StatusInfo>,
    dev_log_rx: broadcast::Receiver<DevLog>,
) -> Result<()> {
    let mut reader = BufReader::new(&conn);
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        anyhow::bail!("connection closed before a request was sent");
    }

    let req: Request = serde_json::from_str(line.trim_end())?;

    match &req {
        Request::SubscribeStatus => {
            let _ = control_tx.send(ControlMessage::StatusSubscribed).await;
            let result = stream_status(conn, status_rx).await;
            let _ = control_tx.send(ControlMessage::StatusUnsubscribed).await;
            return result;
        }
        Request::SubscribeDevLogs { stream_id } => {
            return stream_dev_logs(conn, dev_log_rx, *stream_id).await;
        }
        _ => {}
    }

    let (respond_to, response) = oneshot::channel();
    control_tx
        .send(ControlMessage::Request { req, respond_to })
        .await?;
    let response: Response = response.await?;

    write_response(&conn, &response).await?;

    Ok(())
}

async fn stream_dev_logs(
    conn: Stream,
    mut dev_log_rx: broadcast::Receiver<DevLog>,
    stream_id: Uuid,
) -> Result<()> {
    write_response(&conn, &Response::DevLogReady).await?;
    loop {
        match dev_log_rx.recv().await {
            Ok((record_stream_id, record)) if record_stream_id == stream_id => {
                if write_response(&conn, &Response::DevLog { record })
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

/// Writes the current status immediately, then again on every change, until the subscriber
/// disconnects. A write failure just means the subscriber went away -- not an error worth logging.
///
/// Also watches for the subscriber closing its end, so that it's noticed when it happens rather
/// than at the next status change: the supervisor stays alive while anyone is subscribed.
async fn stream_status(conn: Stream, mut status_rx: watch::Receiver<StatusInfo>) -> Result<()> {
    let mut closed = std::pin::pin!(async {
        // A subscriber sends nothing after its request, so any read result means it's gone.
        let mut byte = [0u8; 1];
        let _ = (&conn).read(&mut byte).await;
    });
    loop {
        let status = status_rx.borrow_and_update().clone();
        if write_response(&conn, &Response::Status(status))
            .await
            .is_err()
        {
            return Ok(());
        }
        tokio::select! {
            changed = status_rx.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
            }
            () = &mut closed => return Ok(()),
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
