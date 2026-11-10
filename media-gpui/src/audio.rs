//! Audio decoding and output, adapted from `lewdware/src/audio/decode.rs` and `player.rs`.
//!
//! What changed from the engine (see README.md, "Reuse"):
//!
//! * **Start offset (seeking).** `AudioFrames::open` takes `start`, seeks the input there and
//!   trims the samples before it. The engine only ever plays from zero.
//! * **One output device for the player's lifetime** (`AudioOut`), with a fresh `rodio::Player`
//!   per seek. The engine opens a device sink per playing item and never seeks; reopening the
//!   device on every seek would add its open latency (tens of ms on PulseAudio) to each one.
//! * `TrackClock` starts at `start`, so its position is media time rather than time since open.
//! * Cues, volume fades, the "finished" event and the configured-device lookup are dropped; the
//!   spike opens the default device.
//!
//! `convert_audio_frame`, `convert_samples`, the chunked decode thread and `TrackSource` (padding
//! silence that doesn't move the clock) are the engine's, unchanged apart from the cue removal.

use std::{
    num::NonZero,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use bytemuck::AnyBitPattern;
use ffmpeg_next::{
    self as ffmpeg,
    format::{Sample, sample},
    frame,
};
use rodio::{
    DeviceSinkBuilder, MixerDeviceSink, Player, Source, buffer::SamplesBuffer,
    source::UniformSourceIterator,
};

use crate::{bounded_input::BoundedInput, source::MediaSource};

/// The output device, opened once and shared by every `AudioPlayer` the spike makes.
pub struct AudioOut {
    stream: MixerDeviceSink,
}

impl AudioOut {
    pub fn open() -> Result<Self> {
        let mut stream = DeviceSinkBuilder::open_default_sink()?;
        stream.log_on_drop(false);
        Ok(Self { stream })
    }
}

pub struct AudioFrames {
    ictx: BoundedInput,
    decoder: ffmpeg::decoder::Audio,
    stream_index: usize,
    time_base: ffmpeg::Rational,
    frame: frame::Audio,
    empty_passes: u32,
    /// NEW: samples before this media time are dropped (first pass only, after a seek).
    skip_until: Option<Duration>,
}

const MAX_EMPTY_PASSES: u32 = 3;

pub enum Decoded {
    Samples(SamplesBuffer),
    Looped,
}

impl AudioFrames {
    /// Decodes pass after pass: whether the track loops is decided as it plays (`TrackSource`),
    /// not here, where the decoder runs ahead of playback.
    pub fn open(
        source: MediaSource,
        start: Duration,
    ) -> Result<Option<Self>> {
        ffmpeg::init()?;

        let mut ictx = source.open()?;
        let stream_index = match ictx.streams().best(ffmpeg::media::Type::Audio) {
            Some(stream) => stream.index(),
            None => return Ok(None),
        };

        let media = ictx.stream(stream_index).context("Invalid stream index")?;
        let time_base = media.time_base();
        let context = ffmpeg::codec::Context::from_parameters(media.parameters())?;
        let mut decoder = context.decoder().audio()?;
        decoder.set_packet_time_base(time_base);

        if !start.is_zero() {
            let ts = start.as_micros() as i64; // AV_TIME_BASE is microseconds
            ictx.seek(ts, ..ts).context("seeking the audio")?;
        }

        Ok(Some(Self {
            ictx,
            decoder,
            stream_index,
            time_base,
            frame: frame::Audio::empty(),
            empty_passes: 0,
            skip_until: (!start.is_zero()).then_some(start),
        }))
    }

    pub fn next(&mut self) -> Option<Decoded> {
        loop {
            if let Some(buffer) = self.take_ready() {
                self.empty_passes = 0;
                return Some(Decoded::Samples(buffer));
            }

            if let Some(packet) = self.next_packet() {
                if let Err(err) = self.decoder.send_packet(&packet) {
                    tracing::error!("Failed to send packet: {err}");
                }
                continue;
            }

            let _ = self.decoder.send_eof();

            if let Some(buffer) = self.take_ready() {
                self.empty_passes = 0;
                return Some(Decoded::Samples(buffer));
            }

            // CHANGED: always on to the next pass. The engine stops here when not looping, but the
            // preview's loop button can change between the decoder reaching the end (up to the
            // whole queue ahead, all of a short clip) and playback reaching it.
            self.empty_passes += 1;
            if self.empty_passes >= MAX_EMPTY_PASSES {
                tracing::error!("audio decoded nothing over {MAX_EMPTY_PASSES} passes");
                return None;
            }

            if let Err(err) = self.ictx.seek(0, ..0) {
                tracing::error!("Failed to seek to start: {err}");
            }
            self.decoder.flush();
            self.skip_until = None;

            return Some(Decoded::Looped);
        }
    }

    fn take_ready(&mut self) -> Option<SamplesBuffer> {
        while self.decoder.receive_frame(&mut self.frame).is_ok() {
            let mut samples = match convert_audio_frame(&self.frame) {
                Ok(samples) => samples,
                Err(err) => {
                    tracing::error!("Converting audio frame failed: {err}");
                    continue;
                }
            };

            let (Some(channels), Some(frame_rate)) = (
                NonZero::new(self.frame.channels()),
                NonZero::new(self.frame.rate()),
            ) else {
                continue;
            };

            // NEW: trim everything before the seek target.
            if let (Some(skip_until), Some(pts)) = (self.skip_until, self.frame.pts()) {
                let tb = self.time_base.0 as f64 / self.time_base.1 as f64;
                let frame_start = (pts as f64 * tb).max(0.0);
                let skip = skip_until.as_secs_f64() - frame_start;
                if skip > 0.0 {
                    let drop_frames = (skip * frame_rate.get() as f64).round() as usize;
                    let drop_samples = (drop_frames * channels.get() as usize).min(samples.len());
                    samples.drain(..drop_samples);
                } else {
                    self.skip_until = None;
                }
            }

            if samples.is_empty() {
                continue;
            }

            return Some(SamplesBuffer::new(channels, frame_rate, samples));
        }

        None
    }

    fn next_packet(&mut self) -> Option<ffmpeg::Packet> {
        let stream_index = self.stream_index;
        for (stream, packet) in self.ictx.packets() {
            if stream.index() == stream_index {
                return Some(packet);
            }
        }
        None
    }
}

const DECODE_QUEUE_BUFFERS: usize = 32;
const DECODE_CHUNK_FRAMES: usize = 1024;
const UNDERRUN_SILENCE: Duration = Duration::from_millis(10);

struct Chunk {
    pass: u64,
    samples: Vec<f32>,
}

fn spawn_decode_thread(
    mut frames: AudioFrames,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
) -> mpsc::Receiver<Chunk> {
    let (tx, rx) = mpsc::sync_channel(DECODE_QUEUE_BUFFERS);
    let chunk_samples = DECODE_CHUNK_FRAMES * channels.get() as usize;

    thread::Builder::new()
        .name("audio-decode".into())
        .spawn(move || {
            for pass in 0.. {
                let mut looped = false;

                let decoded = rodio::source::from_fn(|| match frames.next()? {
                    Decoded::Samples(buffer) => Some(buffer),
                    Decoded::Looped => {
                        looped = true;
                        None
                    }
                });
                let mut resampled = UniformSourceIterator::new(decoded, channels, rate);

                loop {
                    let samples: Vec<f32> = resampled.by_ref().take(chunk_samples).collect();
                    if samples.is_empty() {
                        break;
                    }
                    let finished = samples.len() < chunk_samples;
                    if tx.send(Chunk { pass, samples }).is_err() {
                        return;
                    }
                    if finished {
                        break;
                    }
                }

                drop(resampled);
                if !looped {
                    return;
                }
            }
        })
        .expect("spawn audio decode thread");

    rx
}

/// Engine's `TrackClock`: frames of the track actually played, padding excluded.
#[derive(Clone)]
pub struct TrackClock {
    frames: Arc<AtomicU64>,
    rate: NonZero<u32>,
}

impl TrackClock {
    fn new(rate: NonZero<u32>, start: Duration) -> Self {
        let start_frames = (start.as_secs_f64() * rate.get() as f64).round() as u64;
        Self {
            frames: Arc::new(AtomicU64::new(start_frames)),
            rate,
        }
    }

    pub fn position(&self) -> Duration {
        let frames = self.frames.load(Ordering::Relaxed) as u128;
        Duration::from_nanos((frames * 1_000_000_000 / self.rate.get() as u128) as u64)
    }
}

struct TrackSource {
    rx: mpsc::Receiver<Chunk>,
    /// ADDED: read at each seam between passes, as it plays.
    loop_audio: Arc<AtomicBool>,
    /// ADDED: the pass playing, and whether the track has ended at a seam.
    pass: u64,
    ended: bool,
    first: Option<Chunk>,
    samples: Vec<f32>,
    position: usize,
    silence: usize,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
    played: u64,
    clock: TrackClock,
}

impl Iterator for TrackSource {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        if self.silence > 0 {
            self.silence -= 1;
            return Some(0.0);
        }

        if self.ended {
            return None;
        }

        if self.position >= self.samples.len() {
            let chunk = match self.first.take() {
                Some(chunk) => chunk,
                None => match self.rx.try_recv() {
                    Ok(chunk) => chunk,
                    Err(mpsc::TryRecvError::Empty) => {
                        let frames = (self.rate.get() as u128 * UNDERRUN_SILENCE.as_micros()
                            / 1_000_000) as usize;
                        self.silence = frames * self.channels.get() as usize - 1;
                        return Some(0.0);
                    }
                    Err(mpsc::TryRecvError::Disconnected) => return None,
                },
            };
            if chunk.pass != self.pass {
                if !self.loop_audio.load(Ordering::Relaxed) {
                    self.ended = true;
                    return None;
                }
                self.pass = chunk.pass;
            }
            self.samples = chunk.samples;
            self.position = 0;
        }

        if self.position.is_multiple_of(self.channels.get() as usize) {
            self.played += 1;
            self.clock.frames.store(self.played, Ordering::Relaxed);
        }

        let sample = self.samples.get(self.position).copied();
        self.position += 1;
        sample
    }
}

