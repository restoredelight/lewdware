//! Each preview owns its command queue, events and cancellation flag. Replacing a preview
//! drops the receiver, so a late startup/frame can never appear in a different selection.
use crate::{
    MediaSource,
    audio::{AudioOut, AudioPlayer},
    video::{NextFrame, Options, VideoDecoder},
};
use gpui_kit::RenderImage;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video { transparent: bool },
    Audio,
}

#[derive(Clone)]
pub struct PreviewSource {
    pub media: MediaSource,
    pub kind: MediaKind,
    pub duration: Option<Duration>,
}

pub(crate) enum Command {
    Poll,
    Play(bool),
    Seek(Duration),
    Volume(f32),
    Loop(bool),
    Stop,
}
pub(crate) enum Event {
    Ready {
        duration: Option<Duration>,
    },
    Frame {
        image: Arc<RenderImage>,
        position: Duration,
    },
    Position(Duration),
    Ended,
    Error(String),
    Warning(String),
}

pub(crate) struct Worker {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
    poll_pending: Arc<AtomicBool>,
}

impl Worker {
    pub fn start(source: PreviewSource, volume: f32, looping: bool) -> anyhow::Result<Self> {
        let (commands, rx) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let poll_pending = Arc::new(AtomicBool::new(false));
        let (stop, pending) = (cancelled.clone(), poll_pending.clone());
        thread::Builder::new()
            .name("media-preview".into())
            .spawn(move || {
                if let Err(error) = run(source, volume, looping, rx, &tx, &stop, &pending) {
                    if !stop.load(Ordering::Acquire) {
                        let _ = tx.send(Event::Error(format!("{error:#}")));
                    }
                }
            })?;
        Ok(Self {
            commands,
            events,
            cancelled,
            poll_pending,
        })
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn poll(&self) {
        if !self.poll_pending.swap(true, Ordering::AcqRel) {
            self.send(Command::Poll);
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        self.send(Command::Stop);
    }
}

fn open_video(
    source: &PreviewSource,
    start: Duration,
    volume: f32,
    looping: Arc<AtomicBool>,
    out: Option<&AudioOut>,
) -> anyhow::Result<VideoDecoder> {
    VideoDecoder::new(
        Options {
            source: source.media.clone(),
            packed_alpha: matches!(source.kind, MediaKind::Video { transparent: true }),
            volume,
            start,
            threads: 0,
            // Native-size uploads are the measured baseline. No renderer or texture crosses threads.
            target_size: Arc::new(Default::default()),
        },
        looping,
        out,
    )
}

fn run(
    source: PreviewSource,
    mut volume: f32,
    looping: bool,
    commands: Receiver<Command>,
    events: &Sender<Event>,
    cancelled: &AtomicBool,
    poll_pending: &AtomicBool,
) -> anyhow::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    ffmpeg_next::init()?;
    if source.kind == MediaKind::Image {
        let rgba = image::load_from_memory(&source.media.read()?)?.into_rgba8();
        let native = rgba.dimensions();
        let mut bytes = rgba.into_raw();
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let image = Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
            image::RgbaImage::from_raw(native.0, native.1, bytes).unwrap()
        )]));
        if !cancelled.load(Ordering::Acquire) {
            let _ = events.send(Event::Ready { duration: None });
            let _ = events.send(Event::Frame {
                image,
                position: Duration::ZERO,
            });
        }
        return Ok(());
    }
    let out = match AudioOut::open() {
        Ok(out) => Some(out),
        Err(error) if source.kind != MediaKind::Audio => {
            let _ = events.send(Event::Warning(format!("Audio unavailable: {error:#}")));
            None
        }
        Err(error) => return Err(error),
    };
    if cancelled.load(Ordering::Acquire) {
        return Ok(());
    }
    let looping = Arc::new(AtomicBool::new(looping));
    let mut video = if matches!(source.kind, MediaKind::Video { .. }) {
        Some(open_video(
            &source,
            Duration::ZERO,
            volume,
            looping.clone(),
            out.as_ref(),
        )?)
    } else {
        None
    };
    let mut audio = if source.kind == MediaKind::Audio {
        AudioPlayer::new(
            out.as_ref().unwrap(),
            source.media.clone(),
            looping.clone(),
            volume,
            Duration::ZERO,
        )?
    } else {
        None
    };
    anyhow::ensure!(
        video.is_some() || audio.is_some(),
        "This file has no playable audio stream"
    );
    let duration = video.as_ref().and_then(|v| v.duration).or(source.duration);
    let _ = events.send(Event::Ready { duration });
    let mut playing = false;
    let mut preview = video.is_some();

    while let Ok(command) = commands.recv() {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        match command {
            Command::Stop => break,
            Command::Play(play) => {
                playing = play;
                if let Some(v) = &mut video {
                    if play && !preview {
                        v.play();
                    } else {
                        v.pause();
                    }
                }
                if let Some(a) = &audio {
                    if play {
                        a.play();
                    } else {
                        a.pause();
                    }
                }
            }
            Command::Volume(value) => {
                volume = value;
                if let Some(v) = &mut video {
                    v.set_volume(value);
                }
                if let Some(a) = &audio {
                    a.set_volume(value);
                }
            }
            Command::Loop(value) => looping.store(value, Ordering::Relaxed),
            Command::Seek(to) => {
                // Stop before reopening. The stable source handle pins the same archive generation.
                if video.take().is_some() {
                    video = Some(open_video(
                        &source,
                        to,
                        volume,
                        looping.clone(),
                        out.as_ref(),
                    )?);
                    preview = true;
                } else {
                    drop(audio.take());
                    audio = AudioPlayer::new(
                        out.as_ref().unwrap(),
                        source.media.clone(),
                        looping.clone(),
                        volume,
                        to,
                    )?;
                    if playing && !cancelled.load(Ordering::Acquire) {
                        if let Some(a) = &audio {
                            a.play();
                        }
                    }
                    let _ = events.send(Event::Position(to));
                }
            }
            Command::Poll => {
                poll_pending.store(false, Ordering::Release);
                if let Some(v) = &mut video {
                    if let Some(error) = v.stats.error.lock().unwrap().take() {
                        anyhow::bail!(error);
                    }
                    let frame = if preview {
                        v.take_preview().inspect(|_| {
                            preview = false;
                            if playing && !cancelled.load(Ordering::Acquire) {
                                v.play();
                            }
                        })
                    } else if playing {
                        match v.next_frame() {
                            NextFrame::Ready(frame) => Some(frame),
                            NextFrame::Finish => {
                                playing = false;
                                v.pause();
                                let _ = events.send(Event::Ended);
                                None
                            }
                            NextFrame::None => None,
                        }
                    } else {
                        None
                    };
                    if preview && frame.is_none() && v.stats.finished.load(Ordering::Acquire) {
                        anyhow::bail!("No video frame could be decoded");
                    }
                    if let Some(frame) = frame {
                        let _ = events.send(Event::Frame {
                            image: frame.image,
                            position: frame.pass_pts,
                        });
                    }
                } else if let Some(a) = &audio {
                    if playing {
                        if a.empty() {
                            playing = false;
                            let _ = events.send(Event::Ended);
                            continue;
                        }
                        let mut position = a.position();
                        if let Some(d) = duration {
                            if looping.load(Ordering::Relaxed) && !d.is_zero() {
                                position = Duration::from_secs_f64(
                                    position.as_secs_f64() % d.as_secs_f64(),
                                );
                            }
                        }
                        let _ = events.send(Event::Position(position));
                    }
                }
            }
        }
    }
    // AudioOut, the players and decoder receivers drop on this worker, never blocking GPUI.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn bad_image_reports_an_error_instead_of_waiting_forever() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"not an image").unwrap();
        let worker = Worker::start(
            PreviewSource {
                media: MediaSource::from_file(file, 0, 12),
                kind: MediaKind::Image,
                duration: None,
            },
            0.,
            false,
        )
        .unwrap();
        assert!(matches!(
            worker.events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Error(_)
        ));
    }

    #[test]
    fn image_worker_decodes_only_the_pinned_range_to_bgra() {
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 128]))
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"prefix").unwrap();
        file.write_all(encoded.get_ref()).unwrap();
        file.write_all(b"suffix").unwrap();
        let worker = Worker::start(
            PreviewSource {
                media: MediaSource::from_file(file, 6, encoded.get_ref().len() as u64),
                kind: MediaKind::Image,
                duration: None,
            },
            0.,
            false,
        )
        .unwrap();
        assert!(matches!(
            worker.events.recv_timeout(Duration::from_secs(3)).unwrap(),
            Event::Ready { .. }
        ));
        let Event::Frame { image, .. } =
            worker.events.recv_timeout(Duration::from_secs(3)).unwrap()
        else {
            panic!("expected image")
        };
        assert_eq!(image.as_bytes(0).unwrap(), [0, 0, 255, 128].repeat(4));
    }
}
