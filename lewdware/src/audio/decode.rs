use std::{
    num::NonZero,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
    MixerDeviceSink, Player,
    buffer::SamplesBuffer,
    source::UniformSourceIterator,
};

use crate::media::{bounded_input::BoundedInput, MediaSource};

use super::device::{open_sink, record_device_open};

/// Decoded audio, pulled from a media source a buffer at a time and looped back to the start for
/// as long as `loop_audio` says to.
pub struct AudioFrames {
    ictx: BoundedInput,
    decoder: ffmpeg::decoder::Audio,
    stream_index: usize,
    loop_audio: Arc<AtomicBool>,
    /// Reused receive buffer, so a frame per call isn't allocated.
    frame: frame::Audio,
    /// Consecutive passes over the input that yielded no samples at all. See `MAX_EMPTY_PASSES`.
    empty_passes: u32,
}

const MAX_EMPTY_PASSES: u32 = 3;

impl AudioFrames {
    /// `None` when the source carries no audio at all.
    pub fn open(source: MediaSource, loop_audio: Arc<AtomicBool>) -> Result<Option<Self>> {
        ffmpeg::init()?;

        let ictx = source.open()?;
        let stream_index = match ictx.streams().best(ffmpeg::media::Type::Audio) {
            Some(stream) => stream.index(),
            None => return Ok(None),
        };

        let media = ictx.stream(stream_index).context("Invalid stream index")?;
        let context = ffmpeg::codec::Context::from_parameters(media.parameters())?;
        let mut decoder = context.decoder().audio()?;
        decoder.set_packet_time_base(media.time_base());

        Ok(Some(Self {
            ictx,
            decoder,
            stream_index,
            loop_audio,
            frame: frame::Audio::empty(),
            empty_passes: 0,
        }))
    }

    /// The next buffer of samples, or `None` once the track has finished and is not to be looped.
    pub fn next_buffer(&mut self) -> Option<SamplesBuffer> {
        loop {
            if let Some(buffer) = self.take_ready() {
                self.empty_passes = 0;
                return Some(buffer);
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
                return Some(buffer);
            }

            if !self.loop_audio.load(Ordering::Relaxed) {
                return None;
            }

            self.empty_passes += 1;
            if self.empty_passes >= MAX_EMPTY_PASSES {
                tracing::error!(
                    "audio decoded nothing over {MAX_EMPTY_PASSES} passes; giving up rather than looping it again"
                );
                return None;
            }

            tracing::debug!("Looping");

            if let Err(err) = self.ictx.seek(0, ..0) {
                tracing::error!("Failed to seek to start: {err}");
            }

            self.decoder.flush();
        }
    }

    /// Whatever the decoder has ready, converted to samples; `None` once it has nothing left.
    fn take_ready(&mut self) -> Option<SamplesBuffer> {
        while self.decoder.receive_frame(&mut self.frame).is_ok() {
            let samples = match convert_audio_frame(&self.frame) {
                Ok(samples) => samples,
                Err(err) => {
                    tracing::error!("Converting audio frame failed: {err}");
                    continue;
                }
            };

            if samples.is_empty() {
                continue;
            }

            let (Some(channels), Some(frame_rate)) = (
                NonZero::new(self.frame.channels()),
                NonZero::new(self.frame.rate()),
            ) else {
                tracing::warn!("Channels or frame rate is 0");
                continue;
            };

            return Some(SamplesBuffer::new(channels, frame_rate, samples));
        }

        None
    }

    /// The next packet belonging to the stream being decoded, skipping every other stream.
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

struct DecodedBuffers {
    rx: mpsc::Receiver<SamplesBuffer>,
    first: Option<SamplesBuffer>,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
}

impl DecodedBuffers {
    fn start(frames: AudioFrames, channels: NonZero<u16>, rate: NonZero<u32>) -> Option<Self> {
        let (tx, rx) = mpsc::sync_channel(DECODE_QUEUE_BUFFERS);

        thread::spawn(move || {
            let decoded = rodio::source::from_fn({
                let mut frames = frames;
                move || frames.next_buffer()
            });
            let mut resampled = UniformSourceIterator::new(decoded, channels, rate);

            let chunk_samples = DECODE_CHUNK_FRAMES * channels.get() as usize;

            loop {
                let mut chunk = Vec::with_capacity(chunk_samples);
                for _ in 0..chunk_samples {
                    match resampled.next() {
                        Some(sample) => chunk.push(sample),
                        None => break,
                    }
                }

                if chunk.is_empty() {
                    break;
                }

                let finished = chunk.len() < chunk_samples;

                if tx.send(SamplesBuffer::new(channels, rate, chunk)).is_err() || finished {
                    break;
                }
            }
        });

        let first = rx.recv().ok()?;

        Some(Self {
            channels,
            rate,
            first: Some(first),
            rx,
        })
    }

    fn next(&mut self) -> Option<SamplesBuffer> {
        if let Some(first) = self.first.take() {
            return Some(first);
        }

        match self.rx.try_recv() {
            Ok(buffer) => Some(buffer),
            Err(mpsc::TryRecvError::Empty) => Some(self.silence()),
            Err(mpsc::TryRecvError::Disconnected) => None,
        }
    }

