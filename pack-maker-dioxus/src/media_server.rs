use std::{net::SocketAddr, rc::Rc};

use anyhow::{anyhow, bail, Result};
use http_body_util::Full;
use hyper::{
    body::Bytes,
    header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE},
    server::conn::http1,
    service::service_fn,
    Request, Response,
};
use hyper_util::rt::TokioIo;
use shared::encode::FileType;
use tokio::{net::TcpListener, runtime, sync::oneshot, task::LocalSet};

use crate::pack::MediaPackView;

pub async fn start_media_server(media_pack: MediaPackView) -> anyhow::Result<u16> {
    let (port_tx, port_rx) = oneshot::channel();

    std::thread::spawn(move || {
        let rt = runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .expect("Creating tokio runtime failed");

        rt.block_on(async move {
            if let Err(err) = media_server(media_pack, port_tx).await {
                eprintln!("{err}");
            }
        });
    });

    port_rx.await.map_err(|err| err.into())
}

async fn media_server(media_pack: MediaPackView, port_tx: oneshot::Sender<u16>) -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 0));

    let listener = TcpListener::bind(addr).await?;

    let port = listener.local_addr()?.port();

    port_tx
        .send(port)
        .map_err(|_| anyhow!("Port sender closed"))?;

    println!("Started listener on {port}");

    let media_pack = Rc::new(media_pack);

    let local_set = LocalSet::new();

    local_set.run_until(async move {
        loop {
            let (stream, _) = listener.accept().await?;

            println!("Received connection");

            let io = TokioIo::new(stream);

            let media_pack = media_pack.clone();

            tokio::task::spawn_local(async move {
                if let Err(err) = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |request| {
                            println!("Received request: {:?}", request);
                            let media_pack = media_pack.clone();
                            async move {
                                match media_protocol_handler(media_pack, request).await {
                                    Ok(response) => Ok(response),
                                    Err(err) => {
                                        eprintln!("{err}");

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
    }).await

}

pub async fn media_protocol_handler(
    media_pack: Rc<MediaPackView>,
    request: Request<hyper::body::Incoming>,
) -> anyhow::Result<Response<Full<Bytes>>> {
    let path = request.uri().path()[1..].to_string();

    let mut parts = path.split("/");

    match parts.next() {
        Some("thumbnail") => {
            println!("Received thumbnail request");
            let id: u64 = parts
                .next()
                .ok_or_else(|| anyhow!("Missing path"))?
                .parse()?;

            let thumbnail = media_pack.get_thumbnail(id).await?;

            Response::builder()
                .header(CONTENT_TYPE, "image/png")
                .header("Access-Control-Allow-Origin", "*")
                .body(Full::new(Bytes::from(thumbnail)))
                .map_err(|err| err.into())
        }
        Some("preview") => {
            let id: u64 = parts
                .next()
                .ok_or_else(|| anyhow!("Missing path"))?
                .parse()?;

            let thumbnail = media_pack.get_preview(id).await?;

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

                let (file_data, file_type) = media_pack.get_file_range(id, range).await?;

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
                let (data, file_type) = media_pack.get_file_data(id).await?;

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
        FileType::Video => Ok("video/webm"),
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
