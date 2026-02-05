use std::{
    path::PathBuf,
    sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use ffmpeg::{codec, format, software};
use ffmpeg_next::{self as ffmpeg, frame::Video};

use crate::{audio::AudioPlayer, media::FileOrPath};

/// A video decoder using ffmpeg.
///
/// Audio is used as the master clock for synchronization.
///
/// * If the video is ahead of the audio, playback will pause on the current frame until the audio
/// catches up.
/// * If the video is behind the audio, frames will be skipped until the video is back in sync.
pub struct VideoDecoder {
    receiver: Receiver<Option<VideoFrame>>,
    _video: FileOrPath,
    audio_player: Option<AudioPlayer>,
    tolerance: Duration,
    last_frame_time: Instant,
    frame_duration: Duration,
    video_clock: Duration,
    width: u32,
    height: u32,
    native_width: u32,
    native_height: u32,
    paused: bool,
    on_finish: Option<Box<dyn FnMut() + Send>>,
    video_duration: Duration,
}

pub struct VideoFrame {
    pub frame: Video,
    pub pts: Duration,
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

        let (receiver, video_duration, native_width, native_height) =
            spawn_video_stream(path.to_path_buf(), loop_video)?;

        let audio_player = if play_audio {
            match AudioPlayer::new(path.to_path_buf(), loop_video) {
                Ok(audio_player) => Some(audio_player),
                Err(err) => {
                    eprintln!("{err}");
                    None
                }
            }
        } else {
            None
        };

        Ok(Self {
            receiver,
            width,
            height,
            native_width,
            native_height,
            audio_player,
            last_frame_time: Instant::now(),
            frame_duration: Duration::ZERO,
            video_clock: Duration::ZERO,
            tolerance: Duration::from_millis(200),
            _video: video,
            paused: true,
            on_finish: None,
            video_duration,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn native_width(&self) -> u32 {
        self.native_width
    }

    pub fn native_height(&self) -> u32 {
        self.native_height
    }

    pub fn on_finish(&mut self, f: impl FnMut() + Send + 'static) {
        self.on_finish = Some(Box::new(f));
    }

    /// Get the next frame, if it's ready.
    pub fn next_frame(&mut self) -> NextFrame {
        if self.paused {
            return NextFrame::None;
        }

        if !self.needs_next_frame() {
            // Video is ahead of or in sync with audio. Ensure audio is playing.
            if let Some(audio_player) = &self.audio_player {
                audio_player.play();
            }
            return NextFrame::None;
        }

        let frame = loop {
            match self.receiver.try_recv() {
                Ok(Some(frame)) => {
                    // We got a frame, so if we were waiting for it, resume the audio.
                    if let Some(audio_player) = &self.audio_player {
                        audio_player.play();
                    }

                    // If there's no audio, we just display the frame since needs_next_frame said so.
                    if self.audio_player.is_none() {
                        break frame;
                    }

                    // With audio, we might need to drop frames if we are too far behind.
                    if let Some(audio_player) = &self.audio_player {
                        // Compare directly with total audio position
                        if frame.pts < audio_player.position().saturating_sub(self.tolerance) {
                            continue;
                        }
                    }
                    break frame;
                }
                Ok(None) => {
                    if let Some(on_finish) = &mut self.on_finish {
                        on_finish();
                    }
                    // End of stream from decoder. If looping, it will start again.
                    return NextFrame::None;
                }
                Err(TryRecvError::Empty) => {
                    // The decoder is lagging behind the audio, so we pause the audio to wait for it.
                    if let Some(audio_player) = &self.audio_player {
                        audio_player.pause();
                    }
                    return NextFrame::None;
                }
                Err(TryRecvError::Disconnected) => return NextFrame::Finish,
            }
        };

        let next_pts = frame.pts;

        if self.audio_player.is_none() {
            if self.video_clock > Duration::ZERO {
                self.frame_duration = next_pts.saturating_sub(self.video_clock);
            }
            // For the very first frame, frame_duration will be zero, but last_frame_time was
            // set at creation, so needs_next_frame will return true immediately.
        }

        self.video_clock = next_pts;
        self.last_frame_time = Instant::now();

        NextFrame::Ready(frame)
    }

    fn needs_next_frame(&self) -> bool {
        match &self.audio_player {
            Some(audio_player) => {
                // Compare directly with total audio position
                audio_player.position() > self.video_clock
            }
            None => self.last_frame_time.elapsed() >= self.frame_duration,
        }
    }

    pub fn pause(&mut self) {
        if let Some(audio_player) = &self.audio_player {
            audio_player.pause();
        }
        self.paused = true;
    }

    pub fn play(&mut self) {
        if let Some(audio_player) = &self.audio_player {
            audio_player.play();
        }
        self.paused = false;
    }
}

pub enum NextFrame {
    Ready(VideoFrame),
    Finish,
    None,
}

struct VideoMetadata {
    duration: Duration,
    width: u32,
    height: u32,
}

/// Spawn a thread to decode frames from a video.
fn spawn_video_stream(
    path: PathBuf,
    loop_video: bool,
) -> Result<(Receiver<Option<VideoFrame>>, Duration, u32, u32)> {
    // Now returns Result
    let (tx, rx) = sync_channel(10);
    let (meta_tx, meta_rx) = sync_channel(1);

    thread::spawn(move || {
        let video_duration_inner = match get_video_duration(&path) {
            Ok(duration) => duration,
            Err(err) => {
                eprintln!("Failed to get video duration: {err}");
                // Can't send metadata if we can't even get duration, but we'll let decode_video fail or we fail here.
                // If we return, receiver is closed.
                return;
            }
        };

        if let Err(err) = decode_video(path, tx, loop_video, video_duration_inner, meta_tx) {
            eprintln!("Error decoding video: {}", err);
        }
    });

    // Receive metadata from the spawned thread
    let meta = meta_rx
        .recv()
        .context("Failed to receive video metadata from spawn thread")?;

    Ok((rx, meta.duration, meta.width, meta.height))
}

// New helper function to get video duration
fn get_video_duration(path: &PathBuf) -> Result<Duration> {
    ffmpeg::init()?;
    let ictx = format::input(path)?;
    let duration_us = ictx.duration(); // Duration in microseconds

    // ffmpeg's duration is in AV_TIME_BASE units, which is 1,000,000 (microseconds).
    let duration_seconds = duration_us as f64 / 1_000_000.0;
    Ok(Duration::from_secs_f64(duration_seconds))
}

fn decode_video(
    path: PathBuf,
    tx: SyncSender<Option<VideoFrame>>,
    loop_video: bool,
    video_duration: Duration, // New parameter
    meta_tx: SyncSender<VideoMetadata>,
) -> Result<()> {
    ffmpeg::init()?;
    let mut ictx = format::input(&path)?;
    let stream_index = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .context("Couldn't find video stream")?
        .index();

    let video_stream = ictx.stream(stream_index).context("Invalid stream index")?;
    let time_base = video_stream.time_base();
    let avg_frame_rate = video_stream.avg_frame_rate();
    let frame_duration = if avg_frame_rate.numerator() > 0 {
        Duration::from_secs_f64(avg_frame_rate.denominator() as f64 / avg_frame_rate.numerator() as f64)
    } else {
        Duration::from_millis(33)
    };

    let context_decoder = codec::Context::from_parameters(video_stream.parameters())?;
    let mut decoder = context_decoder.decoder().video()?;

    // Enable multi-threaded decoding
    decoder.set_threading(codec::threading::Config {
        kind: codec::threading::Type::Frame,
        count: thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0),
    });

    let mut scaler = software::scaling::context::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        ffmpeg::format::Pixel::RGBA,
        decoder.width(),
        decoder.height(),
        software::scaling::flag::Flags::FAST_BILINEAR,
    )?;

    // Send metadata
    if meta_tx
        .send(VideoMetadata {
            duration: video_duration,
            width: decoder.width(),
            height: decoder.height(),
        })
        .is_err()
    {
        eprintln!("Failed to send video metadata");
        return Ok(());
    }

    let mut decoded = Video::empty();

    let mut current_loop_offset = Duration::ZERO; // Initialize loop offset
    let mut last_pts_duration = Duration::ZERO;

    'main: loop {
        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_index {
                decoder.send_packet(&packet)?;
                while decoder.receive_frame(&mut decoded).is_ok() {
                    let mut frame = Video::empty();
                    scaler.run(&decoded, &mut frame)?;

                    let pts = decoded.pts().unwrap_or(0);
                    let pts_seconds = pts as f64 * (time_base.0 as f64 / time_base.1 as f64);
                    let pts_duration = Duration::from_secs_f64(pts_seconds);
                    last_pts_duration = pts_duration;

                    // Add current_loop_offset to pts
                    if tx
                        .send(Some(VideoFrame {
                            frame,
                            pts: pts_duration + current_loop_offset,
                        }))
                        .is_err()
                    {
                        break 'main;
                    }
                }
            }
        }

        decoder.flush();

        while decoder.receive_frame(&mut decoded).is_ok() {
            let mut frame = Video::empty();
            scaler.run(&decoded, &mut frame)?;

            let pts = decoded.pts().unwrap_or(0);
            let pts_seconds = pts as f64 * (time_base.0 as f64 / time_base.1 as f64);
            let pts_duration = Duration::from_secs_f64(pts_seconds);
            last_pts_duration = pts_duration;

            // Add current_loop_offset to pts
            if tx
                .send(Some(VideoFrame {
                    frame,
                    pts: pts_duration + current_loop_offset,
                }))
                .is_err()
            {
                break 'main;
            }
        }

        if tx.send(None).is_err() {
            break 'main;
        }

        if !loop_video {
            return Ok(());
        }

        ictx.seek(0, ..0)?;
        decoder.flush();
        current_loop_offset += last_pts_duration + frame_duration; // Increment loop offset for next loop
    }

    Ok(())
}

