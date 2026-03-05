use crate::app::UserEvent;
use std::{error::Error, fmt::Display, io, path::Path, rc::Rc, thread};
use winit::event_loop::EventLoopProxy;

use shared::pack_config::Metadata;
use tokio::{
    sync::{
        mpsc::{Sender, channel},
        oneshot,
    },
    task::LocalSet,
};

use crate::{
    audio::AudioPlayer,
    error::LewdwareError,
    lua::{Media, MediaType},
    media::{FileOrPath, pack::MediaPack, types::ImageData},
    video::VideoDecoder,
};

/// Manages all the media (images, audio, videos). Trivially clonable.
#[derive(Clone)]
pub struct MediaManager {
    tx: Sender<MediaRequest>,
}

pub type Result<T, E = MediaError> = std::result::Result<T, E>;

impl MediaManager {
    /// Start up the media manager thread, opening the specified pack file. Returns the pack
    /// metadata.
    pub fn open(
        pack_path: &Path,
        event_loop_proxy: EventLoopProxy<UserEvent>,
    ) -> anyhow::Result<(Self, Metadata)> {
        let (tx, metadata) = spawn_media_manager_thread(pack_path, event_loop_proxy)?;

        Ok((Self { tx }, metadata))
    }

    async fn send<T>(
        &self,
        request_builder: impl FnOnce(oneshot::Sender<T>) -> MediaRequest,
    ) -> Result<T> {
        let (tx, rx) = oneshot::channel();

        if let Err(_) = self.tx.send(request_builder(tx)).await {
            return Err(MediaError::Internal(
                "The media manager receiver was dropped",
            ));
        }

        rx.await
            .map_err(|_| MediaError::Internal("The response sender was dropped"))
    }

    // async fn send(&self, request: MediaRequest) {
    //     if let Err(_) = self.tx.send(request).await {
    //         eprintln!("Media request channel closed");
    //     }
    // }

    pub async fn get_media(&self, name: String, types: MediaTypes) -> Result<Option<Media>> {
        self.send(|tx| MediaRequest::GetMedia {
            types,
            name,
            response_tx: tx,
        })
        .await?
    }

    pub async fn random_media(
        &self,
        types: MediaTypes,
        tags: Option<Vec<String>>,
    ) -> Result<Option<Media>> {
        self.send(|tx| MediaRequest::RandomMedia {
            types,
            tags,
            response_tx: tx,
        })
        .await?
    }

    pub async fn list_media(
        &self,
        types: MediaTypes,
        tags: Option<Vec<String>>,
    ) -> Result<Vec<Media>> {
        self.send(|tx| MediaRequest::ListMedia {
            types,
            tags,
            response_tx: tx,
        })
        .await?
    }

    pub async fn get_image_data(&self, id: u64, width: u32, height: u32) -> Result<ImageData> {
        self.send(|tx| MediaRequest::GetImageData {
            id,
            width,
            height,
            response_tx: tx,
        })
        .await?
    }

    pub async fn get_image_file(&self, id: u64) -> Result<FileOrPath> {
        self.send(|tx| MediaRequest::GetImageFile {
            id,
            response_tx: tx,
        })
        .await?
    }

    pub async fn get_video_data(
        &self,
        id: u64,
        width: u32,
        height: u32,
        loop_video: bool,
        play_audio: bool,
    ) -> Result<VideoDecoder> {
        self.send(|tx| MediaRequest::GetVideoData {
            id,
            response_tx: tx,
            width,
            height,
            loop_video,
            play_audio,
        })
        .await?
    }

    pub async fn get_audio_data(
        &self,
        id: u64,
        audio_id: u64,
        loop_audio: bool,
    ) -> Result<AudioPlayer> {
        self.send(|tx| MediaRequest::GetAudioData {
            id,
            audio_id,
            loop_audio,
            response_tx: tx,
        })
        .await?
    }

