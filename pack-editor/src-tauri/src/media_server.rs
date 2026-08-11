use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::{
    pack::{InvalidRange, Range},
    PackState,
};

#[derive(Clone)]
struct MediaServerState {
    pack: PackState,
    token: String,
}

#[derive(Deserialize)]
struct Authentication {
    token: Option<String>,
}

pub async fn start(pack: PackState, token: String) -> anyhow::Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let router = Router::new()
        .route("/thumbnail/{id}", get(thumbnail_handler))
        .route("/preview/{id}", get(preview_handler))
        .route("/display/{id}", get(display_handler))
        .route("/file/{id}", get(file_handler))
        .with_state(MediaServerState { pack, token });

    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });

    Ok(port)
}

/// The rejection to send back when a request's token is not the one this server started with, or
/// `None` when it is and the handler should carry on.
fn rejection(state: &MediaServerState, authentication: &Authentication) -> Option<Response> {
    if authentication.token.as_deref() == Some(state.token.as_str()) {
        None
    } else {
        Some((StatusCode::UNAUTHORIZED, "Invalid media token").into_response())
    }
}

async fn thumbnail_handler(
    State(state): State<MediaServerState>,
    Query(authentication): Query<Authentication>,
    Path(id): Path<u64>,
) -> Response {
    if let Some(response) = rejection(&state, &authentication) {
        return response;
    }
    let view = {
        let lock = state.pack.lock().await;
        match lock.as_ref() {
            Some(pack) => match pack.get_view() {
                Ok(v) => v,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            },
            None => return (StatusCode::NOT_FOUND, "No pack open").into_response(),
        }
    };
    match view.get_thumbnail(id).await {
        Ok(data) => Response::builder()
            .status(200)
            .header("Content-Type", "image/webp")
            .body(axum::body::Body::from(data))
            .unwrap(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn preview_handler(
    State(state): State<MediaServerState>,
    Query(authentication): Query<Authentication>,
    Path(id): Path<u64>,
) -> Response {
    if let Some(response) = rejection(&state, &authentication) {
        return response;
    }
    let view = {
        let lock = state.pack.lock().await;
        match lock.as_ref() {
            Some(pack) => match pack.get_view() {
                Ok(v) => v,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            },
            None => return (StatusCode::NOT_FOUND, "No pack open").into_response(),
        }
    };
    match view.get_preview(id).await {
        Ok(data) => Response::builder()
            .status(200)
            .header("Content-Type", "image/jpeg")
            .body(axum::body::Body::from(data))
            .unwrap(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn display_handler(
    State(state): State<MediaServerState>,
    Query(authentication): Query<Authentication>,
    Path(id): Path<u64>,
) -> Response {
    if let Some(response) = rejection(&state, &authentication) {
        return response;
    }
    let view = {
        let lock = state.pack.lock().await;
        match lock.as_ref() {
            Some(pack) => match pack.get_view() {
                Ok(v) => v,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            },
            None => return (StatusCode::NOT_FOUND, "No pack open").into_response(),
        }
    };
    match view.get_display(id).await {
        Ok(data) => Response::builder()
            .status(200)
            .header("Content-Type", "image/jpeg")
            .body(axum::body::Body::from(data))
            .unwrap(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn file_handler(
    State(state): State<MediaServerState>,
    Query(authentication): Query<Authentication>,
    Path(id): Path<u64>,
    request_headers: HeaderMap,
) -> Response {
    if let Some(response) = rejection(&state, &authentication) {
        return response;
    }
    let view = {
        let lock = state.pack.lock().await;
        match lock.as_ref() {
            Some(pack) => match pack.get_view() {
                Ok(v) => v,
                Err(e) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
                }
            },
            None => return (StatusCode::NOT_FOUND, "No pack open").into_response(),
        }
    };

    let range_str = request_headers
        .get("Range")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    let requested_range = range_str.is_some();
    let range = match range_str {
        Some(range_str) => match parse_range(&range_str) {
            Ok(r) => r,
            Err(()) => return (StatusCode::RANGE_NOT_SATISFIABLE, "Invalid range").into_response(),
        },
        None => Range {
            start: Some(0),
            end: None,
        },
    };

    match view.get_file_range(id, range).await {
        Ok((dr, ft)) => {
            let partial = requested_range || dr.end < dr.total_size;
            let mut builder = Response::builder()
                .status(if partial { 206 } else { 200 })
                .header("Content-Type", file_type_mime(ft))
                .header("Accept-Ranges", "bytes")
                .header("Content-Length", dr.data.len());
            if partial {
                // Successful ranges are always non-empty, so this subtraction is safe.
                builder = builder.header(
                    "Content-Range",
                    format!("bytes {}-{}/{}", dr.start, dr.end - 1, dr.total_size),
                );
            }
            builder.body(axum::body::Body::from(dr.data)).unwrap()
        }
        Err(error) if error.is::<InvalidRange>() && requested_range => {
            (StatusCode::RANGE_NOT_SATISFIABLE, "Invalid range").into_response()
        }
        // The internally-generated 0- range is invalid only for an empty file. Reading that
        // empty file is safe and lets a range-less request receive a valid zero-length 200.
        Err(error) if error.is::<InvalidRange>() => match view.get_file_data(id).await {
            Ok((data, ft)) => Response::builder()
                .status(200)
                .header("Content-Type", file_type_mime(ft))
                .header("Accept-Ranges", "bytes")
                .header("Content-Length", data.len())
                .body(axum::body::Body::from(data))
                .unwrap(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        },
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

fn file_type_mime(ft: shared::encode::FileType) -> &'static str {
    match ft {
        shared::encode::FileType::Image => "image/avif",
        shared::encode::FileType::Video => "video/mp4",
        shared::encode::FileType::Audio => "audio/ogg",
    }
}

fn parse_range(s: &str) -> Result<Range, ()> {
    let value = s.strip_prefix("bytes=").ok_or(())?;
    if value.contains(',') {
        return Err(());
    }
    let mut parts = value.split('-');
    let start_str = parts.next().ok_or(())?;
    let end_str = parts.next().ok_or(())?;
    if parts.next().is_some() || (start_str.is_empty() && end_str.is_empty()) {
        return Err(());
    }
    let start = if start_str.is_empty() {
        None
    } else {
        Some(start_str.parse().map_err(|_| ())?)
    };
    let end = if end_str.is_empty() {
        None
    } else {
        Some(end_str.parse().map_err(|_| ())?)
    };
    Ok(Range { start, end })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::{parse_range, rejection, Authentication, MediaServerState};

    #[test]
    fn requires_the_startup_media_token() {
        let state = MediaServerState {
            pack: Arc::new(Mutex::new(None)),
            token: "secret".into(),
        };
        assert!(rejection(
            &state,
            &Authentication {
                token: Some("secret".into())
            }
        )
        .is_none());
        let response = rejection(&state, &Authentication { token: None }).expect("rejection");
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn parses_single_byte_ranges() {
        let closed = parse_range("bytes=10-20").unwrap();
        assert_eq!((closed.start, closed.end), (Some(10), Some(20)));
        let open = parse_range("bytes=10-").unwrap();
        assert_eq!((open.start, open.end), (Some(10), None));
        let suffix = parse_range("bytes=-10").unwrap();
        assert_eq!((suffix.start, suffix.end), (None, Some(10)));
    }

    #[test]
    fn rejects_malformed_or_multiple_ranges() {
        for value in [
            "items=0-1",
            "bytes=-",
            "bytes=abc-1",
            "bytes=0-1-2",
            "bytes=0-1,3-4",
        ] {
            assert!(parse_range(value).is_err(), "accepted {value}");
        }
    }
}
