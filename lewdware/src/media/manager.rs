use crate::app::{EventPoster, UserEvent};
use shared::read_pack::Metadata;
use std::{
    error::Error,
    fmt::Display,
    io,
    path::Path,
    rc::Rc,
    sync::{Arc, mpsc as std_mpsc},
    thread,
};

use tokio::{sync::mpsc::channel, task::LocalSet};

use crate::{
    audio::AudioPlayer,
    error::LewdwareError,
    lua::{Media, MediaType, PopupId},
    media::{FileOrPath, pack::MediaPack},
    video::VideoDecoder,
};

/// Manages all the media (images, audio, videos). Trivially clonable.
#[derive(Clone)]
pub struct MediaManager {
    tx: std_mpsc::SyncSender<MediaRequest>,
    wgpu_device: Option<Arc<wgpu::Device>>,
}

pub type Result<T, E = MediaError> = std::result::Result<T, E>;

impl MediaManager {
    /// Start up the media manager thread, opening the specified pack file. Returns the pack
    /// metadata and a handle for the spawned thread.
    ///
    /// The returned `JoinHandle` should be joined once every clone of this `MediaManager` has
    /// been dropped, so the thread's request channel closes and it can shut down, running the
    /// `Drop` impl of its `MediaPack` (which owns a `NamedTempFile` for the pack's extracted
    /// index). Otherwise that temp file is never cleaned up.
    pub fn open(
        pack_path: &Path,
        event_poster: EventPoster,
        wgpu_device: Option<Arc<wgpu::Device>>,
    ) -> anyhow::Result<(Self, Metadata, thread::JoinHandle<()>)> {
        let (tx, metadata, handle) = spawn_media_manager_thread(pack_path, event_poster)?;

        Ok((Self { tx, wgpu_device }, metadata, handle))
    }

    /// Enqueue a request and block the calling thread until a response arrives. Safe to call from
    /// the Lua thread even though it's already driving its own Tokio runtime: this uses
    /// `std::sync::mpsc`, not Tokio's channels, so there's no risk of the "blocking call from
    /// within a runtime" panic that `Sender::blocking_send`/`Receiver::blocking_recv` would raise.
    fn send<T>(
        &self,
        request_builder: impl FnOnce(std_mpsc::Sender<T>) -> MediaRequest,
    ) -> Result<T> {
        let (tx, rx) = std_mpsc::channel();

        if self.tx.send(request_builder(tx)).is_err() {
            return Err(MediaError::Internal(
                "The media manager receiver was dropped",
            ));
        }

        rx.recv()
            .map_err(|_| MediaError::Internal("The response sender was dropped"))
    }

    /// Enqueue a request that doesn't wait for a response — the manager thread delivers the
    /// result later by sending a `UserEvent` directly (see `get_image_data`/`get_video_data`/
    /// `get_audio_data` and `handle_request`). Non-blocking, so it's safe to call from the main
    /// (winit) thread.
    fn try_send(&self, request: MediaRequest) -> Result<()> {
        self.tx
            .try_send(request)
            .map_err(|_| MediaError::Internal("The media manager receiver was dropped or full"))
    }

    pub fn get_media(&self, name: String, types: MediaTypes) -> Result<Option<Media>> {
        self.send(|tx| MediaRequest::GetMedia {
            types,
            name,
            response_tx: tx,
        })?
    }

    pub fn random_media(
        &self,
        types: MediaTypes,
        tags: Option<TagFilter>,
    ) -> Result<Option<Media>> {
        self.send(|tx| MediaRequest::RandomMedia {
            types,
            tags,
            response_tx: tx,
        })?
    }

    pub fn list_media(&self, types: MediaTypes, tags: Option<TagFilter>) -> Result<Vec<Media>> {
        self.send(|tx| MediaRequest::ListMedia {
            types,
            tags,
            response_tx: tx,
        })?
    }

    /// Request an image be decoded/resized to `(width, height)`. Returns as soon as the request
    /// is enqueued — the result arrives later as a `UserEvent::ImageResolved { id, .. }`.
    pub fn get_image_data(
        &self,
        id: PopupId,
        media_id: u64,
        width: u32,
        height: u32,
    ) -> Result<()> {
        self.try_send(MediaRequest::GetImageData {
            id,
            media_id,
            width,
            height,
        })
    }

    pub fn get_image_file(&self, id: u64) -> Result<FileOrPath> {
        self.send(|tx| MediaRequest::GetImageFile {
            id,
            response_tx: tx,
        })?
    }