impl Source for TrackSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        self.channels
    }
    fn sample_rate(&self) -> NonZero<u32> {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// The engine's `AudioPlayer`, minus fades and cues, attached to a shared [`AudioOut`].
pub struct AudioPlayer {
    sink: Player,
    clock: TrackClock,
}

impl AudioPlayer {
    /// `None` when the source has no audio stream.
    pub fn new(
        out: &AudioOut,
        source: MediaSource,
        loop_audio: Arc<AtomicBool>,
        volume: f32,
        start: Duration,
    ) -> Result<Option<Self>> {
        let Some(frames) = AudioFrames::open(source, start)? else {
            return Ok(None);
        };

        let config = out.stream.config();
        let (channels, rate) = (config.channel_count(), config.sample_rate());

        let rx = spawn_decode_thread(frames, channels, rate);
        let Ok(first) = rx.recv() else {
            return Ok(None);
        };

        let sink = Player::connect_new(out.stream.mixer());
        sink.pause();
        sink.set_volume(volume);

        let clock = TrackClock::new(rate, start);
        let played = clock.frames.load(Ordering::Relaxed);

        sink.append(TrackSource {
            rx,
            loop_audio,
            pass: first.pass,
            ended: false,
            first: Some(first),
            samples: Vec::new(),
            position: 0,
            silence: 0,
            channels,
            rate,
            played,
            clock: clock.clone(),
        });

        Ok(Some(Self { sink, clock }))
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn play(&self) {
        self.sink.play();
    }

    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume);
    }

    pub fn position(&self) -> Duration {
        self.clock.position()
    }

    pub fn output_position(&self) -> Duration {
        self.sink.get_pos()
    }

    pub fn empty(&self) -> bool {
        self.sink.empty()
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.sink.stop();
    }
}

