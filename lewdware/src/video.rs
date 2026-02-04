use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::AtomicU64,
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use ffmpeg::{codec, format, software};
use ffmpeg_next::{self as ffmpeg, Packet, frame::Video};
use rusqlite::fallible_iterator::FallibleIterator;

use crate::{
    audio::{AudioMessage, AudioPlayer},
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
    _video: FileOrPath,
    audio_player: Option<AudioPlayer>,
    tolerance: Duration,
    last_frame_time: Instant,
    frame_duration: Duration,
    position: Duration,
    width: u32,
    height: u32,
    paused: bool,
    on_finish: Option<Box<dyn FnMut() + Send>>
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
            audio_player,
            last_frame_time: Instant::now(),
            frame_duration: Duration::ZERO,
            position: Duration::ZERO,
            tolerance: Duration::from_millis(200),
            _video: video,
            paused: true,
            on_finish: None,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn on_finish(&mut self, f: impl FnMut() + Send + 'static) {
        self.on_finish = Some(Box::new(f));
    }

    /// Get the next frame, if it's ready.
    pub fn next_frame(&mut self) -> NextFrame {
        if self.paused || !self.needs_next_frame() {
            return NextFrame::None;
        }

        let frame = loop {
            match self.receiver.try_recv() {
                Ok(Some(frame)) => {
                    if self.audio_player.as_ref().is_none_or(|audio_player| {
                        self.position + frame.duration + self.tolerance >= audio_player.position()
                    }) {
                        break frame;
                    } else {
                        self.position += frame.duration;
                    }
                }
                Ok(None) => {
                    if let Some(on_finish) = &mut self.on_finish {
                        on_finish();
                    }
                }
                Err(TryRecvError::Empty) => return NextFrame::None,
                Err(TryRecvError::Disconnected) => return NextFrame::Finish,
            }
        };

        self.position += frame.duration;
        self.last_frame_time = Instant::now();
        self.frame_duration = frame.duration;

        NextFrame::Ready(frame)
    }

    fn needs_next_frame(&self) -> bool {
        match &self.audio_player {
            Some(audio_player) => audio_player.position() > self.position,
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

    let mut decoded = Video::empty();

    loop {
        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_index {
                decoder.send_packet(&packet)?;
                if decoder.receive_frame(&mut decoded).is_ok() {
                    let mut frame = Video::empty();
                    scaler.run(&decoded, &mut frame)?;

                    let duration = frame_duration(&packet, &stream);

                    tx.send(Some(VideoFrame { frame, duration }))?;
                }
            }
        }

        decoder.flush();

        while decoder.receive_frame(&mut decoded).is_ok() {
            println!("??");
            let mut frame = Video::empty();
            scaler.run(&decoded, &mut frame)?;

            // let duration = frame_duration(&frame.packet(), &video_stream);

            // tx.send(Some(VideoFrame { frame, duration }))?;
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