    /// Request a video decoder be set up. Returns as soon as the request is enqueued — the
    /// result arrives later as a `UserEvent::VideoResolved { id, .. }`.
    pub fn get_video_data(
        &self,
        id: PopupId,
        media_id: u64,
        loop_video: bool,
        play_audio: bool,
    ) -> Result<()> {
        let wgpu_device = self.wgpu_device.clone();
        self.try_send(MediaRequest::GetVideoData {
            id,
            media_id,
            loop_video,
            play_audio,
            wgpu_device,
        })
    }

    /// Request an audio decoder be set up. Returns as soon as the request is enqueued — the
    /// result arrives later as a `UserEvent::AudioResolved { id, .. }`.
    pub fn get_audio_data(&self, id: u64, media_id: u64, loop_audio: bool) -> Result<()> {
        self.try_send(MediaRequest::GetAudioData {
            id,
            media_id,
            loop_audio,
        })
    }

    pub fn get_mode(&self, id: u64) -> anyhow::Result<Vec<u8>> {
        self.send(|tx| MediaRequest::GetModeData {
            id,
            response_tx: tx,
        })?
    }
}

fn spawn_media_manager_thread(
    pack_path: &Path,
    event_poster: EventPoster,
) -> anyhow::Result<(
    std_mpsc::SyncSender<MediaRequest>,
    Metadata,
    thread::JoinHandle<()>,
)> {
    let (req_tx, req_rx) = std_mpsc::sync_channel(20);

    let file = MediaPack::open(pack_path)?;
    let metadata = file.metadata().clone();

    let handle = thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        // `send`/`try_send` above use `std::sync::mpsc` so they can block the Lua thread (which
        // is busy driving its own Tokio runtime) without risking the "blocking call from within a
        // runtime" panic that `Sender::blocking_send` guards against. That means `req_rx` can't be
        // awaited directly by the async loop below, so this thread bridges it into a Tokio channel
        // first. This bridge thread has no Tokio runtime of its own entered, so `blocking_send` is
        // safe here.
        let (async_tx, mut async_rx) = channel(20);
        thread::spawn(move || {
            while let Ok(request) = req_rx.recv() {
                if async_tx.blocking_send(request).is_err() {
                    break;
                }
            }
        });

        let local = LocalSet::new();
        local.spawn_local(async move {
            let manager = Rc::new(file);

            while let Some(request) = async_rx.recv().await {
                let manager = manager.clone();
                let event_poster = event_poster.clone();

                tokio::task::spawn_local(async move {
                    handle_request(manager, request, event_poster).await;
                });
            }

            // Dropping `manager` here (rather than leaving it to fall out of scope with the
            // task) makes it explicit: once the request channel closes, the pack's temp files
            // (e.g. its extracted SQLite index) are cleaned up before the thread exits.
            drop(manager);
        });

        rt.block_on(local);
    });

    Ok((req_tx, metadata, handle))
}