fn convert_audio_frame(frame: &frame::Audio) -> Result<Vec<f32>> {
    let channels = frame.channels() as usize;
    let samples = frame.samples();
    let mut interleaved = vec![0f32; samples * channels];

    match frame.format() {
        Sample::U8(t) => {
            convert_samples::<u8>(frame, t, &mut interleaved, samples, channels, |s| {
                (s as f32 - 128.0) / 128.0
            })
        }
        Sample::I16(t) => {
            convert_samples::<i16>(frame, t, &mut interleaved, samples, channels, |s| {
                s as f32 / 32_768.0
            })
        }
        Sample::I32(t) => {
            convert_samples::<i32>(frame, t, &mut interleaved, samples, channels, |s| {
                s as f32 / 2_147_483_648.0
            })
        }
        Sample::I64(t) => {
            convert_samples::<i64>(frame, t, &mut interleaved, samples, channels, |s| {
                (s as f64 / 9_223_372_036_854_775_808.0) as f32
            })
        }
        Sample::F32(t) => {
            convert_samples::<f32>(frame, t, &mut interleaved, samples, channels, |s| s)
        }
        Sample::F64(t) => {
            convert_samples::<f64>(frame, t, &mut interleaved, samples, channels, |s| s as f32)
        }
        Sample::None => bail!("No sample type"),
    }

    Ok(interleaved)
}

fn convert_samples<T: Copy + AnyBitPattern>(
    frame: &frame::Audio,
    sample_type: sample::Type,
    interleaved: &mut [f32],
    samples: usize,
    channels: usize,
    convert_fn: impl Fn(T) -> f32,
) {
    match sample_type {
        sample::Type::Packed => {
            let all: &[T] = bytemuck::cast_slice(frame.data(0));
            for (i, &s) in all.iter().take(samples * channels).enumerate() {
                interleaved[i] = convert_fn(s);
            }
        }
        sample::Type::Planar => {
            for ch in 0..channels {
                let data: &[T] = bytemuck::cast_slice(frame.data(ch));
                for (i, &s) in data.iter().take(samples).enumerate() {
                    interleaved[i * channels + ch] = convert_fn(s);
                }
            }
        }
    }
}