    fn silence(&self) -> SamplesBuffer {
        let frames = (self.rate.get() as u128 * UNDERRUN_SILENCE.as_micros() / 1_000_000) as usize;

        SamplesBuffer::new(
            self.channels,
            self.rate,
            vec![0.0; frames * self.channels.get() as usize],
        )
    }
}

pub fn setup_decoder(
    source: MediaSource,
    loop_audio: Arc<AtomicBool>,
) -> Result<Option<(MixerDeviceSink, Player)>> {
    let frames = match AudioFrames::open(source, loop_audio)? {
        Some(frames) => frames,
        None => return Ok(None),
    };

    let open_started = std::time::Instant::now();
    let (mut stream, _) = open_sink()?;
    record_device_open(open_started.elapsed());

    stream.log_on_drop(false);

    let config = stream.config();
    let Some(mut decoded) =
        DecodedBuffers::start(frames, config.channel_count(), config.sample_rate())
    else {
        return Ok(None);
    };

    let sink = Player::connect_new(stream.mixer());
    sink.pause();

    let source = rodio::source::from_fn(move || decoded.next());
    sink.append(source);

    Ok(Some((stream, sink)))
}

fn convert_audio_frame(frame: &frame::Audio) -> Result<Vec<f32>> {
    let channels = frame.channels() as usize;
    let samples = frame.samples();
    let mut interleaved = vec![0f32; samples * channels];

    match frame.format() {
        Sample::U8(sample_type) => {
            convert_samples::<u8>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| (sample as f32 - 128.0) / 128.0,
            );
        }
        Sample::I16(sample_type) => {
            convert_samples::<i16>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| sample as f32 / 32_768.0,
            );
        }
        Sample::I32(sample_type) => {
            convert_samples::<i32>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| sample as f32 / 2_147_483_648.0,
            );
        }
        Sample::I64(sample_type) => {
            convert_samples::<i64>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| (sample as f64 / 9_223_372_036_854_775_808.0) as f32,
            );
        }
        Sample::F32(sample_type) => {
            convert_samples::<f32>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| sample,
            );
        }
        Sample::F64(sample_type) => {
            convert_samples::<f64>(
                frame,
                sample_type,
                &mut interleaved,
                samples,
                channels,
                |sample| sample as f32,
            );
        }
        Sample::None => {
            bail!("No sample type");
        }
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
            let data = frame.data(0);
            let all_samples: &[T] = bytemuck::cast_slice(data);

            for (i, &sample) in all_samples.iter().take(samples * channels).enumerate() {
                interleaved[i] = convert_fn(sample);
            }
        }
        sample::Type::Planar => {
            for ch in 0..channels {
                let data = frame.data(ch);
                let channel_samples: &[T] = bytemuck::cast_slice(data);

                for (i, &sample) in channel_samples.iter().take(samples).enumerate() {
                    interleaved[i * channels + ch] = convert_fn(sample);
                }
            }
        }
    }
}

#[cfg(test)]
mod sidecar {
    use super::*;
    use rodio::Source;
    use std::path::{Path, PathBuf};

    fn binaries_dir() -> PathBuf {
        std::env::var_os("LEWDWARE_SIDECAR_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../pack-editor/src-tauri/binaries")
            })
    }

    fn ffmpeg(dir: &Path, args: &[&str]) {
        let ffmpeg = binaries_dir().join("lewdware-ffmpeg");
        assert!(
            ffmpeg.is_file(),
            "no ffmpeg sidecar at {} -- run deploy/linux/download_ffmpeg_sidecars.sh from the \
             repo root, or set LEWDWARE_SIDECAR_DIR",
            binaries_dir().display()
        );

        let output = std::process::Command::new(ffmpeg)
            .current_dir(dir)
            .args(["-y", "-loglevel", "error"])
            .args(args)
            .output()
            .expect("run sidecar ffmpeg");

        assert!(
            output.status.success(),
            "ffmpeg {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn fixture(dir: &Path) -> PathBuf {
        ffmpeg(
            dir,
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:a",
                "libopus",
                "-b:a",
                "64k",
                "clip.opus",
            ],
        );

        dir.join("clip.opus")
    }

    fn frames(path: &Path, loop_audio: bool) -> AudioFrames {
        let source = MediaSource {
            path: path.to_path_buf(),
            offset: 0,
            length: std::fs::metadata(path).expect("fixture exists").len(),
        };

        AudioFrames::open(source, Arc::new(AtomicBool::new(loop_audio)))
            .expect("open fixture")
            .expect("fixture has an audio stream")
    }

    fn count_samples(buffer: &SamplesBuffer) -> usize {
        let channels = buffer.channels().get() as usize;
        buffer.clone().count() / channels
    }

    #[test]
    #[ignore]
    fn a_track_played_once_decodes_to_its_full_length() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = fixture(tmp.path());

        let mut frames = frames(&path, false);
        let mut samples = 0;
        let mut rate = 0;
        while let Some(buffer) = frames.next_buffer() {
            rate = buffer.sample_rate().get();
            samples += count_samples(&buffer);
        }

        assert!(rate > 0, "decoded nothing at all");

        let seconds = samples as f64 / rate as f64;
        assert!(
            (seconds - 2.0).abs() < 0.05,
            "decoded {seconds:.3}s of a 2s track"
        );
    }

    #[test]
    #[ignore]
    fn looping_decodes_every_pass_in_full() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = fixture(tmp.path());

        let once = {
            let mut frames = frames(&path, false);
            let mut samples = 0;
            while let Some(buffer) = frames.next_buffer() {
                samples += count_samples(&buffer);
            }
            samples
        };

        let mut frames = frames(&path, true);
        for pass in 1..=3 {
            let mut samples = 0;
            let mut buffers = 0;
            while samples < once {
                let buffer = frames.next_buffer().expect("a looping source never ends");
                samples += count_samples(&buffer);

                buffers += 1;
                assert!(
                    buffers < 100_000,
                    "pass {pass} produced {buffers} buffers and only {samples} of the {once} samples in a pass; the decoder is not accepting packets"
                );
            }

            assert!(
                samples < once + once / 10,
                "pass {pass} ran {samples} samples past the {once} of a single pass, so a pass boundary was missed"
            );
        }
    }
}