async fn handle_request(pack: Rc<MediaPack>, request: MediaRequest, event_poster: EventPoster) {
    if !match request {
        MediaRequest::GetMedia {
            types,
            name,
            response_tx,
        } => response_tx.send(pack.get_media(name, types)).is_ok(),
        MediaRequest::RandomMedia {
            types,
            tags,
            response_tx,
        } => response_tx.send(pack.random_media(types, tags)).is_ok(),
        MediaRequest::ListMedia {
            types,
            tags,
            response_tx,
        } => response_tx.send(pack.list_media(types, tags)).is_ok(),
        MediaRequest::GetImageData {
            id,
            media_id,
            width,
            height,
        } => {
            let result = pack.get_image_data(media_id, width, height).await;
            event_poster(UserEvent::ImageResolved { id, result })
        }
        MediaRequest::GetImageFile { id, response_tx } => {
            response_tx.send(pack.get_image_file(id).await).is_ok()
        }
        MediaRequest::GetVideoData {
            id,
            media_id,
            play_audio,
            loop_video,
            wgpu_device,
        } => {
            let result = pack.get_video_data(media_id).and_then(|data| {
                VideoDecoder::new(
                    data.source,
                    play_audio,
                    loop_video,
                    data.transparent,
                    wgpu_device,
                )
                .map_err(MediaError::VideoError)
            });
            event_poster(UserEvent::VideoResolved { id, result })
        }
        MediaRequest::GetAudioData {
            id,
            media_id,
            loop_audio,
        } => {
            let result = pack.get_audio_data(media_id).and_then(|source| {
                AudioPlayer::new(source, loop_audio, Some(id), Some(event_poster.clone()))
                    .map_err(MediaError::AudioError)
            });
            event_poster(UserEvent::AudioResolved { id, result })
        }
        MediaRequest::GetModeData { id, response_tx } => {
            response_tx.send(pack.get_mode(id)).is_ok()
        }
    } {
        // Either the requester's oneshot receiver was dropped, or (for the `*Resolved` events)
        // the event loop is gone. Normal when a request is abandoned mid-flight, e.g. during
        // shutdown, so this isn't logged as an error.
        tracing::debug!("Failed to deliver response: receiver gone");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaTypes {
    pub image: bool,
    pub video: bool,
    pub audio: bool,
}

impl MediaTypes {
    pub const NONE: Self = Self {
        image: false,
        video: false,
        audio: false,
    };

    pub const ALL: Self = Self {
        image: true,
        video: true,
        audio: true,
    };

    pub const IMAGE: Self = Self {
        image: true,
        video: false,
        audio: false,
    };

    pub const VIDEO: Self = Self {
        image: false,
        video: true,
        audio: false,
    };

    pub const AUDIO: Self = Self {
        image: false,
        video: false,
        audio: true,
    };

    pub fn from_slice(types: &[MediaType]) -> Self {
        let mut result = Self::NONE;

        for t in types {
            match t {
                MediaType::Image => {
                    result.image = true;
                }
                MediaType::Video => {
                    result.video = true;
                }
                MediaType::Audio => {
                    result.audio = true;
                }
            }
        }

        result
    }
}

/// A tag-based filter for media queries. Empty lists impose no constraint, so the default
/// value matches everything. Tags the pack doesn't define never match: they are ignored in
/// `any` and `none`, while an unknown tag in `all` means nothing can satisfy the filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagFilter {
    /// Match media with at least one of these tags.
    pub any: Vec<String>,
    /// Match media with every one of these tags.
    pub all: Vec<String>,
    /// Exclude media with any of these tags.
    pub none: Vec<String>,
}

impl TagFilter {
    pub fn any_of(tags: Vec<String>) -> Self {
        Self {
            any: tags,
            ..Default::default()
        }
    }
}

enum MediaRequest {
    GetMedia {
        types: MediaTypes,
        name: String,
        response_tx: std_mpsc::Sender<Result<Option<Media>>>,
    },
    RandomMedia {
        types: MediaTypes,
        tags: Option<TagFilter>,
        response_tx: std_mpsc::Sender<Result<Option<Media>>>,
    },
    ListMedia {
        types: MediaTypes,
        tags: Option<TagFilter>,
        response_tx: std_mpsc::Sender<Result<Vec<Media>>>,
    },
    GetImageData {
        id: PopupId,
        media_id: u64,
        width: u32,
        height: u32,
    },
    GetImageFile {
        id: u64,
        response_tx: std_mpsc::Sender<Result<FileOrPath>>,
    },
    GetVideoData {
        id: PopupId,
        media_id: u64,
        play_audio: bool,
        loop_video: bool,
        wgpu_device: Option<Arc<wgpu::Device>>,
    },
    GetAudioData {
        id: u64,
        media_id: u64,
        loop_audio: bool,
    },
    GetModeData {
        id: u64,
        response_tx: std_mpsc::Sender<anyhow::Result<Vec<u8>>>,
    },
}

#[derive(Debug)]
pub enum MediaError {
    DbError(rusqlite::Error),
    IoError(io::Error),
    ImageError(image::error::ImageError),
    VideoError(anyhow::Error),
    AudioError(anyhow::Error),
    Internal(&'static str),
}

impl Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::DbError(error) => {
                writeln!(f, "Error querying database")?;
                error.fmt(f)
            }
            MediaError::IoError(err) => err.fmt(f),
            MediaError::ImageError(err) => err.fmt(f),
            MediaError::VideoError(err) => write!(f, "Error decoding video: {err}"),
            MediaError::AudioError(err) => write!(f, "Error decoding audio: {err}"),
            MediaError::Internal(err) => write!(f, "Internal error: {err}"),
        }
    }
}

impl Error for MediaError {}

impl From<rusqlite::Error> for MediaError {
    fn from(value: rusqlite::Error) -> Self {
        Self::DbError(value)
    }
}

impl From<io::Error> for MediaError {
    fn from(value: io::Error) -> Self {
        Self::IoError(value)
    }
}

impl From<image::error::ImageError> for MediaError {
    fn from(value: image::error::ImageError) -> Self {
        Self::ImageError(value)
    }
}

impl From<MediaError> for LewdwareError {
    fn from(value: MediaError) -> Self {
        match value {
            MediaError::Internal(err) => LewdwareError::Internal(err),
            _ => LewdwareError::MediaError(value),
        }
    }
}
