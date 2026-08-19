use crate::app::{EventPoster, UserEvent};
use shared::read_pack::Metadata;
use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    io,
    path::Path,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool, mpsc as std_mpsc},
    thread,
};
use uuid::Uuid;

use tokio::{sync::mpsc::unbounded_channel, task::LocalSet};

use crate::{
    audio::AudioPlayer,
    error::LewdwareError,
    lua::{ItemId, Media, MediaType},
    media::{ExtractedFile, ImageData, pack::MediaPack},
    video::VideoDecoder,
};

/// Manages all the media (images, audio, videos). Trivially clonable.
#[derive(Clone)]
pub struct MediaManager {
    inner: Arc<MediaManagerInner>,
    wgpu_device: Option<Arc<wgpu::Device>>,
}

struct MediaManagerInner {
    tx: Option<std_mpsc::Sender<MediaRequest>>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl Drop for MediaManagerInner {
    fn drop(&mut self) {
        // Close the request channel before joining. This runs only after the last
        // `MediaManager` clone has gone away.
        self.tx.take();

        if let Some(handle) = self.join_handle.take()
            && handle.join().is_err()
        {
            tracing::error!("Media manager thread panicked");
        }
    }
}

pub type Result<T, E = MediaError> = std::result::Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequirementId(pub u64);

#[derive(Clone)]
pub enum MediaRequirement {
    Image {
        media_id: u64,
        width: u32,
        height: u32,
    },
    Video {
        media_id: u64,
        loop_video: Arc<AtomicBool>,
        play_audio: bool,
        volume: f32,
    },
    Audio {
        item_id: ItemId,
        media_id: u64,
        loop_audio: bool,
        volume: f32,
    },
}

/// A decoder is much larger than a handle to a sound or a decoded image, but only one of these
/// exists per piece of media in flight and each is moved straight into the item that asked for
/// it, so boxing the variant would buy an allocation rather than save one.
#[allow(clippy::large_enum_variant)]
pub enum ResolvedMedia {
    Image(ImageData),
    Video(VideoDecoder),
    Audio(AudioPlayer),
}

impl MediaManager {
    /// Start up the media manager thread, opening the specified pack file. Returns the pack
    /// metadata and the pack's stable UUID (from its header -- see `lewdware.pack`). The worker
    /// thread is shut down and joined when the last clone of the returned manager is dropped.
    pub fn open<T: EventPoster>(
        pack_path: &Path,
        event_poster: T,
        wgpu_device: Option<Arc<wgpu::Device>>,
    ) -> anyhow::Result<(Self, Metadata, Uuid)> {
        let (tx, metadata, pack_id, handle) = spawn_media_manager_thread(pack_path, event_poster)?;

        Ok((
            Self {
                inner: Arc::new(MediaManagerInner {
                    tx: Some(tx),
                    join_handle: Some(handle),
                }),
                wgpu_device,
            },
            metadata,
            pack_id,
        ))
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

        if self
            .inner
            .tx
            .as_ref()
            .expect("media manager sender exists while the manager is alive")
            .send(request_builder(tx))
            .is_err()
        {
            return Err(MediaError::Internal(
                "The media manager receiver was dropped",
            ));
        }

        rx.recv()
            .map_err(|_| MediaError::Internal("The response sender was dropped"))
    }

    /// Enqueue a request without waiting for a response. The unbounded ingress keeps this
    /// non-blocking even when one item has many concurrent requirements.
    fn enqueue(&self, request: MediaRequest) -> Result<()> {
        self.inner
            .tx
            .as_ref()
            .expect("media manager sender exists while the manager is alive")
            .send(request)
            .map_err(|_| MediaError::Internal("The media manager receiver was dropped"))
    }

    pub fn get_media(&self, name: String, types: MediaTypes) -> Result<Option<Media>> {
        self.send(|tx| MediaRequest::GetMedia {
            types,
            name,
            response_tx: tx,
        })?
    }

    /// One file by its media id, for the behaviour document's wallpaper/splash slots.
    pub fn get_media_by_id(&self, id: u64) -> Result<Option<Media>> {
        self.send(|tx| MediaRequest::GetMediaById {
            id,
            response_tx: tx,
        })?
    }

