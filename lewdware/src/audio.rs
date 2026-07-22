use anyhow::{Context, Result, bail};
use bytemuck::AnyBitPattern;
use ffmpeg_next::{
    self as ffmpeg,
    format::{Sample, sample},
    frame,
};
use std::{
    num::NonZero,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self},
    time::Duration,
};

use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, Source, buffer::SamplesBuffer};

use crate::{
    app::{EventPoster, UserEvent},
    lua::ItemId,
    media::{MediaSource, bounded_input::BoundedInput},
};

pub struct AudioPlayer {
    _stream: MixerDeviceSink,
    sink: Arc<Player>,
}

impl AudioPlayer {
    pub fn new<T: EventPoster>(
        source: MediaSource,
        loop_audio: Arc<AtomicBool>,
        volume: f32,
        event_poster: Option<(ItemId, T)>,
    ) -> Result<Option<Self>> {
        let (stream, sink) = match setup_decoder(source, loop_audio)? {
            Some(x) => x,
            None => return Ok(None),
        };
        let sink = Arc::new(sink);
        sink.set_volume(volume);

        if let Some((id, event_poster)) = event_poster {
            let sink_clone = sink.clone();
            thread::spawn(move || {
                sink_clone.sleep_until_end();
                event_poster.post_event(UserEvent::AudioFinish { id });
            });
        }

        Ok(Some(Self {
            _stream: stream,
            sink,
        }))
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

    pub fn stop(&self) {
        self.sink.stop();
    }

    pub fn position(&self) -> Duration {
        let pos = self.sink.get_pos();
        pos
    }
}

/// Decoded audio, pulled from a media source a buffer at a time and looped back to the start for
/// as long as `loop_audio` says to.
///
/// Kept apart from [`setup_decoder`] deliberately: opening an output device is what makes the rest
/// of this module impossible to exercise without one, while the decode and loop handling here is
/// the part with something to get wrong.
struct AudioFrames {
    ictx: BoundedInput,
    decoder: ffmpeg::decoder::Audio,
    stream_index: usize,
    loop_audio: Arc<AtomicBool>,
    /// Reused receive buffer, so a frame per call isn't allocated.
    frame: frame::Audio,
    /// Consecutive passes over the input that yielded no samples at all. See `MAX_EMPTY_PASSES`.
    empty_passes: u32,
}

/// How many times the input may be played through, yielding nothing whatsoever, before the track
/// is abandoned rather than looped again.
///
/// A pass that produces no samples produces none however many times it is repeated, so looping it
/// spins this thread at full tilt for as long as the sound is supposed to be playing. Normal
/// playback resets the count on every buffer handed over, so this is only ever reached by a
/// decoder that has become unable to decode -- one left in a drained state by a missing `flush`,
/// say, refusing every packet it is given.
const MAX_EMPTY_PASSES: u32 = 3;

impl AudioFrames {
    /// `None` when the source carries no audio at all.
    fn open(source: MediaSource, loop_audio: Arc<AtomicBool>) -> Result<Option<Self>> {
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
    fn next_buffer(&mut self) -> Option<SamplesBuffer> {
        loop {
            // Before feeding the decoder anything more: this returns as soon as it has a buffer to
            // hand over, so output from the previous call can still be waiting. Taking it first
            // also means `send_packet` below is never handed a packet the decoder has no room for.
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
                    "audio decoded nothing over {MAX_EMPTY_PASSES} passes; giving up rather than                      looping it again"
                );
                return None;
            }

            tracing::info!("Looping");

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

pub fn setup_decoder(
    source: MediaSource,
    loop_audio: Arc<AtomicBool>,
) -> Result<Option<(MixerDeviceSink, Player)>> {
    let mut frames = match AudioFrames::open(source, loop_audio)? {
        Some(frames) => frames,
        None => return Ok(None),
    };

    let mut stream = DeviceSinkBuilder::open_default_sink()?;

    stream.log_on_drop(false);

    let sink = Player::connect_new(stream.mixer());

    sink.pause();

    let source = rodio::source::from_factory(move || frames.next_buffer()).buffered();

    sink.append(source);

    Ok(Some((stream, sink)))
}

fn convert_audio_frame(frame: &frame::Audio) -> Result<Vec<f32>> {
    let channels = frame.channels() as usize;
    let samples = frame.samples();
    let mut interleaved = vec![0f32; samples * channels];

    // ffmpeg can output frames in a bunch of different formats. We want to convert each format to
    // a floating point number between -1 and 1.
    //
    // For unsigned 8 bit integers, for example, the values range from 0 to 255 (2^8 - 1), so we
    // subtract 128 and divide by 128.
    //
    // For signed `n` bit integers, the values range from -2 ^ (n - 1) to 2 ^ (n - 1) - 1, so we
    // divide by 2 ^ (n - 1) to normalize.
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
                // This number is large, so do f64 division to avoid loss of precision
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
    // From the ffmpeg docs:
    // For planar sample formats, each audio channel is in a separate data plane, and linesize is
    // the buffer size, in bytes, for a single plane. All data planes must be the same size. For
    // packed sample formats, only the first data plane is used, and samples for each channel are
    // interleaved. In this case, linesize is the buffer size, in bytes, for the 1 plane.
    match sample_type {
        sample::Type::Packed => {
            let data = frame.data(0);
            // ffmpeg has told us the format and number of samples, but `data` is a raw byte slice,
            // so we need a small bit of unsafe code to convert to our required format.
            //
            // There are `samples` samples in each channel, and in this case all the data is
            // contiguous (packed), so there is a total of `samples * channels` values.
            let all_samples: &[T] = bytemuck::cast_slice(data);

            for (i, &sample) in all_samples.iter().take(samples * channels).enumerate() {
                interleaved[i] = convert_fn(sample);
            }
        }
        sample::Type::Planar => {
            for ch in 0..channels {
                let data = frame.data(ch);
                // Again, we know the format and number of samples. In this case the data for each
                // channel is not stored contiguously, so we handle each channel (a buffer of
                // `samples` values) separately.
                let channel_samples: &[T] = bytemuck::cast_slice(data);

                for (i, &sample) in channel_samples.iter().take(samples).enumerate() {
                    interleaved[i * channels + ch] = convert_fn(sample);
                }
            }
        }
    }
}

/// Decode coverage for [`AudioFrames`], against real files built at test time with the sidecar
/// ffmpeg. Nothing here opens an output device — that is exactly why `AudioFrames` is separate
/// from `setup_decoder`.
///
/// `#[ignore]`d so the default `cargo test` needs no binaries, matching `video::sidecar`. To run:
/// `./deploy/linux/download_ffmpeg_sidecars.sh && cargo test -p lewdware --bin lewdware-engine audio::sidecar -- --ignored`
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

    /// A two-second tone, encoded with the same codec and settings `shared::encode` uses.
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

    /// Samples per channel, which is what has to survive a round of decoding.
    fn count_samples(buffer: &SamplesBuffer) -> usize {
        let channels = buffer.channels().get() as usize;
        buffer.clone().count() / channels
    }

    /// The whole track has to come out. Draining the decoder at the end rather than flushing it is
    /// what makes that true for a codec that holds anything back.
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

        // Opus works in 48kHz frames and carries encoder delay, so this will not land on exactly
        // two seconds; it does have to land within a frame or so of it rather than short.
        let seconds = samples as f64 / rate as f64;
        assert!(
            (seconds - 2.0).abs() < 0.05,
            "decoded {seconds:.3}s of a 2s track"
        );
    }

    /// The risky half of the change: the decoder is put into a drained state at the end of every
    /// pass, and looping depends on `flush` taking it back out again. Without that it would refuse
    /// every packet of the second pass and the track would fall silent, so a loop has to yield as
    /// much audio the second time round as the first.
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
            // A looping source never returns `None`, so take exactly one pass' worth. The bound is
            // what turns a decoder left unable to accept packets into a failure rather than a hang
            // -- `next_buffer` would otherwise spin forever producing nothing.
            let mut buffers = 0;
            while samples < once {
                let buffer = frames.next_buffer().expect("a looping source never ends");
                samples += count_samples(&buffer);

                buffers += 1;
                assert!(
                    buffers < 100_000,
                    "pass {pass} produced {buffers} buffers and only {samples} of the {once}                      samples in a pass; the decoder is not accepting packets"
                );
            }

            assert!(
                samples < once + once / 10,
                "pass {pass} ran {samples} samples past the {once} of a single pass, so a pass \
                 boundary was missed"
            );
        }
    }
}
