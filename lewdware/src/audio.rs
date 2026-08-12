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
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self},
    time::Duration,
};

use rodio::{
    DeviceSinkBuilder, MixerDeviceSink, Player,
    buffer::SamplesBuffer,
    cpal,
    cpal::traits::{DeviceTrait, HostTrait},
    source::UniformSourceIterator,
};

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
        self.sink.get_pos()
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

            // Debug, not info: a short track looping under a mode that plays many of them at once
            // is several lines a second in the log file and the dev-log stream, which drowns
            // everything worth reading. Looping is the normal case, not an event.
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

/// How many device opens go by between summaries. Every open is logged at `debug`, but the file
/// and dev-log layers are filtered at `info` (`shared::logging`), so a per-open line at that level
/// is invisible where it would actually be read -- and one *per sound* at info would be noise in a
/// normal session. A periodic summary is the compromise: nothing in the log for a pack that plays
/// the occasional sting, real numbers the moment something plays a lot of them.
const DEVICE_OPEN_SUMMARY_EVERY: u64 = 16;

static DEVICE_OPENS: AtomicU64 = AtomicU64::new(0);
static DEVICE_OPEN_TOTAL_US: AtomicU64 = AtomicU64::new(0);
static DEVICE_OPEN_MAX_US: AtomicU64 = AtomicU64::new(0);

/// Note how long one output device open took, and summarise every
/// [`DEVICE_OPEN_SUMMARY_EVERY`] of them.
///
/// The engine opens a device stream per playing item, so this is a per-sound cost that lands on
/// the media manager thread ahead of everything queued behind it. If the mean here is tens of
/// milliseconds, that alone explains audio being slow to start when a mode plays many sounds at
/// once -- and it is the number to check before blaming mixing or decoding.
fn record_device_open(elapsed: Duration) {
    let micros = elapsed.as_micros() as u64;

    tracing::debug!(
        elapsed_ms = micros as f64 / 1000.0,
        "opened an output device sink"
    );

    let opens = DEVICE_OPENS.fetch_add(1, Ordering::Relaxed) + 1;
    let total = DEVICE_OPEN_TOTAL_US.fetch_add(micros, Ordering::Relaxed) + micros;
    DEVICE_OPEN_MAX_US.fetch_max(micros, Ordering::Relaxed);

    if opens.is_multiple_of(DEVICE_OPEN_SUMMARY_EVERY) {
        tracing::info!(
            opens,
            mean_ms = (total as f64 / opens as f64) / 1000.0,
            worst_ms = DEVICE_OPEN_MAX_US.load(Ordering::Relaxed) as f64 / 1000.0,
            "output device sinks opened so far (one per playing item)"
        );
    }
}

/// How many buffers may sit between the decoder thread and the audio callback.
///
/// One buffer is [`DECODE_CHUNK_FRAMES`] frames -- about 21ms at 48kHz -- so this is roughly two
/// thirds of a second of slack. Enough that an ordinary scheduling hiccup or a loop's seek-and-flush
/// never reaches the audio side, small enough that a stopped track throws away almost nothing, and
/// bounded so that a hundred playing sounds cannot each grow a queue.
const DECODE_QUEUE_BUFFERS: usize = 32;

/// Frames per buffer handed to the audio side.
///
/// Fixed here rather than following ffmpeg's frame size because the worker now resamples, and its
/// output frame count no longer matches the decoder's input.
const DECODE_CHUNK_FRAMES: usize = 1024;

/// How much silence to hand the audio side when the queue is empty anyway.
///
/// An underrun has to produce *something*: returning `None` would end the track, and blocking
/// would stall the callback -- and once a device sink is shared between every playing sound
/// (`design`/step B), stalling one source stalls all of them. A short slice of silence is the only
/// answer that keeps the deadline, and it is audible as a tick rather than a dropout.
const UNDERRUN_SILENCE: Duration = Duration::from_millis(10);