    pub fn random_media(
        &self,
        types: MediaTypes,
        tags: Option<TagFilter>,
        weights: Option<HashMap<u64, f64>>,
    ) -> Result<Option<Media>> {
        self.send(|tx| MediaRequest::RandomMedia {
            types,
            tags,
            weights,
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

    /// The pack's full tag vocabulary -- every tag it defines, not filtered by any query. See
    /// `MediaPack::list_tags`'s doc comment for why this isn't affected by the user's tag
    /// exclusion list either.
    pub fn list_tags(&self) -> Result<Vec<String>> {
        self.send(|tx| MediaRequest::ListTags { response_tx: tx })?
    }

    pub fn resolve(
        &self,
        requirement_id: RequirementId,
        requirement: MediaRequirement,
    ) -> Result<()> {
        self.enqueue(MediaRequest::Resolve {
            requirement_id,
            requirement,
            wgpu_device: self.wgpu_device.clone(),
        })
    }

    pub fn get_image_file(&self, id: u64) -> Result<ExtractedFile> {
        self.send(|tx| MediaRequest::GetImageFile {
            id,
            response_tx: tx,
        })?
    }

    pub fn get_mode(&self, id: u64) -> anyhow::Result<Vec<u8>> {
        self.send(|tx| MediaRequest::GetModeData {
            id,
            response_tx: tx,
        })?
    }

    /// Reads a named blob from the pack's `pack_data` table (e.g. `"behaviour"` for
    /// behaviour.json). `None` if the pack doesn't carry an entry with this name.
    /// The pack's behaviour document. See `MediaPack::get_behaviour`.
    pub fn get_behaviour(&self) -> anyhow::Result<shared::behaviour::Behaviour> {
        self.send(|tx| MediaRequest::GetBehaviour { response_tx: tx })
            .map_err(anyhow::Error::from)?
    }
}

fn spawn_media_manager_thread<T: EventPoster>(
    pack_path: &Path,
    event_poster: T,
) -> anyhow::Result<(
    std_mpsc::Sender<MediaRequest>,
    Metadata,
    Uuid,
    thread::JoinHandle<()>,
)> {
    let (req_tx, req_rx) = std_mpsc::channel();

    let file = MediaPack::open(pack_path)?;
    let metadata = file.metadata().clone();
    let pack_id = file.header().id;

    let handle = thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        // The synchronous query path uses `std::sync::mpsc`, so it can block the Lua thread (which
        // is busy driving its own Tokio runtime) without risking the "blocking call from within a
        // runtime" panic that `Sender::blocking_send` guards against. That means `req_rx` can't be
        // awaited directly by the async loop below, so this thread bridges it into a Tokio channel
        // first. This bridge thread has no Tokio runtime of its own entered, so `blocking_send` is
        // safe here.
        let (async_tx, mut async_rx) = unbounded_channel();
        thread::spawn(move || {
            while let Ok(request) = req_rx.recv() {
                if async_tx.send(request).is_err() {
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

    Ok((req_tx, metadata, pack_id, handle))
}

async fn handle_request<T: EventPoster>(
    pack: Rc<MediaPack>,
    request: MediaRequest,
    event_poster: T,
) {
    if !match request {
        MediaRequest::GetMedia {
            types,
            name,
            response_tx,
        } => response_tx.send(pack.get_media(name, types)).is_ok(),
        MediaRequest::GetMediaById { id, response_tx } => {
            response_tx.send(pack.get_media_by_id(id)).is_ok()
        }
        MediaRequest::RandomMedia {
            types,
            tags,
            weights,
            response_tx,
        } => response_tx
            .send(match weights {
                Some(weights) => pack
                    .list_media(types, tags)
                    .map(|media| weighted_media(media, &weights)),
                None => pack.random_media(types, tags),
            })
            .is_ok(),
        MediaRequest::ListMedia {
            types,
            tags,
            response_tx,
        } => response_tx.send(pack.list_media(types, tags)).is_ok(),
        MediaRequest::ListTags { response_tx } => response_tx.send(pack.list_tags()).is_ok(),
        MediaRequest::Resolve {
            requirement_id,
            requirement,
            wgpu_device,
        } => {
            let result = match requirement {
                MediaRequirement::Image {
                    media_id,
                    width,
                    height,
                } => pack
                    .get_image_data(media_id, width, height)
                    .await
                    .map(ResolvedMedia::Image),
                MediaRequirement::Video {
                    media_id,
                    loop_video,
                    play_audio,
                    volume,
                } => pack
                    .get_video_data(media_id)
                    .and_then(|data| {
                        VideoDecoder::new(
                            data.source,
                            play_audio,
                            loop_video,
                            volume,
                            data.transparent,
                            wgpu_device,
                        )
                        .map_err(MediaError::VideoError)
                    })
                    .map(ResolvedMedia::Video),
                MediaRequirement::Audio {
                    item_id,
                    media_id,
                    loop_audio,
                    volume,
                } => pack
                    .get_audio_data(media_id)
                    .and_then(|source| {
                        AudioPlayer::new(
                            source,
                            Arc::new(AtomicBool::new(loop_audio)),
                            volume,
                            Some((item_id, event_poster.clone())),
                        )
                        .and_then(|x| x.ok_or(anyhow::anyhow!("No audio stream available")))
                        .map_err(MediaError::AudioError)
                    })
                    .map(ResolvedMedia::Audio),
            };
            event_poster.post_event(UserEvent::MediaResolved {
                requirement_id,
                result,
            })
        }
        MediaRequest::GetImageFile { id, response_tx } => {
            response_tx.send(pack.get_image_file(id).await).is_ok()
        }
        MediaRequest::GetModeData { id, response_tx } => {
            response_tx.send(pack.get_mode(id)).is_ok()
        }
        MediaRequest::GetBehaviour { response_tx } => {
            response_tx.send(pack.get_behaviour()).is_ok()
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
    GetMediaById {
        id: u64,
        response_tx: std_mpsc::Sender<Result<Option<Media>>>,
    },
    RandomMedia {
        types: MediaTypes,
        tags: Option<TagFilter>,
        weights: Option<HashMap<u64, f64>>,
        response_tx: std_mpsc::Sender<Result<Option<Media>>>,
    },
    ListMedia {
        types: MediaTypes,
        tags: Option<TagFilter>,
        response_tx: std_mpsc::Sender<Result<Vec<Media>>>,
    },
    ListTags {
        response_tx: std_mpsc::Sender<Result<Vec<String>>>,
    },
    Resolve {
        requirement_id: RequirementId,
        requirement: MediaRequirement,
        wgpu_device: Option<Arc<wgpu::Device>>,
    },
    GetImageFile {
        id: u64,
        response_tx: std_mpsc::Sender<Result<ExtractedFile>>,
    },
    GetModeData {
        id: u64,
        response_tx: std_mpsc::Sender<anyhow::Result<Vec<u8>>>,
    },
    GetBehaviour {
        response_tx: std_mpsc::Sender<anyhow::Result<shared::behaviour::Behaviour>>,
    },
}

/// Selects one candidate using sparse per-id weights. Missing ids retain the ordinary weight of
/// one. Zero explicitly removes an item from the draw; malformed negative/non-finite values are
/// ignored as if absent. Scaling by the largest weight keeps a set of individually finite values
/// from overflowing when summed.
fn weighted_media(media: Vec<Media>, weights: &HashMap<u64, f64>) -> Option<Media> {
    if media.is_empty() {
        return None;
    }
    let resolved = |item: &Media| match weights.get(&item.id).copied() {
        Some(weight) if weight.is_finite() && weight >= 0.0 => weight,
        _ => 1.0,
    };
    let maximum = media.iter().map(&resolved).fold(0.0_f64, f64::max);
    if maximum == 0.0 {
        let count = media.len();
        return media.into_iter().nth(rand::random_range(0..count));
    }
    let total: f64 = media.iter().map(|item| resolved(item) / maximum).sum();
    let mut draw = rand::random_range(0.0..total);
    let fallback = media.iter().rfind(|item| resolved(item) > 0.0)?.id;
    for item in media {
        let weight = resolved(&item) / maximum;
        if weight > 0.0 && draw < weight {
            return Some(item);
        }
        draw -= weight;
        if item.id == fallback {
            return Some(item);
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::MediaData;

    fn audio(id: u64) -> Media {
        Media {
            id,
            name: format!("{id}.ogg"),
            tags: Vec::new(),
            media_data: MediaData::Audio { duration: 1.0 },
        }
    }

    #[test]
    fn weighted_selection_excludes_zero_weight_candidates() {
        let weights = HashMap::from([(1, 0.0), (2, 5.0), (3, 0.0)]);
        for _ in 0..100 {
            assert_eq!(
                weighted_media(vec![audio(1), audio(2), audio(3)], &weights)
                    .unwrap()
                    .id,
                2
            );
        }
    }

    #[test]
    fn weighted_selection_keeps_sparse_and_malformed_weights_safe() {
        let sparse = HashMap::from([(1, f64::NAN), (2, -1.0)]);
        assert!(weighted_media(vec![audio(1), audio(2), audio(3)], &sparse).is_some());

        let all_zero = HashMap::from([(1, 0.0), (2, 0.0)]);
        assert!(weighted_media(vec![audio(1), audio(2)], &all_zero).is_some());
        assert!(weighted_media(Vec::new(), &HashMap::new()).is_none());
    }
}