    pub async fn get_mode(&self, id: u64) -> anyhow::Result<Vec<u8>> {
        self.send(|tx| MediaRequest::GetModeData { id, response_tx: tx }).await?
    }
}

fn spawn_media_manager_thread(
    pack_path: &Path,
    event_loop_proxy: EventLoopProxy<UserEvent>,
) -> anyhow::Result<(Sender<MediaRequest>, Metadata)> {
    let (req_tx, mut req_rx) = channel(20);

    let file = MediaPack::open(pack_path)?;
    let metadata = file.metadata().clone();

    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        let local = LocalSet::new();
        local.spawn_local(async move {
            let manager = Rc::new(file);

            while let Some(request) = req_rx.recv().await {
                let manager = manager.clone();
                let event_loop_proxy = event_loop_proxy.clone();

                tokio::task::spawn_local(async move {
                    handle_request(manager, request, event_loop_proxy).await;
                });
            }
        });

        rt.block_on(local);
    });

    Ok((req_tx, metadata))
}

async fn handle_request(
    pack: Rc<MediaPack>,
    request: MediaRequest,
    event_loop_proxy: EventLoopProxy<UserEvent>,
) {
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
            width,
            height,
            response_tx,
        } => response_tx
            .send(pack.get_image_data(id, width, height).await)
            .is_ok(),
        MediaRequest::GetImageFile { id, response_tx } => {
            response_tx.send(pack.get_image_file(id).await).is_ok()
        }
        MediaRequest::GetVideoData {
            id,
            width,
            height,
            play_audio,
            loop_video,
            response_tx,
        } => response_tx
            .send(pack.get_video_data(id).await.and_then(|data| {
                VideoDecoder::new(data.file, width, height, play_audio, loop_video)
                    .map_err(|err| MediaError::VideoError(err))
            }))
            .is_ok(),
        MediaRequest::GetAudioData {
            id,
            audio_id,
            loop_audio,
            response_tx,
        } => response_tx
            .send(pack.get_audio_data(id).await.and_then(|file| {
                AudioPlayer::new(
                    file.path().to_path_buf(),
                    loop_audio,
                    Some(audio_id),
                    Some(event_loop_proxy),
                )
                .map_err(|err| MediaError::AudioError(err))
            }))
            .is_ok(),
        MediaRequest::GetModeData { id, response_tx } => {
            response_tx.send(pack.get_mode(id)).is_ok()
        }
    } {
        eprintln!("Failed to send response");
    }
}

struct Request {
    pub id: u64,
    pub request: MediaRequest,
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

enum MediaRequest {
    GetMedia {
        types: MediaTypes,
        name: String,
        response_tx: oneshot::Sender<Result<Option<Media>>>,
    },
    RandomMedia {
        types: MediaTypes,
        tags: Option<Vec<String>>,
        response_tx: oneshot::Sender<Result<Option<Media>>>,
    },
    ListMedia {
        types: MediaTypes,
        tags: Option<Vec<String>>,
        response_tx: oneshot::Sender<Result<Vec<Media>>>,
    },
    GetImageData {
        id: u64,
        width: u32,
        height: u32,
        response_tx: oneshot::Sender<Result<ImageData>>,
    },
    GetImageFile {
        id: u64,
        response_tx: oneshot::Sender<Result<FileOrPath>>,
    },
    GetVideoData {
        id: u64,
        width: u32,
        height: u32,
        play_audio: bool,
        loop_video: bool,
        response_tx: oneshot::Sender<Result<VideoDecoder>>,
    },
    GetAudioData {
        id: u64,
        audio_id: u64,
        loop_audio: bool,
        response_tx: oneshot::Sender<Result<AudioPlayer>>,
    },
    GetModeData {
        id: u64,
        response_tx: oneshot::Sender<anyhow::Result<Vec<u8>>>,
    },
}

#[derive(Debug)]
pub enum MediaError {
    DbError(rusqlite::Error),
    InvalidTag(String),
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
            MediaError::InvalidTag(tag) => {
                write!(f, "Invalid tag '{tag}'")
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
