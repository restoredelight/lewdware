use std::{path::Path, rc::Rc, thread};

use anyhow::Result;
use async_channel::{Receiver, Sender, bounded};
use async_executor::LocalExecutor;
use futures_lite::future::block_on;
use pack_format::config::Metadata;
use tempfile::NamedTempFile;
use winit::event_loop::EventLoopProxy;

use crate::{
    app::UserEvent,
    media::pack::{Audio, Link, Media, MediaPack, Notification, Prompt},
};

/// Manages all the media (images, audio, videos). We use a message system to avoid blocking the
/// event loop on the main thread: requests are sent to a separate thread, which handles reading
/// and decoding, and then sends back responses.
///
/// The [UserEvent::MediaResponse] event will be sent to the event loop whenever a response is
/// available.
pub struct MediaManager {
    tx: Sender<Request>,
    rx: Receiver<Response>,
    id: u64,
}

impl MediaManager {
    /// Start up the media manager thread, opening the specified pack file. Returns the pack
    /// metadata.
    pub fn open(
        pack_path: &Path,
        event_loop_proxy: EventLoopProxy<UserEvent>,
    ) -> Result<(Self, Metadata)> {
        let (tx, rx, metadata) = spawn_media_manager_thread(pack_path, event_loop_proxy)?;

        Ok((Self { tx, rx, id: 0 }, metadata))
    }

    /// Request the media for an image or video popup. Returns the id of the request, or [None] if
    /// the request failed to send (which can happen if the thread is processing too many messages
    /// at once).
    ///
    /// * `only_images`: Whether to allow videos
    pub fn request_media(&mut self, tags: Option<Vec<String>>, only_images: bool) -> Option<u64> {
        self.try_send(MediaRequest::RandomMedia { only_images, tags })
    }

    pub fn request_audio(&mut self, tags: Option<Vec<String>>) -> Option<u64> {
        self.try_send(MediaRequest::RandomAudio { tags })
    }

    pub fn request_notification(&mut self, tags: Option<Vec<String>>) -> Option<u64> {
        self.try_send(MediaRequest::RandomNotification { tags })
    }

    pub fn request_link(&mut self, tags: Option<Vec<String>>) -> Option<u64> {
        self.try_send(MediaRequest::RandomLink { tags })
    }

    pub fn request_prompt(&mut self, tags: Option<Vec<String>>) -> Option<u64> {
        self.try_send(MediaRequest::RandomPrompt { tags })
    }

    pub fn request_wallpaper(&mut self, tags: Option<Vec<String>>) -> Option<u64> {
        self.try_send(MediaRequest::RandomWallpaper { tags })
    }

    fn try_send(&mut self, request: MediaRequest) -> Option<u64> {
        let id = self.id;

        match self.tx.try_send(Request { id, request }) {
            Ok(()) => {
                self.id = self.id.wrapping_add(1);
                Some(id)
            }
            Err(_) => None,
        }
    }

    /// Returns a response if there is one.
    pub fn try_recv(&self) -> Option<Response> {
        self.rx.try_recv().ok()
    }
}

fn spawn_media_manager_thread(
    pack_path: &Path,
    event_loop_proxy: EventLoopProxy<UserEvent>,
) -> Result<(Sender<Request>, Receiver<Response>, Metadata)> {
    let (req_tx, req_rx) = bounded(20);
    let (resp_tx, resp_rx) = bounded(20);

    let file = MediaPack::open(pack_path)?;
    let metadata = file.metadata().clone();

    thread::spawn(move || {
        let executor = Rc::new(LocalExecutor::new());
        let ex = executor.clone();

        block_on(executor.run(async move {
            let manager = Rc::new(file);

            while let Ok(request) = req_rx.recv().await {
                let resp_tx = resp_tx.clone();
                let manager = manager.clone();

                let event_loop_proxy = event_loop_proxy.clone();

                ex.spawn(async move {
                    let response = handle_request(manager, request).await.unwrap();

                    if let Some(response) = response {
                        match resp_tx.send(response).await {
                            Ok(_) => {
                                if let Err(err) =
                                    event_loop_proxy.send_event(UserEvent::MediaResponse)
                                {
                                    eprintln!("{}", err);
                                };
                            }
                            Err(err) => {
                                eprintln!("{}", err);
                            }
                        }
                    }
                })
                .detach();
            }
        }));
    });

    Ok((req_tx, resp_rx, metadata))
}

async fn handle_request(pack: Rc<MediaPack>, request: Request) -> Result<Option<Response>> {
    let response = match request.request {
        MediaRequest::RandomMedia { only_images, tags } => {
            if only_images {
                pack.get_random_image(tags)
                    .await
                    .map(|x| x.map(|image| MediaResponse::Media(Media::Image(image))))
            } else {
                pack.get_random_popup(tags)
                    .await
                    .map(|x| x.map(MediaResponse::Media))
            }
        }
        MediaRequest::RandomAudio { tags } => pack
            .get_random_audio(tags)
            .await
            .map(|x| x.map(MediaResponse::Audio)),
        MediaRequest::RandomNotification { tags } => pack
            .get_random_notification(tags)
            .map(|x| x.map(MediaResponse::Notification)),
        MediaRequest::RandomPrompt { tags } => pack
            .get_random_prompt(tags)
            .map(|x| x.map(MediaResponse::Prompt)),
        MediaRequest::RandomLink { tags } => pack
            .get_random_link(tags)
            .map(|x| x.map(MediaResponse::Link)),
        MediaRequest::RandomWallpaper { tags } => pack
            .get_random_wallpaper(tags)
            .await
            .map(|x| x.map(MediaResponse::Wallpaper)),
    };

    response.map(|x| {
        x.map(|response| Response {
            id: request.id,
            response,
        })
    })
}

struct Request {
    pub id: u64,
    pub request: MediaRequest,
}

pub struct Response {
    pub id: u64,
    pub response: MediaResponse,
}

#[derive(Debug, Clone)]
enum MediaRequest {
    RandomMedia {
        only_images: bool,
        tags: Option<Vec<String>>,
    },
    RandomAudio {
        tags: Option<Vec<String>>,
    },
    RandomNotification {
        tags: Option<Vec<String>>,
    },
    RandomPrompt {
        tags: Option<Vec<String>>,
    },
    RandomLink {
        tags: Option<Vec<String>>,
    },
    RandomWallpaper {
        tags: Option<Vec<String>>,
    },
}

pub enum MediaResponse {
    Media(Media),
    Audio(Audio),
    Notification(Notification),
    Prompt(Prompt),
    Link(Link),
    Wallpaper(NamedTempFile),
}
