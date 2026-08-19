use std::{
    num::NonZero,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use rodio::{
    DeviceSinkBuilder, MixerDeviceSink, Player,
    buffer::SamplesBuffer,
    cpal::{self, traits::{DeviceTrait, HostTrait}},
};

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
pub(crate) fn record_device_open(elapsed: Duration) {
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

/// The device id from `AppConfig::audio_device`, published once at startup by
/// [`set_output_device`].
static CONFIGURED_DEVICE: OnceLock<Option<String>> = OnceLock::new();

/// [`CONFIGURED_DEVICE`] looked up against the host, done once.
static RESOLVED_DEVICE: OnceLock<Option<cpal::Device>> = OnceLock::new();

/// Records which output device sinks should open on. Call once, at startup, before any audio
/// plays; later calls are ignored.
pub fn set_output_device(device: Option<String>) {
    if CONFIGURED_DEVICE.set(device).is_err() {
        tracing::warn!("the output device was already set; ignoring the later choice");
    }
}

/// Looks up a device by its `cpal::DeviceId` string, or `None` for "system default".
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
pub(crate) enum SinkDevice {
    /// The chosen device, or the system default where that is what was chosen.
    AsRequested,
    /// A device was named but could not be used, so the system default was opened instead.
    FellBack,
}

/// Opens an output sink on the configured device, falling back to the default.
pub(crate) fn open_sink() -> Result<(MixerDeviceSink, SinkDevice), rodio::DeviceSinkError> {
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
pub fn list_audio_devices() -> Result<()> {
    let host = cpal::default_host();

    let default_id = host
        .default_output_device()
        .and_then(|device| device.id().ok());

    let devices = host
        .output_devices()
        .context("could not list the audio output devices")?
        .filter_map(|device| {
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

/// How loud the test tone is.
const TEST_TONE_AMPLITUDE: f32 = 0.22;

/// Plays a short chime on `device` and returns once it has finished. Driven by
/// `shared::audio::TEST_AUDIO_FLAG`.
pub fn play_test_tone(device: Option<String>) -> Result<()> {
    set_output_device(device);

    let (mut stream, outcome) = open_sink().context("could not open an audio output device")?;

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

fn test_tone(channels: NonZero<u16>, rate: NonZero<u32>) -> SamplesBuffer {
    const NOTES: [(f32, f32); 2] = [(587.33, 0.0), (880.0, 0.13)];
    const LENGTH: Duration = Duration::from_millis(650);
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

        let sample = value * TEST_TONE_AMPLITUDE;
        samples.extend(std::iter::repeat_n(sample, channels.get() as usize));
    }

    SamplesBuffer::new(channels, rate, samples)
}