/// The audio side of one track: pops buffers that are already decoded *and already in the device's
/// format*, and does no work of its own.
///
/// Both halves of that matter, and both were measured rather than assumed
/// (`dev-modes/audio-stress`). Decoding used to happen inside the source pull -- on whichever
/// thread drives the sink -- so every sound paid for its own ffmpeg decode on a realtime audio
/// thread. Moving that off alone changed nothing, because a profile of the audio thread showed the
/// real cost was *resampling*: `SampleRateConverter` and friends were around 20% of it, against
/// roughly 8% for the actual OS-facing output.
///
/// So the worker resamples too, to whatever the device asked for. `SampleRateConverter::next`
/// short-circuits to a plain passthrough when its input and output rates are equal
/// (`conversions/sample_rate.rs`), so the converter still sitting in the player's chain costs one
/// forwarded call per sample instead of an interpolation.
///
/// Doing it on the worker also keeps it *correct*: a resampler carries state across its input, so
/// converting each decoded buffer independently would put a discontinuity at every buffer
/// boundary. One converter spans the whole track here.
struct DecodedBuffers {
    rx: mpsc::Receiver<SamplesBuffer>,
    /// One buffer fetched during setup, so playback does not open with an underrun.
    first: Option<SamplesBuffer>,
    /// The device's format, which is also every buffer's format -- including the silence below.
    channels: NonZero<u16>,
    rate: NonZero<u32>,
}

impl DecodedBuffers {
    /// Start decoding and resampling `frames` on its own thread, targeting the device's
    /// `channels`/`rate`, and wait for the first buffer.
    ///
    /// Blocking for that one is deliberate: it runs on the media manager thread during setup,
    /// before the sink is playing, and it costs a single chunk's decode against a device open on
    /// that same thread measured at 31-119ms. It also means a file that cannot be decoded at all
    /// fails here, as "no audio stream available", rather than becoming a sound that plays nothing.
    ///
    /// `None` means the track produced no audio at all.
    fn start(frames: AudioFrames, channels: NonZero<u16>, rate: NonZero<u32>) -> Option<Self> {
        let (tx, rx) = mpsc::sync_channel(DECODE_QUEUE_BUFFERS);

        thread::spawn(move || {
            // One converter over the whole track, so its state is continuous. `from_fn` is
            // the same adapter the audio side used to hold; here it is just the front of the
            // worker's chain.
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

                // Ends when the track does, or when the receiver goes away -- dropping the sink
                // drops the source, which drops `rx`, which fails the next send even if this
                // thread is parked inside one.
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

    /// The next buffer to play: a decoded one if the worker has kept up, silence if it has not,
    /// and `None` once the track is finished and the worker has gone.
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

/// The device id from `AppConfig::audio_device`, published once at startup by
/// [`set_output_device`].
///
/// A process-global rather than a parameter threaded down to every sink open: the output device is
/// one process-wide choice read once from the config, while the two `AudioPlayer::new` call sites
/// sit at the bottom of the media stack (`video`, `media::manager`) with no view of the config.
static CONFIGURED_DEVICE: OnceLock<Option<String>> = OnceLock::new();

/// [`CONFIGURED_DEVICE`] looked up against the host, done once.
///
/// Cached because resolution means enumerating the host's devices, and this file's own numbers say
/// what that would cost: a sink is opened *per playing item*, on the media manager thread, already
/// measured at 31-119ms a time. Paying a full enumeration on top of every sound would land
/// squarely on the path a mode playing many sounds at once is already waiting behind.
static RESOLVED_DEVICE: OnceLock<Option<cpal::Device>> = OnceLock::new();

/// Records which output device sinks should open on. Call once, at startup, before any audio
/// plays; later calls are ignored.
pub fn set_output_device(device: Option<String>) {
    if CONFIGURED_DEVICE.set(device).is_err() {
        tracing::warn!("the output device was already set; ignoring the later choice");
    }
}

/// Looks up a device by its `cpal::DeviceId` string, or `None` for "system default".
///
/// `None` is also what an id that no longer matches anything returns -- an unplugged headset, or a
/// config written on another machine. That is a fallback to the default device for this session
/// and *not* a rewrite of the setting: see `AppConfig::audio_device`.
fn resolve_device(id: &str) -> Option<cpal::Device> {
    let parsed = match id.parse::<cpal::DeviceId>() {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::warn!(%id, "could not parse the saved audio device id ({err}); using the default device");
            return None;
        }
    };

    match cpal::default_host().device_by_id(&parsed) {
        Some(device) => Some(device),
        None => {
            tracing::warn!(
                %id,
                "the chosen audio device is not available; using the default device for this session"
            );
            None
        }
    }
}

/// The device to open sinks on, or `None` to let rodio pick the default.
fn output_device() -> Option<&'static cpal::Device> {
    RESOLVED_DEVICE
        .get_or_init(|| {
            let id = CONFIGURED_DEVICE.get()?.as_deref()?;
            let device = resolve_device(id);

            if let Some(device) = &device {
                tracing::info!(
                    %id,
                    name = device.description().ok().map(|d| d.name().to_owned()),
                    "playing audio on the chosen output device"
                );
            }

            device
        })
        .as_ref()
}

/// Which device a sink ended up on, relative to what was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SinkDevice {
    /// The chosen device, or the system default where that is what was chosen.
    AsRequested,
    /// A device was named but could not be used, so the system default was opened instead.
    FellBack,
}

/// Opens an output sink on the configured device, falling back to the default.
///
/// The fallback is a second line of defence behind [`resolve_device`]: a device can be listed and
/// still refuse to open (exclusive use by another app, a bluetooth sink that dropped between
/// resolution and now). Silence would be the alternative, and audio on the wrong speakers beats no
/// audio at all.
fn open_sink() -> Result<(MixerDeviceSink, SinkDevice), rodio::DeviceSinkError> {
    // Distinguishes "no device chosen" from "chosen device could not be resolved" -- both leave
    // `output_device()` empty, but only the second is a fallback worth reporting.
    let named = CONFIGURED_DEVICE.get().is_some_and(Option::is_some);

    let Some(device) = output_device() else {
        let outcome = if named {
            SinkDevice::FellBack
        } else {
            SinkDevice::AsRequested
        };
        return DeviceSinkBuilder::open_default_sink().map(|stream| (stream, outcome));
    };

    let opened = DeviceSinkBuilder::from_device(device.clone())
        .and_then(|builder| builder.open_sink_or_fallback());

    match opened {
        Ok(stream) => Ok((stream, SinkDevice::AsRequested)),
        Err(err) => {
            tracing::warn!("could not open the chosen audio device ({err}); using the default");
            DeviceSinkBuilder::open_default_sink().map(|stream| (stream, SinkDevice::FellBack))
        }
    }
}

/// Prints this process's view of the output devices as JSON, then exits. Driven by
/// `shared::audio::LIST_AUDIO_DEVICES_FLAG`.
///
/// Lives in the engine for the reason that flag documents: the id the config app stores carries
/// the *host* cpal chose, and only this binary's choice of host is the one that will later have to
/// resolve it.
pub fn list_audio_devices() -> Result<()> {
    let host = cpal::default_host();

    // Which device the host would pick on its own -- shown against "System default" so the picker
    // can say what that currently means rather than leaving it abstract.
    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok());

