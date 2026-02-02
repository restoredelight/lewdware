use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg::{codec, format, software};
use ffmpeg_next::{self as ffmpeg, Packet, frame::Video};
use tiny_skia::{Pixmap, PixmapMut};
use winit::window::Window;

use crate::{
    audio::{AudioMessage, spawn_audio_thread},
    media::FileOrPath,
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
    _video: FileOrPath,
    width: u32,
    height: u32,
}

pub struct VideoFrame {
    pub frame: Video,
    pub duration: Duration,
}

impl VideoDecoder {
    pub fn new(
        video: FileOrPath,
        width: u32,
        height: u32,
        play_audio: bool,
        loop_video: bool,
    ) -> Result<Self> {
        let path = video.path();

        let receiver = spawn_video_stream(path.to_path_buf(), width, height, loop_video);

        let audio_message_tx = if play_audio {
            let (tx, rx) = sync_channel(10);

            spawn_audio_thread(path.to_path_buf(), rx, loop_video);

            Some(tx)
        } else {
            None
        };

        Ok(Self {
            receiver,
            width,
            height,
            _video: video,
            audio_message_tx,
        })
    }

    /// Get the next frame, if it's ready.
    pub fn next_frame(&mut self) -> NextFrame {
        match self.receiver.try_recv() {
            Ok(message) => match message {
                Some(frame) => NextFrame::Ready(frame),
                // The decoding thread sends `None` when the video ends, so we should tell the
                // audio thread to loop.
                None => {
                    if let Some(tx) = self.audio_message_tx.as_ref() {
                        let _ = tx.try_send(AudioMessage::Loop);
                    }

                    NextFrame::Finish
                }
            },
            Err(TryRecvError::Empty) => NextFrame::None,
            Err(TryRecvError::Disconnected) => return NextFrame::Disconnected,
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

    pub fn create_pixmap(&self, frame: &Video) -> Result<Pixmap> {
        let mut pixmap = Pixmap::new(self.width, self.height).unwrap();

        self.copy_frame(frame, pixmap.data_mut())?;

        Ok(pixmap)
    }

    pub fn copy_frame_pixmap(
        &self,
        frame: &Video,
        pixmap: &mut PixmapMut<'_>,
        y_offset: u32,
        border_size: u32,
    ) -> Result<()> {
        let width = self.width as usize;
        let height = self.height as usize;
        let line_size = frame.stride(0); // Bytes per row
        let data = frame.data(0);

        let buf_width = pixmap.width();
        let buf = pixmap.data_mut();

        // Copy row-by-row into a contiguous Vec
        for row_idx in 0..height {
            let src_start = row_idx * line_size;
            let src_end = src_start + width * 4;

            let buf_start = (buf_width * (y_offset + row_idx as u32) + border_size) as usize;

            if src_end <= data.len() {
                buf[buf_start..buf_start + width].copy_from_slice(&data[src_start..src_end]);
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

    pub fn copy_frame_softbuffer(
        &self,
        frame: &Video,
        buf: &mut softbuffer::Buffer<'_, Arc<Window>, Arc<Window>>,
    ) -> Result<()> {
        let width = self.width as usize;
        let line_size = frame.stride(0); // Bytes per row
        let data = frame.data(0);

        // Copy row-by-row into a contiguous Vec
        for (row_idx, chunk) in buf.chunks_exact_mut(width).enumerate() {
            let src_start = row_idx * line_size;
            let src_end = src_start + width * 4;

            for (i, pixel) in data[src_start..src_end].chunks_exact(4).enumerate() {
                let r = pixel[0] as u32;
                let g = pixel[1] as u32;
                let b = pixel[2] as u32;
                let a = pixel[3] as u32;

                chunk[i] = (a << 24) | (r << 16) | (g << 8) | b;
            }
        }

        Ok(())
    }
}

pub enum NextFrame {
    Ready(VideoFrame),
    Finish,
    None,
    Disconnected,
}

/// Spawn a thread to decode frames from a video.
fn spawn_video_stream(
    path: PathBuf,
    width: u32,
    height: u32,
    loop_video: bool,
) -> Receiver<Option<VideoFrame>> {
    let (tx, rx) = sync_channel(10);

    thread::spawn(move || {
        if let Err(err) = decode_video(path, tx, width, height, loop_video) {
            eprintln!("Error decoding video: {}", err);
        }
    });

    rx
}

fn decode_video(
    path: PathBuf,
    tx: SyncSender<Option<VideoFrame>>,
    width: u32,
    height: u32,
    loop_video: bool,
) -> Result<()> {
    ffmpeg::init()?;
    let mut ictx = format::input(&path)?;
    let stream_index = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .context("Couldn't find video stream")?
        .index();

    let video_stream = ictx.stream(stream_index).context("Invalid stream index")?;
    let context_decoder = codec::Context::from_parameters(video_stream.parameters())?;
    let mut decoder = context_decoder.decoder().video()?;

    let mut scaler = software::scaling::context::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        ffmpeg::format::Pixel::RGBA,
        width,
        height,
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

        if !loop_video {
            return Ok(());
        }

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

pub fn copy_frame_pixmap(
    frame: &Video,
    pixmap: &mut PixmapMut<'_>,
    y_offset: u32,
    border_size: u32,
) -> Result<()> {
    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let line_size = frame.stride(0); // Bytes per row
    let data = frame.data(0);

    let buf_width = pixmap.width();
    let buf = pixmap.data_mut();

    // Copy row-by-row into a contiguous Vec
    for row_idx in 0..height {
        let src_start = row_idx * line_size;
        let src_end = src_start + width * 4;

        let buf_start = ((buf_width * (y_offset + row_idx as u32) + border_size) * 4) as usize;

        if src_end <= data.len() {
            buf[buf_start..buf_start + width * 4].copy_from_slice(&data[src_start..src_end]);
        } else {
            bail!("Invalid stride");
        }
    }

    Ok(())
}
