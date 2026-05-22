use axum::{
    Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use tokio::net::TcpListener;

use crate::{pack::Range, PackState};

pub async fn start(pack_state: PackState) -> anyhow::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let router = Router::new()
        .route("/thumbnail/{id}", get(thumbnail_handler))
        .route("/preview/{id}", get(preview_handler))
        .route("/file/{id}", get(file_handler))
        .with_state(pack_state);

    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });

    Ok(port)
}

async fn thumbnail_handler(
    State(pack_state): State<PackState>,
    Path(id): Path<u64>,
) -> Response {
    let view = {
        let lock = pack_state.lock().await;
        match lock.as_ref() {
            Some(pack) => match pack.get_view() {
                Ok(v) => v,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            },
            None => return (StatusCode::NOT_FOUND, "No pack open").into_response(),
        }
    };
    match view.get_thumbnail(id).await {
        Ok(data) => Response::builder()
            .status(200)
            .header("Content-Type", "image/webp")
            .header("Access-Control-Allow-Origin", "*")
            .body(axum::body::Body::from(data))
            .unwrap(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn preview_handler(
    State(pack_state): State<PackState>,
    Path(id): Path<u64>,
) -> Response {
    let view = {
        let lock = pack_state.lock().await;
        match lock.as_ref() {
            Some(pack) => match pack.get_view() {
                Ok(v) => v,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            },
            None => return (StatusCode::NOT_FOUND, "No pack open").into_response(),
        }
    };
    match view.get_preview(id).await {
        Ok(data) => Response::builder()
            .status(200)
            .header("Content-Type", "image/jpeg")
            .header("Access-Control-Allow-Origin", "*")
            .body(axum::body::Body::from(data))
            .unwrap(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn file_handler(
    State(pack_state): State<PackState>,
    Path(id): Path<u64>,
    request_headers: HeaderMap,
) -> Response {
    let view = {
        let lock = pack_state.lock().await;
        match lock.as_ref() {
            Some(pack) => match pack.get_view() {
                Ok(v) => v,
                Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
            },
            None => return (StatusCode::NOT_FOUND, "No pack open").into_response(),
        }
    };

    let range_str = request_headers
        .get("Range")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    if let Some(range_str) = range_str {
        let range = match parse_range(&range_str) {
            Ok(r) => r,
            Err(()) => {
                return (StatusCode::RANGE_NOT_SATISFIABLE, "Invalid range").into_response()
            }
        };
        match view.get_file_range(id, range).await {
            Ok((dr, ft)) => {
                let ct = file_type_mime(ft);
                Response::builder()
                    .status(206)
                    .header("Content-Type", ct)
                    .header("Accept-Ranges", "bytes")
                    .header("Access-Control-Allow-Origin", "*")
                    .header(
                        "Content-Range",
                        format!("bytes {}-{}/{}", dr.start, dr.end - 1, dr.total_size),
                    )
                    .header("Content-Length", dr.end - dr.start)
                    .body(axum::body::Body::from(dr.data))
                    .unwrap()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    } else {
        match view.get_file_data(id).await {
            Ok((data, ft)) => {
                let ct = file_type_mime(ft);
                Response::builder()
                    .status(200)
                    .header("Content-Type", ct)
                    .header("Accept-Ranges", "bytes")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(axum::body::Body::from(data))
                    .unwrap()
            }
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        }
    }
}

fn file_type_mime(ft: shared::encode::FileType) -> &'static str {
    match ft {
        shared::encode::FileType::Image => "image/avif",
        shared::encode::FileType::Video => "video/webm",
        shared::encode::FileType::Audio => "audio/ogg",
    }
}

fn parse_range(s: &str) -> Result<Range, ()> {
    let value = s.strip_prefix("bytes=").ok_or(())?;
    let mut parts = value.split('-');
    let start_str = parts.next().ok_or(())?;
    let end_str = parts.next().ok_or(())?;
    let start = if start_str.is_empty() { None } else { start_str.parse().ok() };
    let end = if end_str.is_empty() { None } else { end_str.parse().ok() };
    Ok(Range { start, end })
}