    let devices = host
        .output_devices()
        .context("could not list the audio output devices")?
        .filter_map(|device| {
            // Both are needed and both can fail per-device (a device can disappear mid-enumeration),
            // so a device missing either is skipped rather than sinking the whole list: without an
            // id it cannot be stored, and without a description it cannot be labelled.
            let id = device.id().ok()?;
            let description = device.description().ok()?;

            if !description.supports_output() {
                return None;
            }

            Some(shared::audio::AudioDeviceInfo {
                is_default: default_id.as_ref() == Some(&id),
                id: id.to_string(),
                name: description.name().to_owned(),
            })
        })
        .collect::<Vec<_>>();

    println!("{}", serde_json::to_string(&devices)?);

    Ok(())
}

/// How loud the test tone is. Well under full scale: this plays at whatever the OS volume happens
/// to be, and a user checking which speakers are live should not be startled by it.
const TEST_TONE_AMPLITUDE: f32 = 0.22;

/// Plays a short chime on `device` and returns once it has finished. Driven by
/// `shared::audio::TEST_AUDIO_FLAG`.
///
/// Goes through [`open_sink`], the same path a real sound takes, so a device that fails here is a
/// device that would have failed in a session -- which is the entire point of a test button. That
/// includes the fallback: if the chosen device cannot be opened the tone plays on the default one,
/// matching what a session would do rather than reporting a failure the engine wouldn't have.
pub fn play_test_tone(device: Option<String>) -> Result<()> {
    set_output_device(device);

    let (mut stream, outcome) = open_sink().context("could not open an audio output device")?;

    // Same reason as `setup_decoder`: rodio otherwise prints a "Dropping DeviceSink" notice to
    // stderr on the way out, and stderr is what the config app quotes back to the user when a
    // probe fails.
    stream.log_on_drop(false);

    let config = stream.config();
    let (channels, rate) = (config.channel_count(), config.sample_rate());

    let sink = Player::connect_new(stream.mixer());
    sink.append(test_tone(channels, rate));
    sink.sleep_until_end();

    let result = shared::audio::TestAudioResult {
        fell_back: outcome == SinkDevice::FellBack,
    };
    println!("{}", serde_json::to_string(&result)?);

    Ok(())
}

