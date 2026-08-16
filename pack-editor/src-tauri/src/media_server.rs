use std::{io, time::Instant};

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::{
    pack::{InvalidRange, Range},
    PackState,
};

/// Target for the media-playback trail: every `/file` request, what it resolved to, how long each
/// stage took, and how much of the body the client actually took -- interleaved with what the
/// player did, which the front end forwards through `trace_media_event` (see `lib.rs`) so both
/// halves read as one ordered story.
///
/// It exists to answer one question, and did: when playback stalls, is the player waiting on a
/// request the server is slow to answer, or has it stopped asking? (It had stopped asking, with
/// the whole file already in hand -- see `MediaDisplay.svelte`'s stall workaround.)
///
/// The per-request detail is `debug`, so it shows on stderr in a development build and stays out
/// of the persisted log; only the anomaly -- a player that had to be nudged -- is `info` and kept.
/// Raise it with `RUST_LOG=media_trace=info` to persist the lot while chasing something.
pub const MEDIA_TRACE: &str = "media_trace";

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

async fn file_handler(
    State(state): State<MediaServerState>,
    Query(authentication): Query<Authentication>,
    Path(id): Path<u64>,
    request_headers: HeaderMap,
) -> Response {
    if let Some(response) = rejection(&state, &authentication) {
        return response;
    }

    let received = Instant::now();
    let requested = request_headers
        .get("Range")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_owned();
    tracing::debug!(target: MEDIA_TRACE, id, range = %requested, "server: request");

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
    // Split out because this is the one wait that has nothing to do with the file: every Tauri
    // command holds the same pack mutex, some of them for their whole duration, so a request
    // issued the moment playback starts can queue behind unrelated editor work.
    let pack_lock_ms = received.elapsed().as_millis();

    let range_str = (requested != "-").then(|| requested.clone());
    let requested_range = range_str.is_some();
    let range = match range_str {
        Some(range_str) => match parse_range(&range_str) {
            Ok(r) => r,
            Err(()) => {
                tracing::debug!(target: MEDIA_TRACE, id, range = %requested, "server: 416 unparseable range");
                return (StatusCode::RANGE_NOT_SATISFIABLE, "Invalid range").into_response();
            }
        },
        None => Range {
            start: Some(0),
            end: None,
        },
    };

    match view.open_file_range(id, range).await {
        Ok((dr, ft)) => {
            let length = dr.len();
            tracing::debug!(
                target: MEDIA_TRACE,
                id,
                range = %requested,
                status = if requested_range { 206 } else { 200 },
                start = dr.start,
                end = dr.end,
                total = dr.total_size,
                length,
                pack_lock_ms,
                opened_ms = received.elapsed().as_millis(),
                "server: responding"
            );
            // 206 only when the client asked for a range. A response to a range-less GET carries
            // the whole file, so it is a 200 -- answering one with a 206 would be a client's cue
            // to treat a partial body as everything there is.
            // A validator on every response, so a client holding part of this media can tell that
            // a later range response is the same entity. Advertising `Accept-Ranges` without one
            // asks a player to reconcile a `206` against what it already has on nothing but the
            // URL -- and a player that cannot reconcile it has no way forward but to wait.
            let mut builder = Response::builder()
                .status(if requested_range { 206 } else { 200 })
                .header("Content-Type", file_type_mime(ft))
                .header("Accept-Ranges", "bytes")
                .header("ETag", format!("\"{}\"", dr.entity_tag))
                .header("Content-Length", dr.len());
            if requested_range {
                // Successful ranges are always non-empty, so this subtraction is safe.
                builder = builder.header(
                    "Content-Range",
                    format!("bytes {}-{}/{}", dr.start, dr.end - 1, dr.total_size),
                );
            }
            builder
                .body(axum::body::Body::from_stream(traced_body(
                    dr.into_stream(),
                    BodyTrace {
                        id,
                        length,
                        sent: 0,
                        started: Instant::now(),
                        first_byte_ms: None,
                        error: None,
                    },
                )))
                .unwrap()
        }
        Err(error) if error.is::<InvalidRange>() && requested_range => {
            tracing::debug!(target: MEDIA_TRACE, id, range = %requested, "server: 416 unsatisfiable range");
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

/// What became of one response body, logged when the body ends -- whichever way it ended.
///
/// A media client routinely takes part of a body and hangs up, coming back later for the rest
/// from where it stopped; that is normal and shows up here as an incomplete body followed by a
/// fresh request. A stall is the case where an incomplete body is *not* followed by one.
struct BodyTrace {
    id: u64,
    length: u64,
    sent: u64,
    started: Instant,
    first_byte_ms: Option<u128>,
    error: Option<String>,
}

impl Drop for BodyTrace {
    fn drop(&mut self) {
        tracing::debug!(
            target: MEDIA_TRACE,
            id = self.id,
            sent = self.sent,
            of = self.length,
            complete = self.sent >= self.length,
            first_byte_ms = self.first_byte_ms,
            elapsed_ms = self.started.elapsed().as_millis(),
            error = self.error.as_deref().unwrap_or("-"),
            "server: body ended"
        );
    }
}

/// Counts a body's bytes on their way out, so [`BodyTrace`] can report what the client took.
///
/// Dropping the returned stream -- which is what hyper does when the client hangs up -- drops the
/// trace with it, so an abandoned body reports itself at the moment it is abandoned.
fn traced_body(
    stream: impl Stream<Item = io::Result<Vec<u8>>> + Send + 'static,
    trace: BodyTrace,
) -> impl Stream<Item = io::Result<Vec<u8>>> + Send {
    futures::stream::unfold(
        (Box::pin(stream), trace),
        |(mut stream, mut trace)| async move {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    if trace.first_byte_ms.is_none() {
                        trace.first_byte_ms = Some(trace.started.elapsed().as_millis());
                    }
                    trace.sent += chunk.len() as u64;
                    Some((Ok(chunk), (stream, trace)))
                }
                Some(Err(error)) => {
                    trace.error = Some(error.to_string());
                    Some((Err(error), (stream, trace)))
                }
                None => None,
            }
        },
    )
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
    use std::sync::{Arc, Mutex as StdMutex};

    use tokio::sync::Mutex;

    use super::*;
    use super::{parse_range, rejection, Authentication, MediaServerState};

    /// Collects a subscriber's output so a trace can be asserted on.
    #[derive(Clone, Default)]
    struct Capture(Arc<StdMutex<Vec<u8>>>);

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    impl io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl tracing_subscriber::fmt::MakeWriter<'_> for Capture {
        type Writer = Self;
        fn make_writer(&self) -> Self::Writer {
            self.clone()
        }
    }

    fn trace_of(body: BodyTrace, chunks: Vec<usize>, take: usize) -> String {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .without_time()
            // Or the fields arrive wrapped in colour escapes and match nothing.
            .with_ansi(false)
            // The trail is `debug`; the default subscriber would stop at `info`.
            .with_max_level(tracing::Level::DEBUG)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let stream = futures::stream::iter(
            chunks
                .into_iter()
                .map(|size| Ok(vec![0u8; size]))
                .collect::<Vec<io::Result<Vec<u8>>>>(),
        );
        futures::executor::block_on(async {
            let mut traced = Box::pin(traced_body(stream, body));
            for _ in 0..take {
                let _ = traced.next().await;
            }
            // What hyper does to a response body when the client hangs up.
            drop(traced);
        });
        capture.text()
    }

    fn body_trace() -> BodyTrace {
        BodyTrace {
            id: 7,
            length: 30,
            sent: 0,
            started: Instant::now(),
            first_byte_ms: None,
            error: None,
        }
    }

    /// The distinction the whole trace exists to draw: a body the client walked away from reports
    /// how much of it the client actually took, at the moment it walked away.
    #[test]
    fn an_abandoned_body_reports_what_the_client_took() {
        let text = trace_of(body_trace(), vec![10, 10, 10], 1);
        assert!(text.contains("server: body ended"), "{text}");
        assert!(text.contains("sent=10"), "{text}");
        assert!(text.contains("of=30"), "{text}");
        assert!(text.contains("complete=false"), "{text}");
    }

    #[test]
    fn a_body_read_to_its_end_reports_complete() {
        let text = trace_of(body_trace(), vec![10, 10, 10], 4);
        assert!(text.contains("sent=30"), "{text}");
        assert!(text.contains("complete=true"), "{text}");
    }

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
