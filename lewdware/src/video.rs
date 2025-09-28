use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    thread,
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use ffmpeg::{codec, format, software};
use ffmpeg_next::{self as ffmpeg, Packet, frame::Video, threading};

use crate::{
    audio::{AudioMessage, spawn_audio_thread},
    media,
};

/// A video decoder using ffmpeg. Audio syncing is only done when the video loops (the audio thread
/// waits for the video to finish before looping). This is because:
/// 1. Keeping the audio in sync can cause it to sound disjointed and jarring.
/// 2. Compared to a dedicated video player, in this app, the user is unlikely to be focused too
///    much on whether the video and audio are in sync. This is compounded by the fact that audio
///    is likely to only become out of sync when there are lots of popups on screen or when the
///    video has been playing for a while.
///
/// * `receiver`: Receives frames from the ffmpeg thread.
/// * `audio_message_tx`: Sends messages to the audio thread.
pub struct VideoDecoder {
    receiver: Receiver<Option<VideoFrame>>,
    audio_message_tx: Option<SyncSender<AudioMessage>>,
    _video: media::Video,
    width: i64,
    height: i64,
}

pub struct VideoFrame {
    pub frame: Video,
    pub duration: Duration,
}

impl VideoDecoder {
    pub fn new(video: media::Video, play_audio: bool) -> Result<Self> {
        let path = video.file.path();

        let receiver = spawn_video_stream(path.to_path_buf());

        let audio_message_tx = if play_audio {
            let (tx, rx) = sync_channel(10);

            spawn_audio_thread(path.to_path_buf(), rx, true);

            Some(tx)
        } else {
            None
        };

        let width = video.width;
        let height = video.height;

        Ok(Self {
            receiver,
            width,
            _video: video,
            height,
            audio_message_tx,
        })
    }

    /// Get the next frame, if it's ready.
    pub fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        loop {
            match self.receiver.try_recv() {
                Ok(message) => match message {
                    Some(frame) => return Ok(Some(frame)),
                    // The decoding thread sends `None` when the video ends, so we should tell the
                    // audio thread to loop.
                    None => {
                        if let Some(tx) = self.audio_message_tx.as_ref() {
                            let _ = tx.try_send(AudioMessage::Loop);
                        }
                    }
                },
                Err(TryRecvError::Empty) => return Ok(None),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Utility method to copy a video frame into a buffer.
    pub fn copy_frame(&self, frame: &Video, buf: &mut [u8]) -> Result<()> {
        let width = self.width as usize;
        let height = self.height as usize;
        let line_size = frame.stride(0); // Bytes per row
        let data = frame.data(0);

        // Copy row-by-row into a contiguous Vec
        for (row_idx, chunk) in buf.chunks_exact_mut(width * 4).enumerate() {
            if row_idx >= height {
                break;
            }

            let src_start = row_idx * line_size;
            let src_end = src_start + width * 4;

            if src_end <= data.len() {
                chunk.copy_from_slice(&data[src_start..src_end]);
            } else {
                bail!("Invalid stride");
            }
        }

        Ok(())
    }

    pub fn pause(&self) {
        if let Some(tx) = self.audio_message_tx.as_ref() {
            let _ = tx.try_send(AudioMessage::Pause);
        }
    }

    pub fn play(&self) {
        if let Some(tx) = self.audio_message_tx.as_ref() {
            let _ = tx.try_send(AudioMessage::Play);
        }
    }
}

/// Spawn a thread to decode frames from a video.
fn spawn_video_stream(path: PathBuf) -> Receiver<Option<VideoFrame>> {
    let (tx, rx) = sync_channel(20);

    thread::spawn(move || {
        if let Err(err) = decode_video(path, tx) {
            eprintln!("Error decoding video: {}", err);
        }
    });

    rx
}

fn decode_video(path: PathBuf, tx: SyncSender<Option<VideoFrame>>) -> Result<()> {
    ffmpeg::init()?;
    let mut ictx = format::input(&path)?;
    let stream_index = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| anyhow!("Couldn't find video stream"))?
        .index();

    let video_stream = ictx.stream(stream_index).unwrap();
    let context_decoder = codec::Context::from_parameters(video_stream.parameters())?;
    let mut decoder = context_decoder.decoder().video()?;
    // Set up multi-threading
    let mut threading = decoder.threading();

    threading.count = 0;
    threading.kind = threading::Type::Frame;

    decoder.set_threading(threading);

    let mut scaler = software::scaling::context::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        ffmpeg::format::Pixel::RGBA,
        decoder.width(),
        decoder.height(),
        software::scaling::flag::Flags::BILINEAR,
    )?;

    loop {
        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_index {
                decoder.send_packet(&packet)?;
                let mut decoded = Video::empty();
                if decoder.receive_frame(&mut decoded).is_ok() {
                    let mut frame = Video::empty();
                    scaler.run(&decoded, &mut frame)?;

                    let duration = frame_duration(&packet, &stream);

                    tx.send(Some(VideoFrame { frame, duration }))?;
                }
            }
        }

        tx.send(None)?;
        ictx.seek(0, ..0)?;
        decoder.flush();
    }
}

fn frame_duration(packet: &Packet, stream: &ffmpeg::Stream) -> Duration {
    let duration_ticks = packet.duration();
    if duration_ticks > 0 {
        // Convert packet duration to seconds using the stream time base
        let tb = stream.time_base();
        let seconds = duration_ticks as f64 * tb.0 as f64 / tb.1 as f64;
        Duration::from_secs_f64(seconds)
    } else {
        println!("No explicit duration, falling back to average frame rate");
        // No explicit duration: fallback to avg_frame_rate
        let rate = stream.avg_frame_rate();
        let fps = if rate.1 != 0 {
            rate.0 as f64 / rate.1 as f64
        } else {
            30.0 // default to 30fps
        };
        Duration::from_secs_f64(1.0 / fps)
    }
}
