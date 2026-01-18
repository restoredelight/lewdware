use std::net::SocketAddr;

use anyhow::{anyhow, bail, Result};
use http_body_util::{Empty, Full};
use hyper::{
    body::Bytes,
    header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE},
    server::conn::http1,
    service::service_fn,
    Request, Response,
};
use hyper_util::rt::TokioIo;
use shared::encode::FileType;
use tauri::{async_runtime, AppHandle, Manager};
use tokio::{net::TcpListener, sync::oneshot};

use crate::{pack, thumbnail::generate_thumbnail, State};

pub async fn start_media_server(
    app_handle: AppHandle,
    port_tx: oneshot::Sender<u16>,
) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 0));

    let listener = TcpListener::bind(addr).await?;

    let port = listener.local_addr()?.port();

    let _ = port_tx.send(port);

    loop {
        let (stream, _) = listener.accept().await?;

        let io = TokioIo::new(stream);
        let app_handle = app_handle.clone();

        async_runtime::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(|request| {
                        let app_handle = app_handle.clone();
                        async move { 
                            match media_protocol_handler(app_handle, request).await {
                                Ok(response) => Ok(response),
                                Err(err) => {
                                    let response = Response::builder()
                                        .status(500)
                                        .header(CONTENT_TYPE, "text/plain")
                                        .body(Full::new(Bytes::from(err.to_string().into_bytes())));

                                    response
                                }
                            }
                        }
                    }),
                )
                .await
            {
                eprintln!("Error serving connection: {:?}", err);
            }
        });
    }
}

pub async fn media_protocol_handler(
    app_handle: AppHandle,
    request: Request<hyper::body::Incoming>,
) -> anyhow::Result<Response<Full<Bytes>>> {
    let path = request.uri().path()[1..].to_string();

    let mut parts = path.split("/");

    let state: State<'_> = app_handle.state();

    match parts.next() {
        Some("thumbnail") => {
            let id: u64 = parts
                .next()
                .ok_or_else(|| anyhow!("Missing path"))?
                .parse()?;

            let (file_data, file_type) = {
                let manager = state.pack.read().await;
                let manager = manager.as_ref().unwrap();

                manager.get_file(id).await?
            };

            let thumbnail =
                generate_thumbnail(app_handle, file_data, file_type == FileType::Image, false)
                    .await?;

            Response::builder()
                .header(CONTENT_TYPE, "image/png")
                .header("Access-Control-Allow-Origin", "*")
                .body(Full::new(Bytes::from(thumbnail)))
                .map_err(|err| err.into())
        }
        Some("big-thumbnail") => {
            let id: u64 = parts
                .next()
                .ok_or_else(|| anyhow!("Missing path"))?
                .parse()?;

            let (file_data, file_type) = {
                let manager = state.pack.read().await;
                let manager = manager.as_ref().unwrap();

                manager
                    .get_file(id)
                    .await?
            };

            let thumbnail =
                generate_thumbnail(app_handle, file_data, file_type == FileType::Image, true)
                    .await?;

            Response::builder()
                .header(CONTENT_TYPE, "image/png")
                .header("Access-Control-Allow-Origin", "*")
                .body(Full::new(Bytes::from(thumbnail)))
                .map_err(|err| err.into())
        }
        Some("file") => {
            let id: u64 = parts
                .next()
                .ok_or_else(|| anyhow!("Missing path"))?
                .parse()?;

            let range_header = request.headers().get(RANGE).and_then(|h| h.to_str().ok());

            let response_builder = Response::builder()
                .header(ACCEPT_RANGES, "bytes")
                .header("Access-Control-Allow-Origin", "*");

            if let Some(range_header) = range_header {
                let range = parse_range(range_header)?;

                let (file_data, file_type) = {
                    let manager = state.pack.read().await;
                    let manager = manager.as_ref().unwrap();

                    manager.get_file_range(id, range).await?
                };

                let content_type = get_content_type(file_type)?;

                response_builder
                    .status(206)
                    .header(CONTENT_TYPE, content_type)
                    .header(CONTENT_LENGTH, file_data.end - file_data.start)
                    .header(
                        CONTENT_RANGE,
                        format!(
                            "bytes {}-{}/{}",
                            file_data.start,
                            file_data.end - 1,
                            file_data.total_size
                        ),
                    )
                    .body(Full::new(Bytes::from(file_data.data)))
                    .map_err(|err| err.into())
            } else {
                let (file_data, file_type) = {
                    let manager = state.pack.read().await;
                    let manager = manager.as_ref().unwrap();

                    manager
                        .get_file(id)
                        .await?
                };

                let data = match file_data {
                    pack::FileData::Path(path) => tokio::fs::read(path).await?,
                    pack::FileData::Data(data) => data,
                };

                let content_type = get_content_type(file_type)?;

                response_builder
                    .status(200)
                    .header(CONTENT_TYPE, content_type)
                    .body(Full::new(Bytes::from(data)))
                    .map_err(|err| err.into())
            }
        }
        _ => bail!("Invalid path"),
    }
}

#[derive(Debug)]
pub struct Range {
    pub start: Option<u64>,
    pub end: Option<u64>,
}

fn get_content_type(file_type: FileType) -> Result<&'static str> {
    match file_type {
        FileType::Image => Ok("image/avif"),
        FileType::Video => Ok("video/webm; codecs=\"vp8,opus\""),
        FileType::Audio => Ok("audio/opus"),
    }
}

fn parse_range(range: &str) -> Result<Range> {
    if let Some(value) = range.strip_prefix("bytes=") {
        let parts: Vec<&str> = value.split('-').collect();
        if parts.len() == 2 {
            let start_str = parts[0].trim();
            let end_str = parts[1].trim();

            let start = if start_str.is_empty() {
                None
            } else {
                Some(start_str.parse()?)
            };

            let end = if end_str.is_empty() {
                None
            } else {
                Some(end_str.parse()?)
            };

            return Ok(Range { start, end });
        }
    }

    bail!("Invalid range");
}