/// A two-note chime, generated rather than shipped as an asset so the test needs nothing from a
/// pack -- it has to work before a pack is even chosen.
///
/// Both notes decay exponentially and the second overlaps the first, which is what stops it
/// sounding like two beeps. The envelope also matters mechanically: a tone that stopped at full
/// amplitude would end on a step discontinuity and click.
fn test_tone(channels: NonZero<u16>, rate: NonZero<u32>) -> SamplesBuffer {
    /// Hz, and when each note starts in seconds. A fifth apart -- D5 and A5.
    const NOTES: [(f32, f32); 2] = [(587.33, 0.0), (880.0, 0.13)];
    const LENGTH: Duration = Duration::from_millis(650);
    /// Larger decays faster. Tuned so each note is inaudible well before the buffer ends.
    const DECAY: f32 = 7.0;

    let rate_f = rate.get() as f32;
    let frames = (rate.get() as u64 * LENGTH.as_millis() as u64 / 1000) as usize;

    let mut samples = Vec::with_capacity(frames * channels.get() as usize);
    for frame in 0..frames {
        let t = frame as f32 / rate_f;

        let value: f32 = NOTES
            .iter()
            .filter(|(_, start)| t >= *start)
            .map(|(frequency, start)| {
                let age = t - start;
                (age * frequency * std::f32::consts::TAU).sin() * (-age * DECAY).exp()
            })
            .sum();

        // Mono content written to every channel, so it comes out of both speakers rather than
        // only the left -- a test tone that only played on one side would read as a fault.
        let sample = value * TEST_TONE_AMPLITUDE;
        samples.extend(std::iter::repeat_n(sample, channels.get() as usize));
    }

    SamplesBuffer::new(channels, rate, samples)
}

pub fn setup_decoder(
    source: MediaSource,
    loop_audio: Arc<AtomicBool>,
) -> Result<Option<(MixerDeviceSink, Player)>> {
    // Opened before the device, so that a video with no audio track costs nothing: this returns
    // `None` and no output stream is ever opened for it.
    let frames = match AudioFrames::open(source, loop_audio)? {
        Some(frames) => frames,
        None => return Ok(None),
    };

    // One output device stream per playing item, opened here on the media manager thread -- which
    // resolves requirements one at a time, so this sits in front of every other piece of media
    // queued behind it. Timed because the cost is the whole question when a mode plays many sounds
    // at once: see `dev-modes/audio-stress`.
    let open_started = std::time::Instant::now();
    let (mut stream, _) = open_sink()?;
    record_device_open(open_started.elapsed());

    stream.log_on_drop(false);

    // The device's format is the worker's target: matching it here is what lets the resampler in
    // the player's chain fall through to a passthrough instead of interpolating every sample.
    let config = stream.config();
    let Some(mut decoded) =
        DecodedBuffers::start(frames, config.channel_count(), config.sample_rate())
    else {
        return Ok(None);
    };

    let sink = Player::connect_new(stream.mixer());

    sink.pause();

    // No `.buffered()`. It exists to let a source be replayed, and kept every sample ever decoded
    // in memory to do it -- unbounded growth for a looping track, and pointless here besides:
    // looping is handled by `AudioFrames` seeking back to the start, so nothing is ever replayed
    // from the cache.
    let source = rodio::source::from_fn(move || decoded.next());

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
